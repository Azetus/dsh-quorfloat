#!/usr/bin/env node
/**
 * Assemble a self-contained, per-platform release bundle into `bundles/`.
 *
 * The output is one directory per platform — `bundles/dsh-quorfloat-<version>-<platform>/` —
 * which is what gets zipped and attached to a GitHub Release, and what is installed
 * by hand to find out whether the OS blocks an unsigned build. It contains the built
 * JS half, this repository's bundle patch with `quorfloatPath` pointed at a bundled
 * executable, the executable itself, the font and its licence, and a
 * `build-info.json` recording exactly what was built.
 *
 * Why the bundle's own patch can name its own binary: `src/host/binary.ts` resolves a
 * relative `quorfloatPath` against the package root, so `./bin/dsh-quorfloat` inside
 * the bundle resolves to the executable shipped next to it. Nothing in the resolver
 * changes for this, and the checkout's own patch keeps its empty value.
 *
 * One macOS bundle covers both chips, because the sidecar is built as a universal
 * executable: both apple targets are compiled and combined with `lipo`. Windows must
 * be assembled on Windows — `x86_64-pc-windows-msvc` needs the MSVC linker — so the
 * same script produces `bin/dsh-quorfloat.exe` and `os: ["win32"]` there. A Windows
 * bundle therefore cannot be produced from this checkout on macOS, and this script
 * says so rather than writing a layout it could not have built.
 *
 * No compression, no signing, no installer: the OS-blocking experiment needs the raw
 * bundle, and signing is a separate decision.
 *
 * Usage:
 *   node scripts/release.mjs [--target <rust-triple>]... [--out <dir>]
 *   node scripts/release.mjs --help
 */

import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import {
  cpSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs'
import { dirname, join, relative, sep } from 'node:path'
import { fileURLToPath } from 'node:url'

import { SIDECAR_NAME, stageSidecar } from './lib/assemble.mjs'
import { buildSidecar, hostTriple } from './lib/cargo.mjs'

const ROOT = dirname(dirname(fileURLToPath(import.meta.url)))

/** The manifest, read once: the version names the bundle, and the manifest ships in it. */
const MANIFEST = JSON.parse(readFileSync(join(ROOT, 'package.json'), 'utf8'))

/** Whether `npm`, `cargo` and `rustup` need a shell to be resolved (they do on Windows). */
const NEEDS_SHELL = process.platform === 'win32'

/**
 * The apple targets one macOS bundle covers.
 *
 * Both, always, by default: a bundle that names only the build machine's chip would
 * be wrong for half the Macs it is handed to, and the layout is per platform rather
 * than per architecture (`os: ["darwin"]`, one `bin/dsh-quorfloat`).
 */
const APPLE_TARGETS = ['aarch64-apple-darwin', 'x86_64-apple-darwin']

/**
 * Scratch space for the combined executable.
 *
 * Inside the checkout rather than the system temporary directory, for the same reason
 * `scripts/dev.mjs` stages into `.dev-home`: an agent-run shell may not be able to
 * write there, and the failure would surface as a `lipo` error rather than as
 * "there is nowhere to put this file".
 */
const SCRATCH = join(ROOT, '.tooling', 'release')

/** Default directory the bundles are written into, relative to the checkout. */
const DEFAULT_OUT = 'bundles'

/** Parse the options this script owns. */
function parseArgs(argv) {
  const options = { targets: [], out: DEFAULT_OUT, help: false }
  for (let index = 0; index < argv.length; index += 1) {
    switch (argv[index]) {
      case '--help':
      case '-h':
        options.help = true
        break
      case '--target':
        options.targets.push(argv[++index])
        break
      case '--out':
        options.out = argv[++index]
        break
      default:
        throw new Error(`unknown option ${JSON.stringify(argv[index])}; try --help`)
    }
  }
  if (options.targets.some(target => typeof target !== 'string' || target === '')) {
    throw new Error('--target needs a Rust triple, for example aarch64-apple-darwin')
  }
  if (typeof options.out !== 'string' || options.out === '') {
    throw new Error('--out needs a directory')
  }
  return options
}

/** Print usage. */
function usage() {
  process.stdout.write(`Assemble a release bundle for this platform into ${DEFAULT_OUT}/.

Usage: node scripts/release.mjs [options]

Options:
  --target <triple>  Rust target to build; repeat for each slice of one bundle.
                     Default: both apple targets on macOS (a universal binary),
                     otherwise the host target.
  --out <dir>        where the bundle directory is written (default: ${DEFAULT_OUT}, relative to the checkout)
  -h, --help         this text

The bundle is assembled into ${join(ROOT, DEFAULT_OUT)}/<name>-<version>-<platform>/. It
is not compressed and not signed. A Windows bundle must be assembled on Windows.
`)
}

/**
 * Run a command in the checkout, inheriting stdio so its output is visible.
 *
 * The shell is enabled on Windows only, because `npm`, `cargo` and `rustup` are `.cmd`
 * shims there and `spawnSync` does not resolve them without one. Everywhere else it
 * stays off, so a path containing a space or a shell metacharacter cannot be re-parsed.
 *
 * @returns true when the command reported success.
 */
function run(command, args) {
  const result = spawnSync(command, args, { cwd: ROOT, stdio: 'inherit', shell: NEEDS_SHELL })
  if (result.error !== undefined) {
    throw new Error(`cannot run ${command}: ${result.error.message}`)
  }
  return result.status === 0
}

/** Capture a command's stdout, or undefined when it cannot run at all. */
function capture(command, args, cwd = ROOT) {
  const result = spawnSync(command, args, { cwd, encoding: 'utf8', shell: NEEDS_SHELL })
  if (result.error !== undefined || result.status !== 0 || typeof result.stdout !== 'string') return undefined
  return result.stdout
}

/** Translate a Rust triple into the npm `os` value the bundle declares. */
function platformOf(triple) {
  if (triple.endsWith('-apple-darwin')) return 'darwin'
  if (triple.includes('-pc-windows-')) return 'win32'
  throw new Error(
    `unsupported target ${JSON.stringify(triple)}: this script assembles darwin and win32 bundles only`,
  )
}

/** The slices a run builds when `--target` says nothing. */
function defaultTargets(host) {
  return process.platform === 'darwin' ? [...APPLE_TARGETS] : [host]
}

/** Build the JS half, including the client bundle the loader consumes. */
function buildJavaScript() {
  process.stdout.write('release: npm run build:plugin (the host half and the client bundle)\n')
  if (!run('npm', ['run', 'build:plugin'])) throw new Error('the JS build failed')
  for (const entry of ['lib/index.js', 'lib/client.js']) {
    if (!existsSync(join(ROOT, entry))) throw new Error(`the JS build produced no ${entry}`)
  }
}

/**
 * Build the frontend, which cargo embeds into the sidecar at compile time.
 *
 * Run here rather than through `npm run build:sidecar`, because a release builds one
 * slice per target and every slice has to embed the same `dist/`; the shared cargo
 * half of both paths is `scripts/lib/cargo.mjs`. Doing it before the slices also means
 * a missing frontend fails before any cross target is touched.
 */
function buildFrontend() {
  process.stdout.write('release: npm run build:frontend (cargo embeds the result in the sidecar)\n')
  if (!run('npm', ['run', 'build:frontend'])) {
    throw new Error('the frontend build failed; run `npm run frontend:install` if its dependencies are missing')
  }
  const dist = join(ROOT, 'quorfloat', 'frontend', 'dist', 'index.html')
  if (!existsSync(dist)) throw new Error(`the frontend build produced no ${relative(ROOT, dist)}`)
}

/**
 * Combine the built slices into the executable the bundle ships.
 *
 * A single slice is used as it is; more than one becomes a universal executable via
 * `lipo`. The result is written to the scratch directory rather than straight into
 * the bundle, because staging it is {@link stageSidecar}'s job — the same code path
 * the development home uses.
 *
 * @param slices - the built executables, in target order.
 * @param name - a file name for the scratch artifact.
 * @returns the executable to stage.
 */
function combineSlices(slices, name) {
  if (slices.length === 1) return slices[0]
  mkdirSync(SCRATCH, { recursive: true })
  const output = join(SCRATCH, name)
  rmSync(output, { force: true })
  process.stdout.write(`release: lipo -create -output ${relative(ROOT, output)} ${slices.join(' ')}\n`)
  if (!run('lipo', ['-create', '-output', output, ...slices])) {
    throw new Error('lipo could not combine the slices into a universal executable')
  }
  return output
}

/**
 * Read `lipo -info` for the staged executable, as the evidence that every architecture
 * really is inside it.
 *
 * Read after staging and through a bundle-relative path, so the recorded text
 * describes the file a user unpacks rather than the directory this machine happened
 * to build it in.
 *
 * @param bundle - the assembled bundle.
 * @param universal - whether more than one slice was combined.
 * @returns the `lipo -info` output, or null for a single-slice bundle.
 */
function lipoInfo(bundle, universal) {
  if (!universal) return null
  const info = capture('lipo', ['-info', join('bin', SIDECAR_NAME)], bundle)
  if (info === undefined) throw new Error('lipo -info could not read the staged executable')
  return info.trim()
}

/**
 * Write the bundle's own patch: the checkout's patch with `quorfloatPath` pointed at
 * the bundled executable.
 *
 * A textual replacement rather than a YAML round trip: the file is hand-written prose
 * plus one row, and re-emitting it through a parser would drop every comment that
 * explains the row. The field is required to appear exactly once, so renaming it in
 * the checkout fails here rather than shipping a bundle that resolves a binary it
 * does not contain.
 *
 * @param bundle - the bundle directory.
 */
function writePatch(bundle) {
  const source = readFileSync(join(ROOT, 'cordis.patch.yml'), 'utf8')
  const lines = source.split('\n')
  const found = lines
    .map((line, index) => (/^\s*quorfloatPath:/u.test(line) ? index : -1))
    .filter(index => index >= 0)
  if (found.length !== 1) {
    throw new Error(`cordis.patch.yml must declare quorfloatPath exactly once (found ${found.length})`)
  }
  const line = lines[found[0]]
  const indent = /^(\s*)/u.exec(line)[1]
  lines[found[0]] = `${indent}quorfloatPath: ./bin/${SIDECAR_NAME}`
  writeFileSync(join(bundle, 'cordis.patch.yml'), lines.join('\n'))
}

/**
 * Write the bundle's manifest: the checkout's, pinned to this platform.
 *
 * Three deliberate changes. `os` is what makes an install refuse the bundle on the
 * wrong platform. `scripts` is dropped because the bundle has no sources and no
 * toolchain for them to run against. `files` is replaced with what the bundle
 * actually contains, so the directory describes itself rather than the checkout it
 * came from — the checkout's list names neither `bin/` nor `fonts/`.
 *
 * @param bundle - the bundle directory.
 * @param platform - the npm `os` value the bundle is built for.
 */
function writeManifest(bundle, platform) {
  const manifest = {
    ...MANIFEST,
    os: [platform],
    files: ['lib', 'bin', 'fonts', 'cordis.patch.yml', 'build-info.json', 'package.json'],
  }
  delete manifest.scripts
  writeFileSync(join(bundle, 'package.json'), `${JSON.stringify(manifest, null, 2)}\n`)
}

/** Every file below a directory, absolute, in a stable order. */
function walk(directory) {
  const files = []
  for (const entry of readdirSync(directory, { withFileTypes: true }).sort((a, b) => (a.name < b.name ? -1 : 1))) {
    const path = join(directory, entry.name)
    if (entry.isDirectory()) files.push(...walk(path))
    else files.push(path)
  }
  return files
}

/**
 * Hash every file in the bundle, keyed by its bundle-relative path.
 *
 * Written last, so it covers everything a user could install except itself. The
 * digests are what makes "is this the build I uploaded" answerable after the fact,
 * and the executable's is the one that matters: it is what an OS gate or a
 * notarization check reports on.
 */
function hashFiles(bundle) {
  const files = {}
  for (const path of walk(bundle)) {
    const bytes = readFileSync(path)
    files[relative(bundle, path).split(sep).join('/')] = {
      bytes: bytes.byteLength,
      sha256: createHash('sha256').update(bytes).digest('hex'),
    }
  }
  return files
}

/** Size of a directory tree, in bytes. */
function treeSize(directory) {
  return walk(directory).reduce((total, path) => total + statSync(path).size, 0)
}

/** Build the layout and write everything the bundle contains. */
function assemble({ bundle, executable, targets, platform }) {
  rmSync(bundle, { recursive: true, force: true })
  mkdirSync(bundle, { recursive: true })
  process.stdout.write(`release: assembling ${relative(ROOT, bundle)}/\n`)

  stageSidecar({
    root: ROOT,
    source: executable,
    binaryDirectory: join(bundle, 'bin'),
    // The bundle keeps `fonts/` beside `bin/` rather than inside it: that is the
    // layout the platform package ships, and `dev.mjs` exercises the same rule.
    assetDirectory: bundle,
    label: 'release',
    onMissing: 'throw',
  })
  cpSync(join(ROOT, 'lib'), join(bundle, 'lib'), { recursive: true, force: true })
  writePatch(bundle)
  writeManifest(bundle, platform)

  const universal = targets.length > 1
  const lipo = lipoInfo(bundle, universal)
  if (lipo !== null) process.stdout.write(`release: ${lipo}\n`)
  const info = {
    name: MANIFEST.name,
    version: MANIFEST.version,
    platform,
    os: [platform],
    targets,
    universal,
    lipo,
    files: hashFiles(bundle),
  }
  writeFileSync(join(bundle, 'build-info.json'), `${JSON.stringify(info, null, 2)}\n`)

  const count = Object.keys(info.files).length
  const megabytes = (treeSize(bundle) / 1_048_576).toFixed(1)
  process.stdout.write(`release: ${relative(ROOT, bundle)}/ (${count} files, ${megabytes} MB)\n`)
}

function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    usage()
    return
  }

  const host = hostTriple()
  const targets = options.targets.length > 0 ? [...new Set(options.targets)] : defaultTargets(host)
  const platforms = new Set(targets.map(platformOf))
  if (platforms.size !== 1) {
    throw new Error(`targets ${targets.join(', ')} belong to different platforms; assemble one bundle per platform`)
  }
  const platform = [...platforms][0]
  // The executable name is the host's (`SIDECAR_NAME`), and the layout is per
  // platform, so a cross-platform bundle could only be assembled wrongly.
  if (platform !== process.platform) {
    throw new Error(
      `a ${platform} bundle must be assembled on ${platform}; this is ${process.platform}, `
      + 'and the sidecar cannot be cross-compiled into the other layout from here',
    )
  }
  if (targets.length > 1 && platform !== 'darwin') {
    throw new Error('only the macOS bundle combines more than one target')
  }

  const out = join(ROOT, options.out)
  const bundle = join(out, `${MANIFEST.name}-${MANIFEST.version}-${platform}`)

  buildJavaScript()
  buildFrontend()
  const slices = targets.map(triple => buildSidecar({
    root: ROOT,
    target: triple,
    release: true,
    label: 'release',
  }))
  const executable = combineSlices(slices, SIDECAR_NAME)
  assemble({ bundle, targets, platform, executable })
}

try {
  main()
} catch (error) {
  process.stderr.write(`release: ${error instanceof Error ? error.message : String(error)}\n`)
  process.exit(1)
}

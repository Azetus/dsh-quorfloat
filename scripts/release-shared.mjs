/** Shared compilation, target selection and packing for both release layouts. */
import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import {
  cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync,
  readdirSync, renameSync, rmSync, statSync, writeFileSync,
} from 'node:fs'
import { basename, delimiter, dirname, join, relative, sep } from 'node:path'
import { fileURLToPath } from 'node:url'

import { SIDECAR_NAME, isBundleJunk } from './assemble.mjs'
import { buildSidecar, hostTriple } from './cargo.mjs'

export const ROOT = dirname(dirname(fileURLToPath(import.meta.url)))
export const MANIFEST = JSON.parse(readFileSync(join(ROOT, 'package.json'), 'utf8'))
export const APPLE_TARGETS = ['aarch64-apple-darwin', 'x86_64-apple-darwin']
const SCRATCH = join(ROOT, '.tooling', 'release')

/** npm's flat tarball stem, also used for staged directories on every platform. */
export function packageStem(name) {
  return name.replace(/^@/u, '').replace(/\//gu, '-')
}

/** Ship readable local documentation; npm's web README uses repository URLs. */
export function stageReadmeResources(directory, { npm = false } = {}) {
  for (const entry of ['README.md', 'README_EN.md', 'LICENSE', 'assets']) {
    cpSync(join(ROOT, entry), join(directory, entry), {
      recursive: true, force: true, filter: path => !isBundleJunk(path),
    })
  }
  if (!npm) return

  const repository = MANIFEST.repository.url.replace(/^git\+/u, '').replace(/\.git$/u, '')
  const ref = encodeURIComponent(process.env.GITHUB_SHA || 'HEAD')
  const raw = `https://raw.githubusercontent.com${new URL(repository).pathname}/${ref}`
  for (const entry of ['README.md', 'README_EN.md']) {
    const path = join(directory, entry)
    const text = readFileSync(path, 'utf8')
      .replace(/\]\((README(?:_EN)?\.md)\)/gu,
        (_match, file) => `](${repository}/blob/${ref}/${file})`)
      .replace(/(<img\b[^>]*\bsrc=")(assets\/[^"<>]+)(")/gu,
        (_match, prefix, file, suffix) => `${prefix}${raw}/${file}${suffix}`)
    writeFileSync(path, text)
  }
}

/** Parse the options this script owns. */
export function parseArgs(argv, defaultOut) {
  const options = { targets: [], out: defaultOut, help: false }
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

/**
 * Run a command in the checkout, inheriting stdio so its output is visible.
 *
 * This helper runs native executables (lipo); npm uses its JavaScript entry below.
 * Arguments and the working directory are passed directly without shell parsing.
 *
 * @returns true when the command reported success.
 */
function run(command, args) {
  const result = spawnSync(command, args, { cwd: ROOT, stdio: 'inherit', shell: false })
  if (result.error !== undefined) {
    throw new Error(`cannot run ${command}: ${result.error.message}`)
  }
  return result.status === 0
}

/** Capture a command's stdout, or undefined when it cannot run at all. */
function capture(command, args, cwd = ROOT) {
  const result = spawnSync(command, args, { cwd, encoding: 'utf8', shell: false })
  if (result.error !== undefined || result.status !== 0 || typeof result.stdout !== 'string') return undefined
  return result.stdout
}

/** Locate npm's JavaScript entry so Windows `.cmd` shims never reparse path arguments. */
function npmCli() {
  const nodeDirectory = dirname(process.execPath)
  const candidates = [
    process.env.npm_execpath,
    join(nodeDirectory, 'node_modules', 'npm', 'bin', 'npm-cli.js'),
    join(nodeDirectory, '..', 'lib', 'node_modules', 'npm', 'bin', 'npm-cli.js'),
  ]
  for (const directory of (process.env.PATH ?? '').split(delimiter).filter(Boolean)) {
    candidates.push(join(directory, process.platform === 'win32' ? 'npm.cmd' : 'npm'))
    candidates.push(join(directory, 'node_modules', 'npm', 'bin', 'npm-cli.js'))
  }
  for (const candidate of candidates) {
    if (typeof candidate !== 'string' || !existsSync(candidate)) continue
    const path = realpathSync(candidate)
    if (basename(path) === 'npm-cli.js') return path
  }
  throw new Error('cannot locate npm-cli.js; run this release through an npm build script')
}

/** Run npm without a shell, preserving spaces and shell metacharacters in each argument. */
export function runNpm(args, { cwd = ROOT, stdio = 'inherit' } = {}) {
  const result = spawnSync(process.execPath, [npmCli(), ...args], {
    cwd, stdio, encoding: 'utf8', shell: false,
  })
  if (result.error !== undefined) throw new Error(`cannot run npm: ${result.error.message}`)
  if (result.status !== 0) throw new Error(`npm ${args[0]} failed (exit ${result.status ?? result.signal})`)
  return result
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
  runNpm(['run', 'build:plugin'])
  for (const entry of ['lib/index.js', 'lib/client.js']) {
    if (!existsSync(join(ROOT, entry))) throw new Error(`the JS build produced no ${entry}`)
  }
}

/**
 * Build the frontend, which cargo embeds into the sidecar at compile time.
 *
 * Run here rather than through `npm run build:sidecar`, because a release builds one
 * slice per target and every slice has to embed the same `dist/`; the shared cargo
 * half of both paths is `scripts/cargo.mjs`. Doing it before the slices also means
 * a missing frontend fails before any cross target is touched.
 */
function buildFrontend() {
  process.stdout.write('release: npm run build:frontend (cargo embeds the result in the sidecar)\n')
  runNpm(['run', 'build:frontend'])
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
export function lipoInfo(bundle, universal) {
  if (!universal) return null
  const info = capture('lipo', ['-info', join('bin', SIDECAR_NAME)], bundle)
  if (info === undefined) throw new Error('lipo -info could not read the staged executable')
  return info.trim()
}

/** Every file below a directory, absolute, in a stable order. */
export function walk(directory) {
  const files = []
  for (const entry of readdirSync(directory, { withFileTypes: true }).sort((a, b) => (a.name < b.name ? -1 : 1))) {
    const path = join(directory, entry.name)
    if (isBundleJunk(path)) continue
    if (entry.isDirectory()) files.push(...walk(path))
    else files.push(path)
  }
  return files
}

/** npm manifests and hash keys always use forward slashes, including on Windows. */
export function bundlePath(bundle, path) {
  return relative(bundle, path).split(sep).join('/')
}

/**
 * Hash every file in the bundle, keyed by its bundle-relative path.
 *
 * Written last, so it covers everything a user could install except itself. The
 * digests are what makes "is this the build I uploaded" answerable after the fact,
 * and the executable's is the one that matters: it is what an OS gate or a
 * notarization check reports on.
 */
export function hashFiles(bundle) {
  const files = {}
  for (const path of walk(bundle)) {
    const bytes = readFileSync(path)
    files[bundlePath(bundle, path)] = {
      bytes: bytes.byteLength,
      sha256: createHash('sha256').update(bytes).digest('hex'),
    }
  }
  return files
}

/** Size of a directory tree, in bytes. */
export function treeSize(directory) {
  return walk(directory).reduce((total, path) => total + statSync(path).size, 0)
}

/** Pack only the freshly assembled directory; expose the final archive after npm succeeds. */
export function packBundle(bundle, archive) {
  const temporary = mkdtempSync(join(dirname(archive), '.pack-'))
  try {
    process.stdout.write(`release: packing ${relative(ROOT, bundle)}/\n`)
    const result = runNpm([
      'pack', '--json', '--ignore-scripts', '--pack-destination', temporary,
      '--cache', join(ROOT, '.tooling', '.npm-cache'),
    ], { cwd: bundle, stdio: ['ignore', 'pipe', 'inherit'] })
    const reported = JSON.parse(result.stdout)
    // npm 12 keys pack results by package name; earlier versions return an array.
    const packed = Array.isArray(reported)
      ? reported
      : reported !== null && typeof reported === 'object' ? Object.values(reported) : []
    if (packed.length !== 1) {
      throw new Error('npm pack did not report exactly one tarball filename')
    }
    const filename = packed[0]?.filename
    if (typeof filename !== 'string' || basename(filename) !== filename) {
      throw new Error('npm pack reported an invalid tarball filename')
    }
    // Same parent filesystem: the final path never names a partially written tarball.
    renameSync(join(temporary, filename), archive)
    process.stdout.write(`release: packed ${relative(ROOT, archive)}\n`)
  } finally {
    rmSync(temporary, { recursive: true, force: true })
  }
}

/** Select native release targets before creating output or building anything. */
export function selectTargets(requestedTargets) {
  const host = hostTriple()
  const targets = requestedTargets.length > 0 ? [...new Set(requestedTargets)] : defaultTargets(host)
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

  return { targets, platform }
}

/** Always compile fresh sources before combining native release slices. */
export function buildReleaseExecutable(targets) {
  buildJavaScript()
  buildFrontend()
  const slices = targets.map(triple => buildSidecar({
    root: ROOT, target: triple, release: true, label: 'release',
  }))
  return combineSlices(slices, SIDECAR_NAME)
}

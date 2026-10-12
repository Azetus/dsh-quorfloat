#!/usr/bin/env node
/**
 * Assemble a self-contained, per-platform release bundle into `bundles/`.
 *
 * The output is a directory and a matching npm tarball per platform:
 * `bundles/dsh-quorfloat-<version>-<platform>/` and the same name with `.tgz`.
 * The directory is assembled first and packed immediately, so the archive contains the
 * artifacts from this build. The directory contains the built
 * JS half, this repository's bundle patch with `quorfloatPath` pointed at a bundled
 * executable, the executable itself, and a
 * `build-info.json` recording exactly what was built.
 *
 * The version in that directory name is also the version the executable reports, and
 * `scripts/version.mjs` is what makes the two the same: it runs before the first
 * build, so a checkout whose `package.json` has already moved on from its
 * `quorfloat/Cargo.toml` fails here rather than after the cross-target compile.
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
 * npm creates the installable tarball; the unpacked bundle remains available.
 * Signing and installer creation are separate steps.
 *
 * Usage:
 *   node scripts/release.mjs [--target <rust-triple>]... [--out <dir>]
 *   node scripts/release.mjs --help
 */

import { cpSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { join, relative, resolve } from 'node:path'

import { SIDECAR_NAME, isBundleJunk, stageSidecar } from './assemble.mjs'
import { assertVersionsAgree } from './version.mjs'
import {
  ROOT, MANIFEST, parseArgs, selectTargets, buildReleaseExecutable,
  lipoInfo, walk, bundlePath, hashFiles, treeSize, packBundle,
} from './release-shared.mjs'

/** Default directory the bundles are written into, relative to the checkout. */
const DEFAULT_OUT = 'bundles'

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

The bundle is assembled into ${join(ROOT, DEFAULT_OUT)}/<name>-<version>-<platform>/
and packed into a matching .tgz beside it. Neither is signed.
A Windows bundle must be assembled on Windows.
`)
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
 * actually contains, as individual file paths rather than whole directories. Metadata
 * created after assembly therefore cannot enter the package through a directory rule.
 * `build-info.json` and `package.json` are listed before they are written.
 *
 * @param bundle - the bundle directory.
 * @param platform - the npm `os` value the bundle is built for.
 */
function writeManifest(bundle, platform) {
  const manifest = {
    ...MANIFEST,
    os: [platform],
    files: [...new Set([...walk(bundle).map(path => bundlePath(bundle, path)), 'build-info.json', 'package.json'])].sort(),
  }
  delete manifest.scripts
  // Offline installs carry their executable and must not fetch platform packages.
  delete manifest.optionalDependencies
  writeFileSync(join(bundle, 'package.json'), `${JSON.stringify(manifest, null, 2)}\n`)
}

/** Build the layout and write everything the bundle contains. */
function assemble({ bundle, executable, targets, platform }) {
  rmSync(bundle, { recursive: true, force: true })
  mkdirSync(bundle, { recursive: true })
  process.stdout.write(`release: assembling ${relative(ROOT, bundle)}/\n`)

  stageSidecar({
    source: executable,
    binaryDirectory: join(bundle, 'bin'),
    label: 'release',
  })
  cpSync(join(ROOT, 'lib'), join(bundle, 'lib'), { recursive: true, force: true, filter: source => !isBundleJunk(source) })
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
  // Before the arguments, because nothing this script can be asked to do is useful
  // on a checkout whose two halves would disagree about which version they are.
  const version = assertVersionsAgree(ROOT)
  const options = parseArgs(process.argv.slice(2), DEFAULT_OUT)
  if (options.help) {
    usage()
    return
  }
  process.stdout.write(`release: version ${version}\n`)
  const { targets, platform } = selectTargets(options.targets)

  const out = resolve(ROOT, options.out)
  // Scope separators cannot be part of a flat offline filename.
  const offlineName = MANIFEST.name.slice(MANIFEST.name.lastIndexOf('/') + 1)
  const bundleName = `${offlineName}-${MANIFEST.version}-${platform}`
  const bundle = join(out, bundleName)
  const archive = join(out, `${bundleName}.tgz`)
  mkdirSync(out, { recursive: true })
  // Delete only this platform's target and the legacy unqualified tarball of this version.
  // A failed build must not leave a stale same-version archive looking ready to install.
  rmSync(archive, { force: true })
  rmSync(join(out, `${offlineName}-${MANIFEST.version}.tgz`), { force: true })

  const executable = buildReleaseExecutable(targets)
  assemble({ bundle, targets, platform, executable })
  packBundle(bundle, archive)
}

try {
  main()
} catch (error) {
  process.stderr.write(`release: ${error instanceof Error ? error.message : String(error)}\n`)
  process.exit(1)
}

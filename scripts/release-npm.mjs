#!/usr/bin/env node
/** Assemble and pack the npm main package and this machine's native dependency. */
import { cpSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { join, relative, resolve } from 'node:path'

import { isBundleJunk, stageSidecar } from './assemble.mjs'
import { assertVersionsAgree } from './version.mjs'
import {
  ROOT, MANIFEST, APPLE_TARGETS, parseArgs, selectTargets, buildReleaseExecutable,
  lipoInfo, walk, bundlePath, hashFiles, packBundle, packageStem,
} from './release-shared.mjs'

const DEFAULT_OUT = 'bundles/npm'
// The host resolver reads the same metadata from its installed main manifest.
const DARWIN_PACKAGE = MANIFEST.quorfloat.platformPackages.darwin
const WINDOWS_PACKAGE = MANIFEST.quorfloat.platformPackages['win32-x64']

function usage() {
  process.stdout.write(`Build npm packages from fresh sources. No packages are published.

Usage: node scripts/release-npm.mjs [options]

Options:
  --target <triple>  Native Rust target; repeat for both macOS slices.
                     macOS requires arm64 + x64; Windows requires x64.
  --out <dir>        Output base (default: ${DEFAULT_OUT}, relative to the checkout).
                     A version subdirectory is always appended.
  -h, --help         This text.

The main package and current platform dependency are assembled and packed into
<out>/<version>/. Other platform outputs in that version directory are preserved.
Publish both native dependencies before publishing the main package once.
`)
}

/** Refuse to put an incomplete or different architecture under a published package name. */
function nativePackageFor(targets, platform) {
  if (platform === 'darwin') {
    if (targets.length !== APPLE_TARGETS.length || !APPLE_TARGETS.every(target => targets.includes(target))) {
      throw new Error('the npm darwin package requires both aarch64-apple-darwin and x86_64-apple-darwin')
    }
    return { name: DARWIN_PACKAGE, os: ['darwin'], cpu: ['arm64', 'x64'] }
  }
  if (targets.length !== 1 || !targets[0].startsWith('x86_64-pc-windows-')) {
    throw new Error('the npm win32-x64 package requires an x86_64-pc-windows target')
  }
  return { name: WINDOWS_PACKAGE, os: ['win32'], cpu: ['x64'] }
}

function copy(source, destination) {
  cpSync(source, destination, { recursive: true, force: true, filter: path => !isBundleJunk(path) })
}

/** Exact inventories keep metadata created after assembly out of npm archives. */
function writePackage(directory, manifest) {
  const files = [...new Set([
    ...walk(directory).map(path => bundlePath(directory, path)),
    'package.json', 'build-info.json',
  ])].sort()
  writeFileSync(join(directory, 'package.json'), `${JSON.stringify({ ...manifest, files }, null, 2)}\n`)
}

function writeInfo(directory, metadata) {
  writeFileSync(join(directory, 'build-info.json'), `${JSON.stringify({
    ...metadata, files: hashFiles(directory),
  }, null, 2)}\n`)
}

function assembleMain(directory) {
  copy(join(ROOT, 'lib'), join(directory, 'lib'))
  copy(join(ROOT, 'README.md'), join(directory, 'README.md'))
  copy(join(ROOT, 'LICENSE'), join(directory, 'LICENSE'))
  const source = readFileSync(join(ROOT, 'cordis.patch.yml'), 'utf8')
  const declarations = source.split('\n').filter(line => /^\s*quorfloatPath:/u.test(line))
  if (declarations.length !== 1 || !/^\s*quorfloatPath:\s*(?:""|'')\s*(?:#.*)?$/u.test(declarations[0])) {
    throw new Error('the npm bundle patch must declare exactly one empty quorfloatPath for dependency resolution')
  }
  writeFileSync(join(directory, 'cordis.patch.yml'), source)
  const manifest = {
    ...MANIFEST,
    optionalDependencies: {
      [DARWIN_PACKAGE]: MANIFEST.version,
      [WINDOWS_PACKAGE]: MANIFEST.version,
    },
    publishConfig: { ...MANIFEST.publishConfig, access: 'public', registry: 'https://registry.npmjs.org/' },
  }
  // Only generated packages can be published; the source checkout remains private.
  for (const key of ['private', 'scripts', 'devDependencies', 'os', 'cpu', 'libc', 'bundleDependencies', 'bundledDependencies', 'workspaces']) {
    delete manifest[key]
  }
  writePackage(directory, manifest)
  // Main-package contents and metadata are identical on both build platforms.
  writeInfo(directory, { name: manifest.name, version: manifest.version })
}

function assembleNative(directory, native, executable, targets) {
  stageSidecar({ source: executable, binaryDirectory: join(directory, 'bin'), label: 'release:npm' })
  copy(join(ROOT, 'LICENSE'), join(directory, 'LICENSE'))
  writeFileSync(join(directory, 'README.md'), `# ${native.name}\n\nNative sidecar for ${MANIFEST.name}. Install ${MANIFEST.name} to use the Harness plugin.\n`)
  const manifest = {
    name: native.name,
    version: MANIFEST.version,
    description: `Native sidecar for ${MANIFEST.name} (${native.os[0]}, ${native.cpu.join(', ')})`,
    license: MANIFEST.license,
    os: native.os,
    cpu: native.cpu,
    exports: { './package.json': './package.json' },
    publishConfig: { access: 'public', registry: 'https://registry.npmjs.org/' },
  }
  for (const key of ['author', 'contributors', 'repository', 'homepage', 'bugs', 'funding']) {
    if (MANIFEST[key] !== undefined) manifest[key] = MANIFEST[key]
  }
  writePackage(directory, manifest)
  const universal = targets.length > 1
  const lipo = lipoInfo(directory, universal)
  if (lipo !== null) process.stdout.write(`release:npm: ${lipo}\n`)
  writeInfo(directory, { name: native.name, version: MANIFEST.version, os: native.os, cpu: native.cpu, targets, universal, lipo })
}

function main() {
  const version = assertVersionsAgree(ROOT)
  const options = parseArgs(process.argv.slice(2), DEFAULT_OUT)
  if (options.help) {
    usage()
    return
  }
  const { targets, platform } = selectTargets(options.targets)
  const native = nativePackageFor(targets, platform)
  const out = join(resolve(ROOT, options.out), version)
  const mainDirectory = join(out, packageStem(MANIFEST.name))
  const nativeDirectory = join(out, packageStem(native.name))
  const mainArchive = join(out, `${packageStem(MANIFEST.name)}-${version}.tgz`)
  const nativeArchive = join(out, `${packageStem(native.name)}-${version}.tgz`)
  mkdirSync(out, { recursive: true })
  // Retire only the old main/current-platform staging outputs of this version.
  const legacyNames = ['dsh-quorfloat', `dsh-quorfloat-${platform === 'darwin' ? 'darwin' : 'win32-x64'}`]
  for (const legacy of legacyNames) {
    if (legacy === packageStem(MANIFEST.name) || legacy === packageStem(native.name)) continue
    rmSync(join(out, legacy), { recursive: true, force: true })
    rmSync(join(out, `${legacy}-${version}.tgz`), { force: true })
  }
  // Remove only this run's outputs; a failure cannot expose stale same-version archives.
  for (const archive of [mainArchive, nativeArchive]) rmSync(archive, { force: true })
  for (const directory of [mainDirectory, nativeDirectory]) {
    rmSync(directory, { recursive: true, force: true })
    mkdirSync(directory, { recursive: true })
  }
  process.stdout.write(`release:npm: version ${version}; output ${relative(ROOT, out)}/\n`)
  const executable = buildReleaseExecutable(targets)
  assembleMain(mainDirectory)
  assembleNative(nativeDirectory, native, executable, targets)
  packBundle(nativeDirectory, nativeArchive)
  packBundle(mainDirectory, mainArchive)
}

try {
  main()
} catch (error) {
  process.stderr.write(`release:npm: ${error instanceof Error ? error.message : String(error)}\n`)
  process.exit(1)
}

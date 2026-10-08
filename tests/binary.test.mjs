/**
 * Executable resolution tests, including the distribution convention.
 *
 * The plugin ships as one npm bundle plus a per-platform package selected by
 * `os`/`cpu` (the pattern Harness itself uses for native payloads). That
 * convention is verified here against a synthetic package layout — created in a
 * temp directory, never installed into the real profile — so a change to the
 * package naming or the `bin/` layout is caught before release rather than by a
 * user whose window never appears.
 */

import assert from 'node:assert/strict'
import { chmodSync, mkdirSync, realpathSync, writeFileSync } from 'node:fs'
import { test } from 'node:test'
import { join } from 'node:path'

import { cleanupDir, loadModule, scratchDir } from './helpers.mjs'

const { resolveQuorfloatBinary, PLATFORM_PACKAGE, EXECUTABLE_NAME } = await loadModule('host/binary.js')

/**
 * Build a fake package root containing an installed platform package.
 *
 * @param options.executable - whether the payload file exists and is executable.
 */
function fakePackage({ executable = true } = {}) {
  const root = scratchDir()
  writeFileSync(join(root, 'package.json'), JSON.stringify({ name: 'dsh-quorfloat', version: '0.0.1', type: 'module' }))
  const packageDir = join(root, 'node_modules', PLATFORM_PACKAGE)
  mkdirSync(join(packageDir, 'bin'), { recursive: true })
  writeFileSync(
    join(packageDir, 'package.json'),
    JSON.stringify({ name: PLATFORM_PACKAGE, version: '0.0.1', os: [process.platform], cpu: [process.arch] }),
  )
  if (executable) {
    const target = join(packageDir, 'bin', EXECUTABLE_NAME)
    writeFileSync(target, '#!/bin/sh\nexit 0\n')
    chmodSync(target, 0o755)
  }
  return { root, packageDir }
}

test('the platform package name follows the os/cpu convention', () => {
  assert.equal(PLATFORM_PACKAGE, `dsh-quorfloat-${process.platform}-${process.arch}`)
  assert.equal(EXECUTABLE_NAME, process.platform === 'win32' ? 'dsh-quorfloat.exe' : 'dsh-quorfloat')
})

test('an installed platform package is found through the package convention', () => {
  const { root, packageDir } = fakePackage()
  try {
    const resolved = resolveQuorfloatBinary({ configuredPath: '', envPath: undefined, packageRoot: root })
    // The resolver reports the path Node's module resolver produced, which macOS
    // canonicalizes through /private; compare canonical forms so the assertion is
    // about the layout rather than about the symlink.
    assert.equal(realpathSync(resolved.path), realpathSync(join(packageDir, 'bin', EXECUTABLE_NAME)))
    assert.equal(resolved.source, 'platform-package')
  } finally {
    cleanupDir(root)
  }
})

test('an explicit configuration path wins over the convention', () => {
  const { root } = fakePackage()
  const overrideDir = scratchDir()
  try {
    const override = join(overrideDir, 'my-quorfloat')
    writeFileSync(override, '#!/bin/sh\nexit 0\n')
    chmodSync(override, 0o755)
    const resolved = resolveQuorfloatBinary({ configuredPath: override, envPath: undefined, packageRoot: root })
    assert.equal(resolved.path, override)
    assert.equal(resolved.source, 'config')
  } finally {
    cleanupDir(root)
    cleanupDir(overrideDir)
  }
})

test('the environment variable is honoured when no explicit path is set', () => {
  const root = scratchDir()
  const envDir = scratchDir()
  try {
    writeFileSync(join(root, 'package.json'), JSON.stringify({ name: 'dsh-quorfloat', version: '0.0.1', type: 'module' }))
    const target = join(envDir, 'quorfloat-from-env')
    writeFileSync(target, '#!/bin/sh\nexit 0\n')
    chmodSync(target, 0o755)
    const resolved = resolveQuorfloatBinary({ configuredPath: '', envPath: target, packageRoot: root })
    assert.equal(resolved.path, target)
    assert.equal(resolved.source, 'env')
  } finally {
    cleanupDir(root)
    cleanupDir(envDir)
  }
})

test('a development cargo artifact is used when nothing is installed', () => {
  const root = scratchDir()
  try {
    writeFileSync(join(root, 'package.json'), JSON.stringify({ name: 'dsh-quorfloat', version: '0.0.1', type: 'module' }))
    const devPath = join(root, 'quorfloat', 'target', 'release')
    mkdirSync(devPath, { recursive: true })
    const target = join(devPath, EXECUTABLE_NAME)
    writeFileSync(target, '#!/bin/sh\nexit 0\n')
    chmodSync(target, 0o755)
    const resolved = resolveQuorfloatBinary({ configuredPath: '', envPath: undefined, packageRoot: root })
    assert.equal(resolved.path, target)
    assert.equal(resolved.source, 'dev-build')
  } finally {
    cleanupDir(root)
  }
})

test('a missing executable reports every candidate it tried', () => {
  const root = scratchDir()
  try {
    writeFileSync(join(root, 'package.json'), JSON.stringify({ name: 'dsh-quorfloat', version: '0.0.1', type: 'module' }))
    let thrown
    try {
      resolveQuorfloatBinary({ configuredPath: '/nonexistent/one', envPath: '/nonexistent/two', packageRoot: root })
    } catch (error) {
      thrown = error
    }
    assert.ok(thrown !== undefined, 'resolution must fail loudly rather than returning a bad path')
    assert.equal(thrown.code, 'unavailable')
    assert.match(thrown.message, /quorfloat executable not found/)
    assert.match(thrown.message, /DSH_QUORFLOAT_PATH/, 'the message says how to fix it')
    const sources = thrown.detail.attempts.map(attempt => attempt.source)
    assert.deepEqual(sources, ['config', 'env', 'platform-package', 'dev-build'])
    assert.ok(thrown.detail.attempts.every(attempt => attempt.ok === false))
  } finally {
    cleanupDir(root)
  }
})

test('a non-executable platform payload is a packaging error', () => {
  // A platform package payload must be a real executable; accepting a script
  // there would hide a broken package behind "it works on my machine".
  const root = scratchDir()
  try {
    writeFileSync(join(root, 'package.json'), JSON.stringify({ name: 'dsh-quorfloat', version: '0.0.1', type: 'module' }))
    const packageDir = join(root, 'node_modules', PLATFORM_PACKAGE)
    mkdirSync(join(packageDir, 'bin'), { recursive: true })
    writeFileSync(join(packageDir, 'package.json'), JSON.stringify({ name: PLATFORM_PACKAGE, version: '0.0.1' }))
    const payload = join(packageDir, 'bin', EXECUTABLE_NAME)
    writeFileSync(payload, '#!/bin/sh\nexit 0\n')
    chmodSync(payload, 0o644)
    let thrown
    try {
      resolveQuorfloatBinary({ configuredPath: '', envPath: undefined, packageRoot: root })
    } catch (error) {
      thrown = error
    }
    assert.ok(thrown !== undefined)
    const attempt = thrown.detail.attempts.find(entry => entry.source === 'platform-package')
    assert.equal(attempt.reason, 'not executable')
  } finally {
    cleanupDir(root)
  }
})

test('an explicit path may name a non-executable node script', () => {
  // The documented developer escape hatch: point the override at a launcher.
  // Scripts checked out of git often have no executable bit, and refusing them
  // produced "executable not found" for a file sitting right there.
  const root = scratchDir()
  try {
    writeFileSync(join(root, 'package.json'), JSON.stringify({ name: 'dsh-quorfloat', version: '0.0.1', type: 'module' }))
    const launcher = join(root, 'launcher.mjs')
    writeFileSync(launcher, '#!/usr/bin/env node\nprocess.exit(0)\n')
    chmodSync(launcher, 0o644)
    const resolved = resolveQuorfloatBinary({ configuredPath: launcher, envPath: undefined, packageRoot: root })
    assert.equal(resolved.source, 'config')
    assert.equal(resolved.path, launcher)
    assert.deepEqual(resolved.args, [launcher], 'the script is passed to the interpreter as an argument')
  } finally {
    cleanupDir(root)
  }
})

test('a non-executable file that is not a node script is still refused', () => {
  const root = scratchDir()
  try {
    writeFileSync(join(root, 'package.json'), JSON.stringify({ name: 'dsh-quorfloat', version: '0.0.1', type: 'module' }))
    const target = join(root, 'mystery-binary')
    writeFileSync(target, 'EI\x7fELF not really\n')
    chmodSync(target, 0o644)
    let thrown
    try {
      resolveQuorfloatBinary({ configuredPath: target, envPath: undefined, packageRoot: root })
    } catch (error) {
      thrown = error
    }
    assert.ok(thrown !== undefined)
    const attempt = thrown.detail.attempts.find(entry => entry.source === 'config')
    assert.equal(attempt.reason, 'not executable, and it does not declare a node interpreter')
  } finally {
    cleanupDir(root)
  }
})

test('the platform package may carry the macOS .app bundle instead of a bare executable', { skip: process.platform !== 'darwin' ? 'the .app layout is a macOS convention' : false }, () => {
  // The shipping form is a Tauri .app bundle; the resolver must find the real
  // executable inside it, or a packaged install would report "not found" for a
  // payload that is sitting right there.
  const root = scratchDir()
  try {
    writeFileSync(join(root, 'package.json'), JSON.stringify({ name: 'dsh-quorfloat', version: '0.0.1', type: 'module' }))
    const packageDir = join(root, 'node_modules', PLATFORM_PACKAGE)
    const macosDir = join(packageDir, 'bin', `${EXECUTABLE_NAME}.app`, 'Contents', 'MacOS')
    mkdirSync(macosDir, { recursive: true })
    writeFileSync(join(packageDir, 'package.json'), JSON.stringify({ name: PLATFORM_PACKAGE, version: '0.0.1', os: [process.platform], cpu: [process.arch] }))
    const payload = join(macosDir, EXECUTABLE_NAME)
    writeFileSync(payload, '#!/bin/sh\nexit 0\n')
    chmodSync(payload, 0o755)
    const resolved = resolveQuorfloatBinary({ configuredPath: '', envPath: undefined, packageRoot: root })
    // Compared through realpath for the same reason the bare-layout test does:
    // macOS canonicalises the scratch path through /private.
    assert.equal(realpathSync(resolved.path), realpathSync(payload))
    assert.equal(resolved.source, 'platform-package')
    assert.deepEqual(resolved.args, [], 'a bundled executable is launched directly')
  } finally {
    cleanupDir(root)
  }
})

test('a bare executable in the platform package wins over a bundled .app', { skip: process.platform !== 'darwin' ? 'the .app layout is a macOS convention' : false }, () => {
  // A developer who drops a bare executable next to the shipped bundle is
  // overriding the payload; the escape hatch must stay reachable.
  const root = scratchDir()
  try {
    writeFileSync(join(root, 'package.json'), JSON.stringify({ name: 'dsh-quorfloat', version: '0.0.1', type: 'module' }))
    const packageDir = join(root, 'node_modules', PLATFORM_PACKAGE)
    mkdirSync(join(packageDir, 'bin'), { recursive: true })
    writeFileSync(join(packageDir, 'package.json'), JSON.stringify({ name: PLATFORM_PACKAGE, version: '0.0.1', os: [process.platform], cpu: [process.arch] }))
    const bare = join(packageDir, 'bin', EXECUTABLE_NAME)
    writeFileSync(bare, '#!/bin/sh\nexit 0\n')
    chmodSync(bare, 0o755)
    const macosDir = join(packageDir, 'bin', `${EXECUTABLE_NAME}.app`, 'Contents', 'MacOS')
    mkdirSync(macosDir, { recursive: true })
    writeFileSync(join(macosDir, EXECUTABLE_NAME), '#!/bin/sh\nexit 0\n')
    chmodSync(join(macosDir, EXECUTABLE_NAME), 0o755)
    const resolved = resolveQuorfloatBinary({ configuredPath: '', envPath: undefined, packageRoot: root })
    assert.equal(realpathSync(resolved.path), realpathSync(bare))
  } finally {
    cleanupDir(root)
  }
})

#!/usr/bin/env node
/**
 * The one way this repository asks cargo for the sidecar executable.
 *
 * Two scripts build the Rust half: `scripts/build-sidecar.mjs` (the `build:sidecar`
 * unit, and through it the local full build `build:local`) and `scripts/release.mjs`
 * (once per release slice, which `lipo` then combines; that is the publishable
 * `build`). Both must issue the same command line and look
 * for the artifact in the same place, or a release cannot be reproduced from the
 * debug build that was tested. That rule lives here once.
 *
 * The host target is deliberately built **without** `--target`: cargo then reuses the
 * cache in `target/<profile>/`, which is the artifact `scripts/dev.mjs` runs. A cross
 * target gets `--target <triple>` and lands in `target/<triple>/<profile>/`, which is
 * where cargo puts it. The caller names a triple either way; whether it is "cross"
 * is decided here by comparing it against the host.
 *
 * The profile is explicit rather than inferred from the presence of an artifact: a
 * missing release build must never be silently satisfied by a stale debug one.
 *
 * `scripts/dev.mjs` keeps its own cargo invocation on purpose — it is the one caller
 * whose build is decided by a `--release` flag rather than by which script is running.
 */

import { spawnSync } from 'node:child_process'
import { existsSync } from 'node:fs'
import { join, relative } from 'node:path'

import { SIDECAR_NAME } from './assemble.mjs'

/** `rustc -vV` is asked at most once per process; every caller wants the same answer. */
let host

/**
 * Capture a command's stdout, or undefined when it cannot run at all.
 *
 * @param command - the executable to run.
 * @param args - its arguments.
 * @returns the captured stdout, or undefined.
 */
function capture(command, args) {
  // Rust toolchain commands are native executables on Windows too.
  const result = spawnSync(command, args, { encoding: 'utf8', shell: false })
  if (result.error !== undefined || result.status !== 0 || typeof result.stdout !== 'string') return undefined
  return result.stdout
}

/**
 * The Rust triple this machine builds for.
 *
 * Cached because both callers ask for it and it cannot change mid-run.
 *
 * @returns the host triple, for example `aarch64-apple-darwin`.
 * @throws {Error} when rustc cannot be asked for it.
 */
export function hostTriple() {
  if (host !== undefined) return host
  const output = capture('rustc', ['-vV'])
  const found = output === undefined ? undefined : /^host:\s*(\S+)$/mu.exec(output)?.[1]
  if (found === undefined) {
    throw new Error('cannot ask rustc for its host triple; is the Rust toolchain installed and on PATH?')
  }
  host = found
  return host
}

/**
 * Fail before cargo does when a cross target's standard library is not installed.
 *
 * Cargo's own message for this names a missing `core` crate and a path under the
 * toolchain, which reads as a broken crate rather than a missing component. A
 * toolchain that is not managed by rustup is not checked here: cargo will report it.
 *
 * @param triple - the Rust target being cross-compiled to.
 */
function requireRustTarget(triple) {
  const output = capture('rustup', ['target', 'list', '--installed'])
  if (output === undefined) return
  const installed = output.split('\n').map(line => line.trim()).filter(Boolean)
  if (!installed.includes(triple)) {
    throw new Error(`the Rust target ${triple} is not installed; install it with \`rustup target add ${triple}\``)
  }
}

/**
 * Build one sidecar executable and return its absolute path.
 *
 * The command is run with the checkout as its working directory and an inherited
 * stdio, so cargo's own progress and errors are what the caller's user sees.
 *
 * @param options.root - the checkout holding `quorfloat/Cargo.toml`.
 * @param options.target - the Rust triple to build; `undefined` means this machine.
 * @param options.release - build the release profile instead of debug.
 * @param options.label - the prefix of the progress line, `build:sidecar` or `release`.
 * @returns the absolute path of the built executable.
 * @throws {Error} when cargo cannot run, fails, or produces no artifact.
 */
export function buildSidecar({ root, target, release = false, label = 'build' }) {
  const hostTarget = hostTriple()
  const triple = target ?? hostTarget
  const profile = release ? 'release' : 'debug'
  const crossCompiling = triple !== hostTarget
  if (crossCompiling) requireRustTarget(triple)

  const args = ['build']
  if (process.env.DSH_QUORFLOAT_CARGO_LOCKED === '1') args.push('--locked')
  if (release) args.push('--release')
  args.push('--manifest-path', 'quorfloat/Cargo.toml')
  if (crossCompiling) args.push('--target', triple)

  process.stdout.write(`${label}: cargo ${args.join(' ')}\n`)
  const result = spawnSync('cargo', args, { cwd: root, stdio: 'inherit', shell: false })
  if (result.error !== undefined) {
    throw new Error(`cannot run cargo: ${result.error.message}`)
  }
  if (result.status !== 0) {
    throw new Error(`the Rust build failed for ${triple}`)
  }

  const artifact = join(
    root,
    'quorfloat', 'target',
    ...(crossCompiling ? [triple] : []),
    profile, SIDECAR_NAME,
  )
  if (!existsSync(artifact)) {
    throw new Error(`the Rust build for ${triple} produced no ${relative(root, artifact)}`)
  }
  return artifact
}

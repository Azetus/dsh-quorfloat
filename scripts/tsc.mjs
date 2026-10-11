#!/usr/bin/env node
/**
 * Run the pinned TypeScript compiler without a shell and without `.bin`.
 *
 * `npm run build:plugin` used to spell the compiler as
 * `./.tooling/node_modules/.bin/tsc`. That is a POSIX shell command, and it broke
 * on Windows exactly where it was most shell-specific: `./` is a POSIX
 * convention for "this is a path, not a PATH lookup", and `cmd.exe` reads the
 * leading `./` as a command whose name is `.` ("'.' is not recognized as an
 * internal or external command"). Swapping in `.\` would only move the breakage
 * to macOS, so the fix is to stop asking a shell to resolve the path at all.
 *
 * This script resolves the compiler from the toolchain's own manifest instead:
 * `.tooling/node_modules/typescript/package.json`'s `bin.tsc`, which the
 * installed package declares and which npm exposes as `.bin/tsc`. The entry is a
 * JavaScript file, so it runs under `process.execPath` — the Node already
 * running this script. Consequences, all of them deliberate:
 *
 * - no shell is involved, so there is nothing to quote and no `.cmd`/`.ps1` shim
 *   to choose between;
 * - no PATH lookup, so the compiler is the one `.tooling` pinned and not
 *   whatever a stray global `tsc` happens to be;
 * - a missing toolchain fails with the command that installs it, rather than
 *   with `'.' is not recognized`.
 *
 * Usage:
 *   node scripts/tsc.mjs -p tsconfig.json [--noEmit]
 *   node scripts/tsc.mjs --version
 */

import { spawnSync } from 'node:child_process'
import { existsSync, readFileSync } from 'node:fs'
import { dirname, isAbsolute, join, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const ROOT = dirname(dirname(fileURLToPath(import.meta.url)))

/** The directory `npm run toolchain` installs into. */
const TOOLCHAIN = join(ROOT, '.tooling')

/**
 * The absolute JavaScript entry point of the pinned compiler.
 *
 * Read from the installed package rather than hard-coded, so renaming the entry
 * upstream is followed instead of missed, and resolved relative to the package
 * directory so it cannot come from anywhere but `.tooling`.
 *
 * @returns the absolute path of `bin/tsc` of the installed TypeScript.
 * @throws {Error} when the toolchain is absent or does not declare `tsc`.
 */
function typescriptEntry() {
  const manifestPath = join(TOOLCHAIN, 'node_modules', 'typescript', 'package.json')
  if (!existsSync(manifestPath)) {
    throw new Error('the TypeScript toolchain is not installed in .tooling/; run `npm run toolchain` first')
  }
  let manifest
  try {
    manifest = JSON.parse(readFileSync(manifestPath, 'utf8'))
  } catch (error) {
    throw new Error(`cannot read ${relative(ROOT, manifestPath)}: ${error instanceof Error ? error.message : String(error)}`)
  }
  const bin = manifest.bin
  const declared = typeof bin === 'string' ? bin : bin?.tsc
  if (typeof declared !== 'string' || declared === '') {
    throw new Error(`typescript@${String(manifest.version)} declares no \`tsc\` entry in its package.json \`bin\``)
  }
  // npm's `bin` paths are package-relative; an absolute one is honoured as-is.
  const entry = isAbsolute(declared) ? declared : resolve(dirname(manifestPath), declared)
  if (!existsSync(entry)) {
    throw new Error(`typescript declares \`${declared}\`, but ${relative(ROOT, entry)} does not exist`)
  }
  return entry
}

function main() {
  const entry = typescriptEntry()
  const args = process.argv.slice(2)
  // `process.execPath`, not `node`: the interpreter running this script is the
  // one npm guaranteed, and a `.cmd` shim cannot enter the picture.
  const result = spawnSync(process.execPath, [entry, ...args], { cwd: ROOT, stdio: 'inherit' })
  if (result.error !== undefined) {
    throw new Error(`cannot run ${relative(ROOT, entry)}: ${result.error.message}`)
  }
  if (result.signal !== null) {
    process.stderr.write(`tsc: terminated by ${result.signal}\n`)
    process.exit(1)
  }
  process.exit(result.status ?? 1)
}

try {
  main()
} catch (error) {
  process.stderr.write(`tsc: ${error instanceof Error ? error.message : String(error)}\n`)
  process.exit(1)
}

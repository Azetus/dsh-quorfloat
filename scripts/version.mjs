#!/usr/bin/env node
/**
 * The one check that keeps the product version in step across the files that carry it.
 *
 * Two files declare it outright: the root `package.json` (whose `version` names the
 * release bundle `scripts/release.mjs` assembles) and `quorfloat/Cargo.toml` (whose
 * `[package] version` cargo compiles into the sidecar, which reports it in `hello`
 * and in `diag/snapshot`). They are separate files because separate tools read them,
 * and nothing but this check makes them agree: a bundle named for one version whose
 * executable reports another is indistinguishable from a stale download once it is
 * installed, and the user has no way to tell the two apart.
 *
 * `quorfloat/tauri.conf.json` normally has **no** `version` at all. It is optional in
 * the Tauri schema, `tauri-utils` documents that "[i]f removed the version number from
 * `Cargo.toml` is used", and `tauri-codegen` implements exactly that by falling back to
 * `CARGO_PKG_VERSION`; `tauri-build` likewise only overrides the Windows resource's
 * FileVersion and ProductVersion when the field is set. So omitting it removes a source
 * of the version rather than adding a default, and this check treats "absent" as
 * agreement — while still naming the file in a failure, so the reader sees all three.
 * If a value is present there it must match the other two like any other source.
 *
 * `PROTOCOL_VERSION` is deliberately not compared here. It is the host↔sidecar wire
 * contract, bumped when the frames change, and it is independent of the product
 * version on purpose — a protocol bump is not a release and a release is not a
 * protocol bump.
 *
 * `quorfloat/Cargo.lock` is not read either: cargo rewrites its own entry from
 * `Cargo.toml` on the next build, so a stale value there is repaired rather than
 * refused, and a release always builds after this check.
 */

import { readFileSync } from 'node:fs'
import { join } from 'node:path'

/**
 * Read the `version` of a Cargo manifest's `[package]` table.
 *
 * A line scan rather than a TOML parser: this is the only field the checkout needs
 * from a manifest, and the repository has no parser to reuse. The scan is
 * table-aware on purpose — `[dependencies] tauri = { version = "..." }` is a
 * `version` line too, and reading that one would compare the shell's version
 * instead of the product's.
 *
 * @param source - the manifest's text.
 * @returns the declared version, or undefined when the table has no `version`.
 */
function packageVersionOf(source) {
  let inPackageTable = false
  for (const line of source.split('\n')) {
    const table = /^\s*\[([^\]]+)\]\s*$/u.exec(line)
    if (table !== null) {
      inPackageTable = table[1] === 'package'
      continue
    }
    if (!inPackageTable) continue
    const version = /^\s*version\s*=\s*"([^"]+)"\s*$/u.exec(line)
    if (version !== null) return version[1]
  }
  return undefined
}

/**
 * Read what each file that can carry the product version currently says.
 *
 * `required` marks the sources a release cannot do without: a `package.json` with no
 * `version` names no bundle, and a Cargo manifest with no `version` cannot be built.
 * The Tauri config is not required, for the reason in this module's header.
 *
 * @param root - the checkout holding the three files.
 * @returns one `{ file, version, required }` entry per source, in a stable order.
 */
function readSources(root) {
  const manifest = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'))
  const tauri = JSON.parse(readFileSync(join(root, 'quorfloat', 'tauri.conf.json'), 'utf8'))
  const cargo = readFileSync(join(root, 'quorfloat', 'Cargo.toml'), 'utf8')
  return [
    {
      file: 'package.json',
      required: true,
      version: typeof manifest.version === 'string' ? manifest.version : undefined,
    },
    {
      file: 'quorfloat/Cargo.toml',
      required: true,
      version: packageVersionOf(cargo),
    },
    {
      file: 'quorfloat/tauri.conf.json',
      required: false,
      version: typeof tauri.version === 'string' ? tauri.version : undefined,
    },
  ]
}

/**
 * Refuse to build a release whose two halves would report different versions.
 *
 * Called before any build work — a mismatch found after the `lipo` step has already
 * cost the cross-target compile, and one found after upload cannot be fixed at all.
 *
 * @param root - the checkout to read, as an absolute path.
 * @returns the agreed version, for a caller that wants to print it.
 * @throws {Error} when a required source has no version, or any two declared values
 *   differ. The message names every source, its value, and what to do about it.
 */
export function assertVersionsAgree(root) {
  const sources = readSources(root)
  const declared = sources.filter(source => source.version !== undefined)
  const complete = sources.every(source => source.required === false || source.version !== undefined)
  const agree = declared.every(source => source.version === declared[0]?.version)
  if (complete && agree) return declared[0].version

  const listed = sources.map((source) => {
    if (source.version !== undefined) return `  ${source.file}: ${source.version}`
    if (source.required) return `  ${source.file}: (no version)`
    return `  ${source.file}: (none, Tauri falls back to quorfloat/Cargo.toml)`
  })
  throw new Error(
    `the product version disagrees across the checkout:\n${listed.join('\n')}\n`
    + 'Set every source that declares one to the version being released — for example '
    + '`0.2.0` in package.json and quorfloat/Cargo.toml — and leave quorfloat/tauri.conf.json '
    + 'without a `version` field, which is the state Tauri falls back from. Then build once so '
    + 'quorfloat/Cargo.lock records the same version.',
  )
}

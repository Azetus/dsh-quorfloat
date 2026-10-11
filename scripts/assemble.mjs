#!/usr/bin/env node
/**
 * The one staging rule for the sidecar executable and the assets that ship with it.
 *
 * `scripts/dev.mjs` stages a throwaway home that mirrors what a user installs, and
 * `scripts/release.mjs` stages the release bundle. The two must agree on the layout —
 * an executable in one directory with `fonts/<file>` and `fonts/OFL.txt` beside it —
 * or what was tested in development is not what a user installs. That rule lives
 * here once so the two cannot drift.
 *
 * The destination directory is deliberately a parameter rather than derived from the
 * checkout: in the development home the executable sits at the root of the staged
 * tree, while a bundle keeps it in `bin/` with `fonts/` one level up. Only the
 * *rule* is shared, not one hard-coded path.
 *
 * The font and the licence are copied together on purpose: a font without its
 * licence is the packaging mistake this layout exists to make impossible.
 */

import { chmodSync, cpSync, existsSync, mkdirSync, rmSync, statSync } from 'node:fs'
import { join } from 'node:path'

/** The executable name, matching what the host half resolves per platform. */
export const SIDECAR_NAME = process.platform === 'win32' ? 'dsh-quorfloat.exe' : 'dsh-quorfloat'

/** The font the panel loads. The egui-era lookup (`quorfloat/src/ui/fonts.rs`) is
 *  gone with the view layer; the web frontend (M4) consumes it from the bundled
 *  assets, and the staged `fonts/` directory stays so the shipped layout rule
 *  keeps being exercised. */
export const FONT_NAME = 'NotoSansSC-VF.otf'

/** The licence that must sit beside it. Same directory, so they travel together. */
export const FONT_LICENCE = 'OFL.txt'

/**
 * The asset groups staged beside the executable, as `<source>` under the
 * checkout's `quorfloat/assets/` and `<source>` under the destination.
 */
const ASSET_GROUPS = [{ source: 'fonts', names: [FONT_NAME, FONT_LICENCE] }]

/**
 * Asset directories the repository no longer has.
 *
 * They are removed rather than left behind: a stale copy in the destination is how
 * a panel keeps working on a machine whose checkout would not — the same reason
 * every file below is mirrored, not merely added.
 */
const STALE_ASSET_DIRECTORIES = ['icons']

/**
 * Copy the font and its licence into the staged layout.
 *
 * @param options.root - the checkout that holds `quorfloat/assets/`.
 * @param options.assetDirectory - the directory the assets are written into;
 *   `fonts/` is created below it.
 * @param options.label - the message prefix, `dev` or `release`.
 * @param options.onMissing - what a missing asset does. `warn` is the development
 *   behaviour (the panel falls back to Latin only); `throw` is the release
 *   behaviour, because a bundle that ships without its font is broken rather than
 *   degraded.
 * @returns the asset names that were missing.
 * @throws {Error} when an asset is missing and `onMissing` is `throw`.
 */
export function stageFonts({ root, assetDirectory, label = 'dev', onMissing = 'warn' }) {
  for (const stale of STALE_ASSET_DIRECTORIES) {
    rmSync(join(assetDirectory, stale), { recursive: true, force: true })
  }
  const missing = []
  for (const group of ASSET_GROUPS) {
    const target = join(assetDirectory, group.source)
    for (const name of group.names) {
      const source = join(root, 'quorfloat', 'assets', group.source, name)
      const destination = join(target, name)
      if (!existsSync(source)) {
        missing.push(name)
        // Mirror the repository rather than keeping a copy from an earlier run: a
        // staged asset the repository no longer has would let the panel look
        // healthy while a fresh checkout is broken — and it is the very failure
        // this staging exists to show you.
        rmSync(destination, { force: true })
        continue
      }
      mkdirSync(target, { recursive: true })
      cpSync(source, destination, { force: true })
    }
  }
  // Spelled with forward slashes on every platform: the path names a repository
  // location, and it is the string a developer copies into a command.
  const where = 'quorfloat/assets/fonts'
  const advice = 'run `npm run fonts` to fetch them'
  if (missing.length === 0) {
    process.stdout.write(`${label}: staged the font and its licence in ${assetDirectory}\n`)
    return { missing }
  }
  if (onMissing === 'throw') {
    throw new Error(`missing ${missing.join(' and ')} in ${where}/; ${advice}`)
  }
  process.stdout.write(
    `${label}: missing ${missing.join(' and ')} in ${where}/\n`
    + `${label}: the panel will fall back to Latin only — ${advice}\n`,
  )
  return { missing }
}

/**
 * Copy a built executable into the staged layout, with the assets it ships with.
 *
 * The previous executable is removed rather than overwritten in place: on Windows a
 * running sidecar holds the file open, and that failure has to read as "stop the
 * instance", not as an `EBUSY` from the middle of a copy.
 *
 * @param options.root - the checkout, for the assets.
 * @param options.source - the built executable to stage.
 * @param options.binaryDirectory - where the executable is written.
 * @param options.assetDirectory - where the assets are written; defaults to the
 *   binary directory, which is the development layout.
 * @param options.label - the message prefix, `dev` or `release`.
 * @param options.onMissing - see {@link stageFonts}.
 * @returns the staged executable path, and the asset result.
 * @throws {Error} when the existing executable cannot be replaced.
 */
export function stageSidecar({
  root,
  source,
  binaryDirectory,
  assetDirectory = binaryDirectory,
  label = 'dev',
  onMissing = 'warn',
}) {
  mkdirSync(binaryDirectory, { recursive: true })
  const destination = join(binaryDirectory, SIDECAR_NAME)
  if (existsSync(destination)) {
    try {
      rmSync(destination, { force: true })
    } catch {
      throw new Error(`cannot replace the staged sidecar; stop the existing ${label} instance and retry`)
    }
  }
  cpSync(source, destination, { force: true })
  makeExecutable(destination)
  const fonts = stageFonts({ root, assetDirectory, label, onMissing })
  return { path: destination, fonts }
}

/**
 * Set the executable bit on the staged file.
 *
 * `cpSync` already carries the source's mode, but the bit is set explicitly because a
 * bundle assembled from an archive or a downloaded artifact would otherwise ship an
 * executable nobody can run — and the host half reports that as "not executable"
 * rather than as a packaging fault.
 *
 * Windows is skipped outright: there is no executable bit to set, the host half's
 * `accessSync(path, X_OK)` is an existence check there, and so the mode is not part
 * of what makes the staged sidecar launchable. The call is also non-fatal on the
 * platforms that do have the bit: a filesystem that cannot express it (exFAT, a
 * network share, a read-only mount) must not turn "stage the sidecar" into a build
 * failure, and the mode copied by `cpSync` is still in place.
 *
 * @param path - the staged executable.
 */
function makeExecutable(path) {
  if (process.platform === 'win32') return
  try {
    chmodSync(path, statSync(path).mode | 0o111)
  } catch (error) {
    process.stderr.write(
      `cannot set the executable bit on ${path}: ${error instanceof Error ? error.message : String(error)}\n`,
    )
  }
}

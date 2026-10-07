/**
 * Fetch the bundled font the panel needs in order to draw more than Latin.
 *
 * Why a download instead of a committed file: the font is ~14 MB, and a binary that
 * size in git is paid for by every clone and every diff forever. Pinning the URL and
 * the SHA-256 costs one small script and makes the fetch verifiable — a changed or
 * truncated file fails loudly instead of rendering slightly wrong glyphs.
 *
 * **It is a build prerequisite**: the Rust crate compiles the font in with
 * `include_bytes!`, so `cargo build` stops without this file — and `build.rs` stops it
 * with this command named.
 *
 * It is deliberately **not** part of `npm run build`: a build step that reaches the
 * network is a build that fails on a plane, in CI without egress, or behind a proxy.
 * Running it is an explicit step (`npm run fonts`), and `npm run dev` tells you when the
 * file it needs is not there.
 *
 * It fetches **two** files: the CJK font, and the licence that must travel with it. Both
 * are pinned by SHA-256 and both land in `quorfloat/assets/fonts/`; the platform package
 * is built by copying that directory whole, so the font and its licence cannot be
 * separated by accident.
 *
 * Icons are deliberately not fetched: they arrive as compiled-in bytes from the
 * `egui-phosphor` crate, so there is no icon asset to pin, ship or lose.
 *
 * The font is Noto Sans SC (the noto-cjk *subset* variable font, OFL-1.1): it covers
 * Simplified and Traditional Chinese, Japanese, Greek and Cyrillic, and — being
 * variable — supplies every weight from one file. The copyright notice and the version
 * this pins are recorded in `assets/fonts/OFL.txt`. What it does *not* cover (Korean,
 * Arabic, Hebrew, Thai, Devanagari, emoji) is recorded in `docs/progress.md`, because
 * a panel that silently shows boxes for a language is worse than one that says so.
 *
 * Usage:
 *   node scripts/fetch-font.mjs            # download when missing
 *   node scripts/fetch-font.mjs --force    # re-download
 *   node scripts/fetch-font.mjs --check    # verify only, download nothing
 */

import { createHash } from 'node:crypto'
import { existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const ROOT = dirname(dirname(fileURLToPath(import.meta.url)))

/** Where the asset lives, relative to the repository root. */
export const FONT_DIR = join(ROOT, 'quorfloat/assets/fonts')

/** The one file this project bundles. */
export const FONT_FILE = 'NotoSansSC-VF.otf'

/**
 * The pinned source and its expected digest.
 *
 * Committed as data rather than fetched at run time: if upstream re-publishes the file
 * under the same tag, the digest stops matching and this fails, which is the point —
 * silently shipping different bytes under a version we tested is how a font change
 * becomes a rendering bug nobody can explain.
 */
export const FONT_SOURCE = {
  url: 'https://raw.githubusercontent.com/notofonts/noto-cjk/main/Sans/Variable/OTF/Subset/NotoSansSC-VF.otf',
  sha256: 'd13ed01ec8aa45d6178999b648e96fb92150683e9f8e2a581f2acf208dcbe44b',
  bytes: 15_054_748,
  license: 'SIL Open Font License 1.1 (see quorfloat/assets/fonts/OFL.txt)',
}

/** The licence file that must travel with the CJK font. */
export const LICENSE_FILE = 'OFL.txt'

/**
 * The licence, fetched from upstream rather than written by us.
 *
 * Deliberately unmodified: it is the file the font's own project publishes, and a
 * licence that a redistributor has edited is a licence nobody can compare against the
 * original. The notice it does *not* contain — the copyright line — lives in the font's
 * name table (`© 2014-2021 Adobe …, with Reserved Font Name 'Source'`), which is
 * readable only while the font is a file; another reason the font and this text ship
 * together rather than compiled into a binary.
 */
export const LICENSE_SOURCE = {
  url: 'https://raw.githubusercontent.com/notofonts/noto-cjk/main/Sans/LICENSE',
  sha256: '6a73f9541c2de74158c0e7cf6b0a58ef774f5a780bf191f2d7ec9cc53efe2bf2',
  bytes: 4_301,
}

/** The path the sidecar looks for when nothing overrides it. */
export function fontPath() {
  return join(FONT_DIR, FONT_FILE)
}

/** Where the licence text is placed, next to the font it covers. */
export function licensePath() {
  return join(FONT_DIR, LICENSE_FILE)
}

/** SHA-256 of a file, as lowercase hex. */
export function digestOf(path) {
  return createHash('sha256').update(readFileSync(path)).digest('hex')
}

/**
 * Report whether one pinned file is present and intact.
 *
 * @param path - where the file should be.
 * @param pin - the source record holding `sha256`.
 * @returns `{ ok: true, path }` or `{ ok: false, reason }`.
 */
function verifyOne(path, pin) {
  if (!existsSync(path)) return { ok: false, reason: `missing: ${path}` }
  const digest = digestOf(path)
  if (digest !== pin.sha256) return { ok: false, reason: `checksum mismatch: ${path} is ${digest}` }
  return { ok: true, path }
}

/**
 * Report whether both assets are present and intact.
 *
 * The licence is checked like the font, because it is part of what ships: a package
 * whose font arrived and whose licence did not is the failure this catches.
 *
 * @returns `{ ok: true, paths }` or `{ ok: false, reason }`.
 */
/** Every pinned asset, in the order a reader should think about them. */
export function assets() {
  return [
    { ...FONT_SOURCE, path: fontPath(), label: 'font' },
    { ...LICENSE_SOURCE, path: licensePath(), label: 'licence' },
  ]
}

export function verify() {
  const paths = []
  for (const asset of assets()) {
    const state = verifyOne(asset.path, asset)
    if (!state.ok) return state
    paths.push(state.path)
  }
  return { ok: true, path: paths[0], paths }
}

/**
 * Download one pinned file to a temporary neighbour, verify it, then move it into place.
 *
 * The temporary name matters: a half-written file under the real name would be
 * indistinguishable from a complete one on the next run, and the only thing standing
 * between this project and a half-downloaded CJK font is that rename.
 *
 * @param path - where the file belongs.
 * @param pin - the source record (url, sha256, and a label for messages).
 * @param label - what to call it in output.
 */
async function downloadOne(path, pin, label) {
  const partial = `${path}.partial`
  process.stdout.write(`fonts: downloading ${label} from ${pin.url}\n`)
  const response = await fetch(pin.url)
  if (!response.ok) {
    throw new Error(`could not download the ${label}: HTTP ${response.status} ${response.statusText}`)
  }
  const bytes = Buffer.from(await response.arrayBuffer())
  writeFileSync(partial, bytes)
  const digest = digestOf(partial)
  if (digest !== pin.sha256) {
    rmSync(partial, { force: true })
    throw new Error(
      `the downloaded ${label} does not match the pinned checksum\n`
      + `  expected ${pin.sha256}\n  got      ${digest}\n`
      + `  (nothing was installed; upstream may have re-published the file)`,
    )
  }
  renameSync(partial, path)
  process.stdout.write(`fonts: wrote ${path} (${bytes.length} bytes, checksum verified)\n`)
}

/** Download anything missing or corrupt. */
async function download() {
  mkdirSync(FONT_DIR, { recursive: true })
  for (const asset of assets()) {
    if (verifyOne(asset.path, asset).ok) continue
    await downloadOne(asset.path, asset, asset.label)
  }
}

/** Entry point. */
async function main() {
  const force = process.argv.includes('--force')
  const checkOnly = process.argv.includes('--check')
  const state = verify()

  if (state.ok && !force) {
    for (const path of state.paths) {
      process.stdout.write(`fonts: ${path} is present and matches the checksum\n`)
    }
    return
  }
  if (checkOnly) {
    process.stdout.write(`fonts: ${state.reason}\n`)
    process.stdout.write('fonts: run `npm run fonts` to download it\n')
    process.exitCode = 1
    return
  }
  try {
    await download()
  } catch (error) {
    process.stderr.write(`fonts: ${error instanceof Error ? error.message : String(error)}\n`)
    process.exitCode = 1
    return
  }
  const after = verify()
  if (!after.ok) {
    process.stderr.write(`fonts: ${after.reason}\n`)
    process.exitCode = 1
  }
}

// Only run when invoked directly: the tests import `verify`/`fontPath` from here.
if (process.argv[1] !== undefined && import.meta.url === `file://${process.argv[1]}`) {
  await main()
}

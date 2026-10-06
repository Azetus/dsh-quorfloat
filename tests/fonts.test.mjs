/**
 * Bundled-font asset tests.
 *
 * The font is not in git (14 MB), so these tests skip — with a reason, never silently
 * — when it has not been fetched. What they guard is the part that goes wrong
 * quietly: a pin that no longer matches the file, or a path the sidecar does not
 * actually look at.
 */

import assert from 'node:assert/strict'
import { existsSync, readFileSync, statSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { test } from 'node:test'

import { repoRoot } from './helpers.mjs'
import { digestOf } from '../scripts/fetch-font.mjs'
import {
  FONT_DIR,
  FONT_FILE,
  FONT_SOURCE,
  LICENSE_FILE,
  LICENSE_SOURCE,
  fontPath,
  licensePath,
  verify,
} from '../scripts/fetch-font.mjs'

const state = verify()
const skip = state.ok ? false : `${state.reason}; run \`npm run fonts\``

test('the pin names the file the sidecar looks for', () => {
  assert.equal(FONT_FILE, 'NotoSansSC-VF.otf')
  assert.match(fontPath(), new RegExp(`${FONT_FILE}$`))
  // The sidecar resolves `fonts/<file>` next to its executable, so the asset directory
  // has to be named that way too — otherwise the staged and shipped layouts diverge.
  assert.match(fontPath(), /assets\/fonts\//)
})

test('the pin is a checksummed download, not a floating URL', () => {
  assert.match(FONT_SOURCE.url, /^https:\/\/raw\.githubusercontent\.com\//)
  assert.match(FONT_SOURCE.sha256, /^[0-9a-f]{64}$/)
  assert.ok(FONT_SOURCE.bytes > 1_000_000, 'a CJK font is megabytes, not kilobytes')
  assert.match(FONT_SOURCE.license, /OFL/i)
})

test('the sidecar looks for the file the fetch script writes', () => {
  // The two halves are coupled by a constant in Rust and a constant in this script.
  // Renaming either side would not break the build — it would break the *panel*, at
  // runtime, on a machine whose package was built from the other half.
  const source = join(repoRoot, 'quorfloat/src/ui/fonts.rs')
  assert.ok(existsSync(source), `${source} is missing — did the font module move?`)
  const declared = /pub const FONT_FILE: &str = "([^"]+)"/.exec(readFileSync(source, 'utf8'))
  assert.ok(declared !== null, 'fonts.rs names the file it looks for')
  assert.equal(declared[1], FONT_FILE)
})

test('the licence is shipped beside the font it covers, unmodified', () => {
  // OFL 1.1 requires the notice to accompany every copy, and the copyright line lives in
  // the font's name table — unreadable to a user looking at a font file. So this text
  // travels in the same directory as the font, and it is upstream's file rather than our
  // annotated copy: a licence a redistributor has edited is one nobody can compare
  // against the original.
  assert.equal(LICENSE_FILE, 'OFL.txt')
  assert.equal(dirname(licensePath()), FONT_DIR, 'the licence sits in the fonts directory')
  const licence = readFileSync(licensePath(), 'utf8')
  assert.match(licence, /^This Font Software is licensed under the SIL Open Font License,/)
  assert.match(licence, /SIL OPEN FONT LICENSE Version 1\.1 - 26 February 2007/)
  assert.equal(digestOf(licensePath()), LICENSE_SOURCE.sha256, 'byte-for-byte upstream')
})

test('every fetched asset matches its pinned checksum and size', { skip }, () => {
  // `verify` recomputes every digest: a file that changed under the same pin fails here
  // rather than rendering slightly different glyphs in a shipped panel — or shipping a
  // licence that no longer matches the asset it claims to cover.
  const again = verify()
  assert.equal(again.ok, true, again.reason)
  const expected = [fontPath(), licensePath()]
  assert.deepEqual(again.paths, expected, 'every pinned asset is checked, none is forgotten')
  for (const [path, pin] of [[fontPath(), FONT_SOURCE], [licensePath(), LICENSE_SOURCE]]) {
    assert.equal(statSync(path).size, pin.bytes, `the pinned size is the real size: ${path}`)
  }
})

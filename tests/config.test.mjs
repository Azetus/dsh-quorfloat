/**
 * Configuration schema tests.
 *
 * The schema is the plugin's user-facing contract: it is what a patch layer is
 * validated against. These tests pin the two properties that matter most — a
 * fully populated result even for an empty patch, and a loud failure for a
 * misspelled field instead of a silent fallback to a default.
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'

import { loadModule } from './helpers.mjs'

const { Config, DEFAULT_CONFIG } = await loadModule('config.js')

/** Validate a raw config and return either the value or the issues. */
function validate(raw) {
  return Config['~standard'].validate(raw)
}

test('an absent config normalizes to documented defaults', () => {
  const result = validate(undefined)
  assert.ok(!('issues' in result), 'defaults must validate')
  assert.equal(result.value.enabled, true)
  assert.equal(result.value.hotkey, 'Alt+Space')
  assert.equal(result.value.quorfloatPath, '')
  assert.equal(result.value.defaultWorkspaceId, '')
  assert.equal(result.value.window.width, 640)
  assert.equal(result.value.window.maxHeight, 560)
  assert.equal(result.value.window.anchor, 'center', 'the centre of the primary display')
  assert.equal(result.value.window.theme, 'system')
  assert.equal(result.value.heartbeatMs, 5000)
  assert.equal(result.value.heartbeatMissLimit, 3)
  assert.equal(result.value.restartLimit, 3)
  assert.equal(result.value.logLevel, 'info')
})

test('a partial patch fills defaults for the fields it omits', () => {
  const result = validate({ hotkey: '', window: { width: 800 } })
  assert.ok(!('issues' in result))
  assert.equal(result.value.hotkey, '', 'an empty hotkey means "register none"')
  assert.equal(result.value.window.width, 800)
  assert.equal(result.value.window.maxHeight, 560, 'sibling defaults still apply')
  assert.equal(result.value.window.anchor, 'center')
})

test('the panel language is unset by default and accepts only the two ids', () => {
  // Empty is not a third language: it is the state the first launch resolves from the
  // Harness's own locale and then persists (`src/host/language.ts`).
  assert.equal(validate(undefined).value.window.language, '')
  assert.equal(validate({ window: { language: '' } }).value.window.language, '')
  assert.equal(validate({ window: { language: 'zh' } }).value.window.language, 'zh')
  assert.equal(validate({ window: { language: 'en' } }).value.window.language, 'en')

  const rejected = validate({ window: { language: 'fr' } })
  assert.ok('issues' in rejected, 'a language this build does not ship is refused, not remapped')
  assert.deepEqual(rejected.issues[0].path, ['window', 'language'])
})

test('an unknown top-level field is rejected by name', () => {
  const result = validate({ hotkey: 'Alt+Space', hotket: 'Alt+Space' })
  assert.ok('issues' in result)
  assert.equal(result.issues.length, 1)
  assert.match(result.issues[0].message, /unknown configuration field/)
  assert.deepEqual(result.issues[0].path, ['hotket'])
})

test('an unknown nested field is rejected with its path', () => {
  const result = validate({ window: { with: 800 } })
  assert.ok('issues' in result)
  assert.deepEqual(result.issues[0].path, ['window', 'with'])
})

test('type and range violations are reported per field', () => {
  const result = validate({
    heartbeatMs: -1,
    window: { width: 'wide', theme: 'neon' },
    startupTimeoutMs: 1.5,
    logLevel: 'verbose',
  })
  assert.ok('issues' in result)
  const paths = result.issues.map(issue => issue.path.join('.'))
  assert.deepEqual(paths.sort(), ['heartbeatMs', 'logLevel', 'startupTimeoutMs', 'window.theme', 'window.width'])
})

test('a non-mapping config is rejected once, not field by field', () => {
  const result = validate(['not', 'a', 'mapping'])
  assert.ok('issues' in result)
  assert.equal(result.issues.length, 1)
  assert.match(result.issues[0].message, /must be a mapping/)
})

test('the exported defaults equal a validation of the empty patch', () => {
  const result = validate({})
  assert.ok(!('issues' in result))
  assert.deepEqual(result.value, DEFAULT_CONFIG)
})

/**
 * The panel's language: where the first value comes from, and how it reaches the sidecar.
 *
 * Three decisions are load-bearing here and each has a failure mode that is invisible from
 * the host side:
 *
 * - the **seed mapping**, because a wrong answer is persisted and then outlives every
 *   later launch;
 * - the **reading of the Harness's locale preference**, because it is another plugin's
 *   service and may be absent, slow, or shaped differently;
 * - the **carry to the sidecar**, because the value has to be in the spawn environment
 *   before the first frame *and* in the `ready` payload.
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'

import { buildSupervisor, cleanupDir, loadModule, readReport, scratchDir, waitFor } from './helpers.mjs'

const {
  FALLBACK_LANGUAGE,
  HARNESS_LOCALE_NAMESPACE,
  HARNESS_LOCALE_PREFERENCE_FIELD,
  SUPPORTED_LANGUAGES,
  isLanguage,
  readHarnessLocalePreference,
  resolvePanelLanguage,
} = await loadModule('host/language.js')

const { DEFAULT_CONFIG } = await loadModule('config.js')

/** A `settings` service whose descriptor list is exactly what the test wants. */
function settingsService(rows) {
  return { describe: () => rows }
}

/** A `settings` descriptor for the Harness's locale namespace. */
function localeRow(value, user) {
  return { ns: HARNESS_LOCALE_NAMESPACE, value, user }
}

/** A `configEditor` service answering the given configuration rows. */
function configEditorService(rows) {
  return { configuration: () => rows }
}

test('the two ids are the Harness\'s own, and nothing else is a language', () => {
  assert.deepEqual([...SUPPORTED_LANGUAGES].sort(), ['en', 'zh'])
  assert.equal(FALLBACK_LANGUAGE, 'en')
  assert.equal(isLanguage('zh'), true)
  assert.equal(isLanguage('en'), true)
  assert.equal(isLanguage(''), false, 'the schema\'s unset is not a language')
  assert.equal(isLanguage('ZH'), false, 'a second spelling would never match the frontend\'s type')
  assert.equal(isLanguage(undefined), false)
})

test('an explicit panel setting is used as it stands', () => {
  // The setting outranks the seed: once the settings page has written a value, the
  // Harness's language must not be able to move the panel again.
  assert.equal(resolvePanelLanguage('zh', 'en'), 'zh')
  assert.equal(resolvePanelLanguage('en', 'zh'), 'en')
  assert.equal(resolvePanelLanguage('zh', undefined), 'zh')
  assert.equal(resolvePanelLanguage('en', undefined), 'en')
})

test('with no setting, the Harness\'s own language seeds the panel', () => {
  assert.equal(resolvePanelLanguage('', 'zh'), 'zh')
  assert.equal(resolvePanelLanguage('', 'en'), 'en')
  assert.equal(resolvePanelLanguage(undefined, 'zh'), 'zh', 'an absent field is the same unset state')
})

test('a Harness language we do not support becomes English, not Chinese', () => {
  // "Unsupported" is not "absent": the Harness is on a language this build does not ship,
  // and English is the one both audiences can read.
  assert.equal(resolvePanelLanguage('', 'fr'), 'en')
  assert.equal(resolvePanelLanguage('', 'zh-Hant'), 'en', 'a variant is not one of the two ids')
  assert.equal(resolvePanelLanguage('', ''), 'en')
})

test('an unreachable Harness becomes English rather than blocking the panel', () => {
  assert.equal(resolvePanelLanguage('', undefined), 'en')
  assert.equal(resolvePanelLanguage('', null), 'en')
})

test('a setting this build does not know still lets the Harness seed it', () => {
  // Only a hand-built config can carry this (the schema rejects it), and the honest
  // reading of a value that is not a language is "no setting here".
  assert.equal(resolvePanelLanguage('de', 'zh'), 'zh')
  assert.equal(resolvePanelLanguage('de', 'fr'), 'en')
  assert.equal(resolvePanelLanguage('de', undefined), 'en')
})

test('the Harness preference is read from its settings namespace and field', () => {
  const get = name =>
    name === 'settings'
      ? settingsService([{ ns: 'some-other-plugin' }, localeRow({ [HARNESS_LOCALE_PREFERENCE_FIELD]: 'zh' })])
      : undefined
  assert.equal(readHarnessLocalePreference(get), 'zh')
  // And the raw value is passed through unmapped: deciding what an unsupported id means is
  // the resolver's job, not the reader's.
  assert.equal(readHarnessLocalePreference(() => settingsService([localeRow({ preference: 'fr' })])), 'fr')
})

test('the effective projection is preferred, and the profile override is the fallback', () => {
  const both = () => settingsService([localeRow({ preference: 'zh' }, { preference: 'en' })])
  assert.equal(readHarnessLocalePreference(both), 'zh', 'the value the browser half follows')
  const onlyOverride = () => settingsService([localeRow(undefined, { preference: 'en' })])
  assert.equal(readHarnessLocalePreference(onlyOverride), 'en')
})

test('an unreachable Harness is no preference, never an exception', () => {
  // Every one of these is a real composition: no service, a service without the method,
  // a method that throws, and a shape this build does not recognise.
  assert.equal(readHarnessLocalePreference(() => undefined), undefined)
  assert.equal(readHarnessLocalePreference(() => ({})), undefined)
  assert.equal(readHarnessLocalePreference(() => ({ describe: () => { throw new Error('boom') } })), undefined)
  assert.equal(readHarnessLocalePreference(() => ({ describe: () => 'not a list' })), undefined)
  assert.equal(readHarnessLocalePreference(() => settingsService([localeRow({ preference: 7 })])), undefined)
  assert.equal(readHarnessLocalePreference(() => settingsService([localeRow({ preference: '  ' })])), undefined)
})

test('the persisted user-settings document is read when the settings service is absent', () => {
  // The `configEditor` fallback is what answers while the locale plugin's fiber is not
  // active yet — the case a real profile hits on a cold start.
  const get = name =>
    name === 'configEditor'
      ? configEditorService([
          { entry: { options: { id: 'some-other-plugin' } }, override: {}, inherited: {} },
          {
            entry: { options: { id: HARNESS_LOCALE_NAMESPACE } },
            override: { [HARNESS_LOCALE_PREFERENCE_FIELD]: 'en' },
            inherited: { [HARNESS_LOCALE_PREFERENCE_FIELD]: 'zh' },
          },
        ])
      : undefined
  assert.equal(readHarnessLocalePreference(get), 'en')
  // An absent `settings` service whose `configEditor` throws is still no preference.
  assert.equal(
    readHarnessLocalePreference(() => ({ configuration: () => { throw new Error('boom') } })),
    undefined,
  )
})

test('the seed, composed end to end, is the mapping the design asks for', () => {
  const seed = get => resolvePanelLanguage('', readHarnessLocalePreference(get))
  assert.equal(seed(() => settingsService([localeRow({ preference: 'zh' })])), 'zh')
  assert.equal(seed(() => settingsService([localeRow({ preference: 'ja' })])), 'en')
  assert.equal(seed(() => undefined), 'en')
})

test('the resolved language reaches the sidecar before the first frame and in the ready payload', async () => {
  // The environment is what the sidecar persists on the first launch; the `ready` payload is
  // what a running sidecar reads. A setting missing from either is a panel that draws in one
  // language and labels its tray in another.
  const dir = scratchDir()
  const reportPath = `${dir}/report.json`
  const { supervisor, config, restore } = await buildSupervisor({
    mode: 'normal',
    config: { window: { ...DEFAULT_CONFIG.window, language: 'en' }, heartbeatMs: 0 },
    reportPath,
  })
  try {
    await supervisor.start()
    // The report is written as the peer exits, so the test waits for the handshake (which is
    // what the `ready` payload follows) and reads everything after the stop below.
    await waitFor('the sidecar to handshake', async () => {
      const snapshot = supervisor.snapshot()
      return snapshot.state === 'running' && snapshot.handshaken === true ? snapshot : undefined
    })
  } finally {
    // The peer writes its observation report as it exits, so it is read after the stop.
    await supervisor.stop()
    const report = await waitFor('the peer report', async () => await readReport(reportPath))
    assert.equal(report.windowLanguageEnv, 'en', 'the spawn environment carries it before the first frame')
    assert.equal(report.readyWindow?.language, 'en', 'and the ready payload carries it too')
    assert.equal(config.window.language, 'en')
    restore()
    cleanupDir(dir)
  }
})

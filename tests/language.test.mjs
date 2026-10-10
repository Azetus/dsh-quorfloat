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
import { readFile } from 'node:fs/promises'
import { join } from 'node:path'
import { test } from 'node:test'

import { buildSupervisor, cleanupDir, loadModule, readReport, repoRoot, scratchDir, waitFor } from './helpers.mjs'

const {
  FALLBACK_LANGUAGE,
  HARNESS_LOCALE_NAMESPACE,
  HARNESS_LOCALE_PREFERENCE_FIELD,
  SUPPORTED_LANGUAGES,
  isLanguage,
  readHarnessLocalePreference,
  resolvePanelLanguage,
  resolvePanelLanguageDecision,
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

test('a decision is told apart from the fallback the panel\'s webview may outrank', () => {
  // The point of the flag: `en` is both "the user chose English" and "nobody chose
  // anything", and only the first may be written down as the panel's own setting. The
  // second leaves the question to the webview (`navigator.languages`), which is the
  // Harness's own rule for a state with no stored preference.
  assert.deepEqual(resolvePanelLanguageDecision('zh', undefined), { language: 'zh', decided: true })
  assert.deepEqual(resolvePanelLanguageDecision('en', undefined), { language: 'en', decided: true })
  assert.deepEqual(resolvePanelLanguageDecision('', 'zh'), { language: 'zh', decided: true })
  assert.deepEqual(resolvePanelLanguageDecision('', 'en'), { language: 'en', decided: true })

  // A stored id this build does not ship is not a decision, and neither is an absent or
  // unreadable Harness: both resolve to `en` *and* leave the webview the answer.
  assert.deepEqual(resolvePanelLanguageDecision('', 'zh-Hant'), { language: 'en', decided: false })
  assert.deepEqual(resolvePanelLanguageDecision('', 'fr'), { language: 'en', decided: false })
  assert.deepEqual(resolvePanelLanguageDecision('', undefined), { language: 'en', decided: false })
  assert.deepEqual(resolvePanelLanguageDecision('', null), { language: 'en', decided: false })

  // Only a hand-built config can carry `de` (the schema rejects it), and a value that is
  // not a language is "no setting here": the Harness still decides when it can.
  assert.deepEqual(resolvePanelLanguageDecision('de', 'zh'), { language: 'zh', decided: true })
  assert.deepEqual(resolvePanelLanguageDecision('de', 'fr'), { language: 'en', decided: false })

  // The plain resolver is exactly the language half of the decision, so the two can never
  // disagree about what a launch starts with.
  for (const [configured, harness] of [['zh', undefined], ['', 'zh-Hant'], ['de', 'en'], ['', undefined]]) {
    assert.equal(resolvePanelLanguage(configured, harness), resolvePanelLanguageDecision(configured, harness).language)
  }
})

/**
 * Spawn the mock peer once and read back what it observed about the language.
 *
 * The peer is the only place both routes meet — the spawn environment (what the first
 * launch persists) and the `ready` payload (what a running process reads) — so the test
 * asks it rather than the host's own configuration object.
 *
 * @param windowConfig - the effective `window` section to spawn with.
 * @returns the peer's report and the configuration that produced it.
 */
async function observedLanguage(windowConfig) {
  const dir = scratchDir()
  const reportPath = `${dir}/report.json`
  const { supervisor, config, restore } = await buildSupervisor({
    mode: 'normal',
    config: { window: windowConfig, heartbeatMs: 0 },
    reportPath,
  })
  let report
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
    report = await waitFor('the peer report', async () => await readReport(reportPath))
    restore()
    cleanupDir(dir)
  }
  return { report, config }
}

test('the resolved language reaches the sidecar before the first frame and in the ready payload', async () => {
  // The environment is what the sidecar persists on the first launch; the `ready` payload is
  // what a running sidecar reads. A setting missing from either is a panel that draws in one
  // language and labels its tray in another.
  const { report, config } = await observedLanguage({
    ...DEFAULT_CONFIG.window,
    language: 'en',
    languageDecided: true,
  })
  assert.equal(report.windowLanguageEnv, 'en', 'the spawn environment carries it before the first frame')
  assert.equal(report.windowLanguageDecidedEnv, '1', 'and it says this one is a decision')
  assert.equal(report.readyWindow?.language, 'en', 'and the ready payload carries it too')
  assert.equal(report.readyWindow?.languageDecided, true, 'with the same meaning of `en`')
  assert.equal(config.window.language, 'en')
})

test('a language nobody chose is marked as a fallback on the wire', async () => {
  // Without this marker the sidecar cannot tell the host's `en` from a real choice, and it
  // would write the fallback down as the panel's own setting before the panel's webview
  // ever had the chance to report `navigator.languages`.
  const { report, config } = await observedLanguage({
    ...DEFAULT_CONFIG.window,
    language: 'en',
    languageDecided: false,
  })
  assert.equal(report.windowLanguageEnv, 'en', 'the fallback is still what the first frame draws with')
  assert.equal(report.windowLanguageDecidedEnv, '0', 'but the sidecar is told it is not a decision')
  assert.equal(report.readyWindow?.languageDecided, false)
  assert.equal(config.window.languageDecided, false)
})

test('every command the page invokes is one the shell registers', async () => {
  // Two halves of one wire with nothing else checking them: TypeScript compiles the
  // frontend against a string, and the Rust handler list is just a macro invocation, so a
  // rename on either side is a panel whose answer silently never arrives. The webview's
  // language report is the newest and the least visible of these — when it is lost, the
  // panel simply keeps the `en` it started with.
  const api = await readFile(join(repoRoot, 'quorfloat/frontend/src/api.ts'), 'utf8')
  const rust = await readFile(join(repoRoot, 'quorfloat/src/main.rs'), 'utf8')
  const invoked = [...api.matchAll(/invoke\('([a-z_]+)'/g)].map(match => match[1])
  const registered = [...rust.matchAll(/generate_handler!\[([\s\S]*?)\]/g)]
    .flatMap(match => match[1].split(','))
    .map(entry => entry.trim())
    .filter(entry => entry !== '')

  assert.ok(invoked.includes('report_languages'), 'the language report has a wrapper to go through')
  const missing = invoked.filter(name => !registered.includes(name))
  assert.deepEqual(missing, [], 'every invoked command is registered by the shell')
})

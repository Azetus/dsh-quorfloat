import { describe, expect, test } from 'vitest'
import { reportedLanguages, type LanguageSource } from '../src/lib/languages'

// The panel's own language is seeded from this list while nobody has chosen one, so the
// reading has to be exact where it matters (order, tags verbatim) and total where it
// cannot be (an absent or throwing `navigator` must still produce a report rather than an
// exception — the shell resolves an empty report to `en`, which is a usable panel).

describe('reportedLanguages', () => {
  test('the browser list is carried verbatim, in the browser order', () => {
    // The order *is* the preference order: the shell resolves it by walking the list, so a
    // reordered or deduplicated reading would answer a different question than the user did.
    expect(reportedLanguages({ languages: ['zh-Hans-CN', 'en-US'] })).toEqual(['zh-Hans-CN', 'en-US'])
    expect(reportedLanguages({ languages: ['en-US'] })).toEqual(['en-US'])
  })

  test('a webview with no list reports its single language instead', () => {
    // `navigator.language` is the documented fallback, and it is a complete answer on its own.
    expect(reportedLanguages({ language: 'zh-CN' })).toEqual(['zh-CN'])
    expect(reportedLanguages({ languages: [], language: 'en-US' })).toEqual(['en-US'])
  })

  test('an absent or unusable source is an empty report, never an exception', () => {
    // `[]` is not a language: the shell reads it as "nothing was reported" and keeps `en`.
    // `undefined` is not that case — it selects the live `navigator` (the default), which
    // is the one thing a test cannot assert without depending on jsdom's own locale.
    expect(reportedLanguages({})).toEqual([])
    expect(reportedLanguages({ languages: undefined, language: undefined })).toEqual([])
    expect(reportedLanguages({ languages: [] })).toEqual([])
    // A list of the wrong shape is not a list. The single value, if any, still stands.
    expect(reportedLanguages({ languages: 'zh-CN' as unknown as string[] })).toEqual([])
    expect(reportedLanguages({ languages: 'zh-CN' as unknown as string[], language: 'en-US' })).toEqual(['en-US'])
  })

  test('blank entries and non-strings are dropped, and the real tags survive', () => {
    // Only tags travel: an empty string or a number would put noise in the shell's record
    // of what this machine says without adding an answer to it.
    const source = { languages: ['', 'zh-CN', 42, '   ', null, 'en-US'] } as unknown as LanguageSource
    expect(reportedLanguages(source)).toEqual(['zh-CN', 'en-US'])
    // A list made only of noise falls through to the single language, as an empty one does.
    const noisy = { languages: ['', '  '], language: 'en-GB' } as unknown as LanguageSource
    expect(reportedLanguages(noisy)).toEqual(['en-GB'])
  })

  test('a navigator that refuses to answer yields an empty report rather than throwing', () => {
    // Reading these properties runs webview code, and an embedded webview can be told not
    // to answer. The report must not become the thing that fails.
    const refusing: LanguageSource = {
      get languages(): readonly string[] {
        throw new Error('the webview refused navigator.languages')
      },
      get language(): string {
        throw new Error('the webview refused navigator.language')
      },
    }
    expect(reportedLanguages(refusing)).toEqual([])

    // A refused list is not a refused report: the single language is still read.
    const listRefused: LanguageSource = {
      get languages(): readonly string[] {
        throw new Error('the webview refused navigator.languages')
      },
      language: 'zh-CN',
    }
    expect(reportedLanguages(listRefused)).toEqual(['zh-CN'])
  })
})

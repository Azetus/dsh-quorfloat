import { describe, expect, test } from 'vitest'
import { en } from '../src/lib/i18n/en'
import { zh } from '../src/lib/i18n/zh'
import {
  languageOf, msg, raw, resolveText, translate, type Language, type MessageKey,
} from '../src/lib/i18n'

describe('dictionaries', () => {
  test('both languages carry exactly the same keys', () => {
    // The compile-time half is the `Record<MessageKey, string>` annotation on `zh.ts`;
    // this is the same promise checked at runtime, so a key cannot be added to one file
    // and forgotten in the other without a failure here too.
    expect(Object.keys(zh).sort()).toEqual(Object.keys(en).sort())
  })

  test('every key has a non-empty string in both languages', () => {
    for (const key of Object.keys(en) as MessageKey[]) {
      expect(en[key], `en ${key}`).not.toBe('')
      expect(zh[key], `zh ${key}`).not.toBe('')
    }
  })

  test('the migrated surface is the size the migration counted', () => {
    // 96 keys covering the measured literals plus the endonyms and the new settings row.
    // A guard against a key silently disappearing in a refactor.
    expect(Object.keys(en).length).toBe(96)
  })
})

describe('languageOf', () => {
  test('exactly en is English, exactly zh is Chinese', () => {
    expect(languageOf('en')).toBe('en')
    expect(languageOf('zh')).toBe('zh')
  })

  test('an absent field is Chinese, the panel’s own default', () => {
    // The field is newer than some shells: reading it defensively is what keeps the panel
    // from crashing on a snapshot that predates it.
    expect(languageOf(undefined)).toBe('zh')
  })

  test('null, a number, and an unknown code are Chinese too', () => {
    expect(languageOf(null)).toBe('zh')
    expect(languageOf(42)).toBe('zh')
    expect(languageOf('fr')).toBe('zh')
    expect(languageOf('EN')).toBe('zh')
  })
})

describe('translate', () => {
  test('resolves one key per language', () => {
    expect(translate('zh', 'common.close')).toBe('关闭')
    expect(translate('en', 'common.close')).toBe('Close')
  })

  test('fills {placeholders}', () => {
    expect(translate('zh', 'stats.rounds', { turns: 3, steps: 9, speed: '229' })).toBe('3 轮 9 步 · 229 tok/s')
    expect(translate('en', 'stats.rounds', { turns: 3, steps: 9, speed: '229' })).toBe('Turns 3 · Steps 9 · 229 tok/s')
  })

  test('a placeholder with no param stays visible, never blank', () => {
    const out = translate('en', 'stats.rounds', { turns: 3 })
    expect(out).toContain('{steps}')
    expect(out).toContain('{speed}')
  })

  test('a key in neither dictionary renders as the key itself', () => {
    // The documented fallback ladder's last rung: a string is always on screen.
    expect(translate('en', 'nope.nope' as MessageKey)).toBe('nope.nope')
    expect(translate('zh', 'nope.nope' as MessageKey)).toBe('nope.nope')
  })

  test('an unknown language reads Chinese, never blank', () => {
    expect(translate('fr' as unknown as Language, 'common.close')).toBe('关闭')
  })
})

describe('resolveText', () => {
  test('raw host data passes through untouched', () => {
    expect(resolveText('en', raw('My workspace'))).toBe('My workspace')
    expect(resolveText('zh', raw('My workspace'))).toBe('My workspace')
  })

  test('a key token is looked up in the active language', () => {
    expect(resolveText('en', msg('session.new'))).toBe('New conversation')
    expect(resolveText('zh', msg('session.new'))).toBe('新会话')
  })

  test('a key token carries its own params', () => {
    expect(resolveText('zh', msg('time.minutesAgo', { minutes: 5 }))).toBe('5 分钟前')
    expect(resolveText('en', msg('time.minutesAgo', { minutes: 5 }))).toBe('5 min ago')
  })
})

import { describe, expect, test } from 'vitest'
import { msg, resolveText } from '../src/lib/i18n'
import { cacheLabel, contextPercent, roundsText, speedLabel, tokenLabel } from '../src/lib/stats'
import type { Stats } from '../src/lib/state'

describe('speedLabel', () => {
  test('ten and above round to an integer', () => {
    expect(speedLabel(229)).toBe('229')
    expect(speedLabel(229.6)).toBe('230')
  })
  test('below ten keeps one decimal', () => {
    expect(speedLabel(3.14159)).toBe('3.1')
  })
  test('nothing reported is a dash, never a zero', () => {
    expect(speedLabel(null)).toBe('—')
  })
})

describe('tokenLabel', () => {
  test('under a thousand is the number itself', () => {
    expect(tokenLabel(999)).toBe('999')
  })
  test('thousands use K with one decimal', () => {
    expect(tokenLabel(85_500)).toBe('85.5K')
  })
  test('millions use M with one decimal', () => {
    expect(tokenLabel(1_240_000)).toBe('1.2M')
  })
  test('nothing reported is a dash', () => {
    expect(tokenLabel(null)).toBe('—')
  })
})

describe('cacheLabel', () => {
  test('an exact full hit is a hundred', () => {
    expect(cacheLabel(100)).toBe('100%')
  })
  test('a partial hit never rounds up to a hundred', () => {
    expect(cacheLabel(99.96)).toBe('99.9%')
    expect(cacheLabel(99.5)).toBe('99.5%')
  })
  test('nothing reported is a dash', () => {
    expect(cacheLabel(null)).toBe('—')
  })
})

describe('contextPercent', () => {
  test('over the window clamps at a hundred', () => {
    expect(contextPercent(1500, 1000)).toBe(100)
  })
  test('a share is one decimal', () => {
    expect(contextPercent(515, 1000)).toBe(51.5)
  })
  test('missing either number is not a number', () => {
    expect(contextPercent(null, 1000)).toBeNull()
    expect(contextPercent(10, null)).toBeNull()
  })
})

/** A stats record with everything the rounds group reads. */
const stats = (over: Partial<Stats>): Stats => ({
  turns: 0,
  steps: 0,
  tokensPerSecond: null,
  totalTokens: null,
  cacheHitPercent: null,
  contextTokens: null,
  contextLimit: null,
  ...over,
})

describe('roundsText', () => {
  test('it returns the key and the figures, not a sentence', () => {
    expect(roundsText(stats({ turns: 3, steps: 9, tokensPerSecond: 229 })))
      .toEqual(msg('stats.rounds', { turns: 3, steps: 9, speed: '229' }))
  })

  test('the dictionary phrases it per language', () => {
    const text = roundsText(stats({ turns: 3, steps: 9, tokensPerSecond: 229 }))
    expect(resolveText('zh', text)).toBe('3 轮 9 步 · 229 tok/s')
    expect(resolveText('en', text)).toBe('Turns 3 · Steps 9 · 229 tok/s')
  })

  test('an unreported speed keeps the tested dash, in both languages', () => {
    const text = roundsText(stats({ turns: 1, steps: 2 }))
    expect(resolveText('zh', text)).toBe('1 轮 2 步 · — tok/s')
    expect(resolveText('en', text)).toBe('Turns 1 · Steps 2 · — tok/s')
  })
})

import { describe, expect, test } from 'vitest'
import { cacheLabel, contextPercent, speedLabel, tokenLabel } from '../src/lib/stats'

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

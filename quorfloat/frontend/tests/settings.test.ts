import { describe, expect, test } from 'vitest'
import { keepOpenChecked, themeColorScheme } from '../src/lib/settings'

describe('themeColorScheme', () => {
  test('following the system keeps both palettes', () => {
    expect(themeColorScheme('system')).toBe('light dark')
  })
  test('an explicit theme pins one palette', () => {
    expect(themeColorScheme('light')).toBe('light')
    expect(themeColorScheme('dark')).toBe('dark')
  })
})

describe('keepOpenChecked', () => {
  test('the design label and the configuration behaviour meet once, inverted', () => {
    // hideOnBlur=false means the panel stays expanded: the switch reads "on".
    expect(keepOpenChecked(false)).toBe('true')
    expect(keepOpenChecked(true)).toBe('false')
  })
})

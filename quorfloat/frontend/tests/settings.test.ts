import { describe, expect, test } from 'vitest'
import { hidesOnBlur, keepOpenChecked, themeColorScheme } from '../src/lib/settings'

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

describe('hidesOnBlur', () => {
  test('the panel hides exactly when the setting says it hides', () => {
    // `hidesOnBlur` must not be negated: this flag and the setting say the same
    // thing, while the switch renders their inverse (`keepOpenChecked`).
    expect(hidesOnBlur(true)).toBe(true)
    expect(hidesOnBlur(false)).toBe(false)
  })
})

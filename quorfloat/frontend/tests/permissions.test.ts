// Category 1 (interface): the built-in permission labels the panel owns.
//
// The host supplies a stable `value` and a display `name`; the name may be written in the
// Harness's language rather than the panel's. These cases pin the boundary: the three
// conventional presets come from our dictionary, and every other preset keeps its name.

import { describe, expect, test } from 'vitest'
import { resolveText } from '../src/logic/i18n'
import { permissionLabel } from '../src/logic/permissions'

describe('built-in permission labels', () => {
  test.each([
    ['read-only', 'Read Only', '仅可查看'],
    ['workspace-write', 'Workspace Write', '工作区内修改'],
    ['danger-full-access', 'Full access', '完全权限'],
  ])('%s follows the panel language, not the host label', (value, english, chinese) => {
    const text = permissionLabel(value, 'host label')
    expect(resolveText('en', text)).toBe(english)
    expect(resolveText('zh', text)).toBe(chinese)
  })
})

describe('configured permission labels', () => {
  test('an unknown value keeps the host name verbatim', () => {
    const text = permissionLabel('team-read', 'Team read')
    expect(resolveText('en', text)).toBe('Team read')
    expect(resolveText('zh', text)).toBe('Team read')
  })

  test('a built-in value still maps even when the host name is localized', () => {
    const text = permissionLabel('read-only', '仅可查看')
    expect(resolveText('en', text)).toBe('Read Only')
    expect(resolveText('zh', text)).toBe('仅可查看')
  })
})

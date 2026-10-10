import { describe, expect, test } from 'vitest'
import { msg, resolveText } from '../src/lib/i18n'
import { chordFromEvent, isBareTypingKey, validateChord } from '../src/lib/hotkey'

/** A keydown the recorder would capture. */
function press(code: string, modifiers: Partial<Pick<KeyboardEvent, 'metaKey' | 'ctrlKey' | 'altKey' | 'shiftKey'>> = {}): KeyboardEvent {
  return new KeyboardEvent('keydown', {
    code,
    metaKey: modifiers.metaKey ?? false,
    ctrlKey: modifiers.ctrlKey ?? false,
    altKey: modifiers.altKey ?? false,
    shiftKey: modifiers.shiftKey ?? false,
  })
}

describe('chordFromEvent', () => {
  test('a chord is spelled in the fixed order the host writes', () => {
    expect(chordFromEvent(press('KeyK', { metaKey: true, shiftKey: true }))).toBe('Cmd+Shift+K')
    expect(chordFromEvent(press('KeyK', { ctrlKey: true, altKey: true }))).toBe('Ctrl+Alt+K')
  })
  test('letters, digits and punctuation map to their names', () => {
    expect(chordFromEvent(press('KeyK'))).toBe('K')
    expect(chordFromEvent(press('Digit7'))).toBe('7')
    expect(chordFromEvent(press('BracketLeft'))).toBe('[')
    expect(chordFromEvent(press('F13'))).toBe('F13')
    expect(chordFromEvent(press('Space'))).toBe('Space')
  })
  test('a modifier alone is not a chord', () => {
    expect(chordFromEvent(press('ShiftLeft', { shiftKey: true }))).toBeNull()
    expect(chordFromEvent(press('MetaLeft', { metaKey: true }))).toBeNull()
  })
  test('a key this build does not know is not a chord', () => {
    expect(chordFromEvent(press('OSLeft'))).toBeNull()
  })
})

describe('validateChord', () => {
  test('a bare typing key is refused, in the shell\'s words', () => {
    const verdict = validateChord('M')
    expect(verdict.ok).toBe(false)
    if (!verdict.ok) expect(resolveText('zh', verdict.reason)).toContain('裸按键')
  })
  test('the refusal is a key, so the reason follows the language', () => {
    const bare = validateChord('M')
    const empty = validateChord('')
    const twoKeys = validateChord('Alt+K+J')
    expect(bare).toEqual({ ok: false, reason: msg('hotkey.bareKey') })
    expect(empty).toEqual({ ok: false, reason: msg('hotkey.unreadable', { chord: '' }) })
    expect(twoKeys).toEqual({ ok: false, reason: msg('hotkey.unreadable', { chord: 'Alt+K+J' }) })
    if (!bare.ok) expect(resolveText('en', bare.reason)).toContain('bare key')
    if (!twoKeys.ok) expect(resolveText('en', twoKeys.reason)).toContain('Alt+K+J')
  })
  test('a bare function key is exempt', () => {
    expect(validateChord('F13').ok).toBe(true)
  })
  test('a modifier makes any typing key acceptable', () => {
    expect(validateChord('Cmd+Shift+K').ok).toBe(true)
    expect(validateChord('Ctrl+7').ok).toBe(true)
  })
  test('a chord with two keys is refused', () => {
    expect(validateChord('Alt+K+J').ok).toBe(false)
  })
})

describe('isBareTypingKey', () => {
  test('letters, digits and punctuation are typing keys; F-keys are not', () => {
    expect(isBareTypingKey('M')).toBe(true)
    expect(isBareTypingKey('7')).toBe(true)
    expect(isBareTypingKey('/')).toBe(true)
    expect(isBareTypingKey('F13')).toBe(false)
    expect(isBareTypingKey('Space')).toBe(false)
  })
})

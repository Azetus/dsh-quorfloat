// Category 1 (interface): the shortcut recorder's pure key policy and recording decisions.
//
// These are the rules the shell mirrors in `app/bridge.rs`. The regression they guard is
// not cosmetic: a bare Space/Enter/Tab accepted here becomes a desktop-wide grab, and a
// recording that cannot be cancelled leaves every key intercepted.

import { describe, expect, test } from 'vitest'
import { recordingStep, validateChord, type KeyStroke } from '../src/logic/hotkey'

/** A keydown with no modifiers unless the case says otherwise. */
function stroke(key: string, code: string, modifiers: Partial<KeyStroke> = {}): KeyStroke {
  return { key, code, metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, ...modifiers }
}

describe('the bare-key policy', () => {
  test.each([
    'Space', 'Enter', 'Tab', 'Backspace', 'Delete', 'Insert', 'Home', 'End',
    'PageUp', 'PageDown', 'Up', 'Down', 'Left', 'Right', 'Escape', 'Esc',
    'a', 'Z', '0', '9', ';', '/', '`',
  ])('%s cannot be grabbed without a modifier', chord => {
    expect(validateChord(chord).ok).toBe(false)
  })

  test.each(['F1', 'F12', 'F13', 'F24'])('%s is the function-key exemption', chord => {
    expect(validateChord(chord).ok).toBe(true)
  })

  test.each(['Alt+Space', 'Ctrl+Enter', 'Shift+Tab', 'Shift+A', 'Cmd+K', 'Ctrl+Shift+F5'])(
    '%s carries a modifier and is allowed',
    chord => {
      expect(validateChord(chord).ok).toBe(true)
    },
  )

  test('an empty chord is unreadable, not a bare key', () => {
    expect(validateChord('').ok).toBe(false)
  })
})

describe('what a keydown means while recording', () => {
  test('Escape cancels instead of becoming a chord', () => {
    expect(recordingStep(stroke('Escape', 'Escape'))).toEqual({ kind: 'cancel' })
  })

  test('a bare Enter yields instead of being captured', () => {
    expect(recordingStep(stroke('Enter', 'Enter'))).toEqual({ kind: 'yield' })
  })

  test('a bare Tab yields, so focus can move on', () => {
    expect(recordingStep(stroke('Tab', 'Tab'))).toEqual({ kind: 'yield' })
  })

  test('modified Enter and Tab are still recordable chords', () => {
    expect(recordingStep(stroke('Enter', 'Enter', { ctrlKey: true }))).toEqual({ kind: 'chord', chord: 'Ctrl+Enter' })
    expect(recordingStep(stroke('Tab', 'Tab', { shiftKey: true }))).toEqual({ kind: 'chord', chord: 'Shift+Tab' })
  })

  test('Space becomes a chord the policy then refuses', () => {
    const step = recordingStep(stroke(' ', 'Space'))
    expect(step).toEqual({ kind: 'chord', chord: 'Space' })
    if (step.kind === 'chord') expect(validateChord(step.chord).ok).toBe(false)
  })

  test('a bare letter and a function key are offered as chords', () => {
    expect(recordingStep(stroke('a', 'KeyA'))).toEqual({ kind: 'chord', chord: 'A' })
    expect(recordingStep(stroke('F13', 'F13'))).toEqual({ kind: 'chord', chord: 'F13' })
  })

  test('modifier-only and unmapped keys keep the recorder armed', () => {
    expect(recordingStep(stroke('Shift', 'ShiftLeft', { shiftKey: true }))).toEqual({ kind: 'ignore' })
    expect(recordingStep(stroke('MediaPlayPause', 'MediaPlayPause'))).toEqual({ kind: 'ignore' })
  })
})

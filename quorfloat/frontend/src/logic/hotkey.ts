// The shortcut recorder: turning a keyboard press into the host's accelerator
// spelling, with the same rules the shell enforces (`app/bridge.rs`:
// `validate_hotkey`). The two halves must agree, or a chord the panel records
// would be one the shell refuses on the next start.

import { msg, type Text } from './i18n'

/** The accelerator a keydown stands for, or null for a key that is not one. */
export function chordFromEvent(event: KeyboardEvent): string | null {
  if (isModifier(event.code)) return null
  const key = keyName(event.code)
  if (key === null) return null
  const parts: string[] = []
  if (event.metaKey) parts.push('Cmd')
  if (event.ctrlKey) parts.push('Ctrl')
  if (event.altKey) parts.push('Alt')
  if (event.shiftKey) parts.push('Shift')
  parts.push(key)
  return parts.join('+')
}

/**
 * Whether a chord the recorder captured is one the shell will accept.
 *
 * Mirrors the Rust rule: a **bare** letter, digit, or punctuation key would be
 * grabbed from the whole desktop and take that key away from every application.
 * Function keys are exempt — nobody types an F13 by accident.
 *
 * @returns `{ ok: true }`, or the reason as a dictionary key the caller resolves.
 */
export function validateChord(chord: string): { ok: true } | { ok: false; reason: Text } {
  if (chord.trim() === '') return { ok: false, reason: msg('hotkey.unreadable', { chord: '' }) }
  const parts = chord.split('+').map(part => part.trim())
  const modifiers = new Set(['cmd', 'command', 'super', 'meta', 'win', 'ctrl', 'control', 'alt', 'option', 'opt', 'shift'])
  const keys = parts.filter(part => !modifiers.has(part.toLowerCase()))
  if (keys.length !== 1) return { ok: false, reason: msg('hotkey.unreadable', { chord }) }
  const key = keys[0]!
  if (parts.length === 1 && isBareTypingKey(key)) {
    return { ok: false, reason: msg('hotkey.bareKey') }
  }
  return { ok: true }
}

/** The bare keys that would steal typing from every application. */
export function isBareTypingKey(name: string): boolean {
  return (
    /^[A-Z]$/.test(name) ||
    /^[0-9]$/.test(name) ||
    ['`', '-', '=', '[', ']', '\\', ';', "'", ',', '.', '/'].includes(name)
  )
}

/** Map a `KeyboardEvent.code` to the accelerator key name, or null. */
function keyName(code: string): string | null {
  const named: Record<string, string> = {
    Space: 'Space',
    Enter: 'Enter',
    Tab: 'Tab',
    Backspace: 'Backspace',
    Delete: 'Delete',
    Insert: 'Insert',
    Home: 'Home',
    End: 'End',
    PageUp: 'PageUp',
    PageDown: 'PageDown',
    ArrowUp: 'Up',
    ArrowDown: 'Down',
    ArrowLeft: 'Left',
    ArrowRight: 'Right',
    Backquote: '`',
    Minus: '-',
    Equal: '=',
    BracketLeft: '[',
    BracketRight: ']',
    Backslash: '\\',
    Semicolon: ';',
    Quote: "'",
    Comma: ',',
    Period: '.',
    Slash: '/',
  }
  if (code in named) return named[code] ?? null
  if (/^Key[A-Z]$/.test(code)) return code.slice(3)
  if (/^Digit[0-9]$/.test(code)) return code.slice(5)
  if (/^F([1-9]|1[0-9]|2[0-4])$/.test(code)) return code
  return null
}

function isModifier(code: string): boolean {
  return ['MetaLeft', 'MetaRight', 'ControlLeft', 'ControlRight', 'AltLeft', 'AltRight', 'ShiftLeft', 'ShiftRight'].includes(code)
}

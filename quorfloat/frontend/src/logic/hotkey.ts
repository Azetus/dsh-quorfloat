// The shortcut recorder: turning a keyboard press into the host's accelerator
// spelling, deciding when a press means "stop recording" instead, and enforcing
// the same bare-key rule the shell enforces (`app/bridge.rs`: `validate_hotkey`).
// The two halves must agree, or a chord the panel records would be one the shell
// refuses on the next start.

import { msg, type Text } from './i18n'

/** The fields of a keyboard event the recorder reads; `KeyboardEvent` fits it. */
export interface KeyStroke {
  readonly key: string
  readonly code: string
  readonly metaKey: boolean
  readonly ctrlKey: boolean
  readonly altKey: boolean
  readonly shiftKey: boolean
}

/**
 * What one keydown means while the recorder is armed.
 *
 * - `cancel`: Escape; stop recording without changing the accelerator;
 * - `yield`: a bare Enter or Tab; stop recording and let the key do its normal job;
 * - `ignore`: a modifier-only or unmapped key; stay armed;
 * - `chord`: a candidate accelerator for {@link validateChord}.
 */
export type RecordingStep =
  | { readonly kind: 'cancel' }
  | { readonly kind: 'yield' }
  | { readonly kind: 'ignore' }
  | { readonly kind: 'chord'; readonly chord: string }

/**
 * Decide what a keydown means while recording.
 *
 * Escape is the recorder's cancel key, not a chord: the shell can parse `Escape`,
 * but a global Escape would close menus in every application. A completely bare
 * Enter or Tab is navigation, not a shortcut, so it ends recording instead of
 * becoming a grab; modified forms (`Ctrl+Enter`, `Shift+Tab`) are still chords.
 *
 * @param event - the keydown's relevant fields.
 * @returns what the caller should do next.
 */
export function recordingStep(event: KeyStroke): RecordingStep {
  if (event.key === 'Escape') return { kind: 'cancel' }
  const bare = !event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey
  if (bare && (event.key === 'Enter' || event.key === 'Tab')) return { kind: 'yield' }
  const chord = chordFromEvent(event)
  return chord === null ? { kind: 'ignore' } : { kind: 'chord', chord }
}

/** The accelerator a keydown stands for, or null for a key that is not one. */
export function chordFromEvent(event: KeyStroke): string | null {
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
 * The shell grabs an accelerator from the whole desktop. A chord with no modifier
 * at all would therefore take that key away from every application — and the keys
 * that hurt most are not letters: Space, Enter, Tab, Backspace, and the arrow keys
 * are all typing or navigation. The one exception is the function-key row: nobody
 * types an F13 by accident, so bare `F1`–`F24` stay available.
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
  const hasModifier = parts.some(part => modifiers.has(part.toLowerCase()))
  if (!hasModifier && !isFunctionKey(key)) {
    return { ok: false, reason: msg('hotkey.bareKey') }
  }
  return { ok: true }
}

/** Whether a bare key is the one exemption: the function-key row. */
function isFunctionKey(name: string): boolean {
  return /^F([1-9]|1[0-9]|2[0-4])$/.test(name)
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

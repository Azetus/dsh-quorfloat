// The summon-shortcut field: click it, press the chord, done.

import { useT } from '../../hooks/useI18n'
import { reasonText } from '../../logic/reasons'
import type { Reason } from '../../logic/state'

/** Props for {@link HotkeyField}. */
export interface HotkeyFieldProps {
  /** The accelerator the shell last accepted, or null. */
  readonly requested: string | null
  /** Why the current accelerator is not in force, if it is not. */
  readonly reason: Reason | null
  /** Whether the field is waiting for a chord. */
  readonly recording: boolean
  /** Begin recording. */
  readonly onRecord: () => void
  /** Stop recording without changing the chord. */
  readonly onCancel: () => void
}

/**
 * Render the shortcut field.
 *
 * It is a real input so it can be focused and clicked like one, but it is read-only: the
 * chord comes from the keyboard handler, not from typing. Starting a recording is
 * deliberately pointer-only; once armed, Enter/Tab are the recorder's exit rules, and a
 * focused read-only value must not double as a button.
 *
 * @param props - the chord, its reason, and the recording state.
 * @returns the field.
 */
export function HotkeyField({ requested, reason, recording, onRecord, onCancel }: HotkeyFieldProps) {
  const t = useT()
  return (
    <input
      id="q-shortcut"
      className={recording ? 'q-shortcut q-recording' : 'q-shortcut'}
      // `'—'` is the design's "no chord yet" placeholder: a symbol, not a word, so it is
      // the same in both languages and stays out of the dictionary.
      value={recording ? t('hotkey.recording') : requested ?? '—'}
      aria-label={t('settings.hotkey')}
      title={recording || reason === null ? '' : t(reasonText(reason))}
      readOnly
      // Starting a recording is pointer-only by design: the click is the explicit "I am
      // changing this" gesture, and the keyboard is only the chord source afterwards.
      onClick={onRecord}
      // A recording belongs to this field. Moving focus away ends it, so clicking another
      // setting (or leaving by Tab) cannot leave the document handler armed.
      onBlur={() => { if (recording) onCancel() }}
    />
  )
}

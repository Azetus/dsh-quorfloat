// The summon-shortcut field: click it, press the chord, done.

import { useT } from '../../hooks/useI18n'

/** Props for {@link HotkeyField}. */
export interface HotkeyFieldProps {
  /** The accelerator the shell last accepted, or null. */
  readonly requested: string | null
  /** Why the current accelerator is not in force, if it is not. */
  readonly reason: string | null
  /** Whether the field is waiting for a chord. */
  readonly recording: boolean
  /** Begin recording. */
  readonly onRecord: () => void
}

/**
 * Render the shortcut field.
 *
 * It is a real input so it can be focused and clicked like one, but it is read-only: the
 * chord comes from the keyboard handler, not from typing.
 *
 * @param props - the chord, its reason, and the recording state.
 * @returns the field.
 */
export function HotkeyField({ requested, reason, recording, onRecord }: HotkeyFieldProps) {
  const t = useT()
  return (
    <input
      id="q-shortcut"
      className={recording ? 'q-shortcut q-recording' : 'q-shortcut'}
      // `'—'` is the design's "no chord yet" placeholder: a symbol, not a word, so it is
      // the same in both languages and stays out of the dictionary.
      value={recording ? t('hotkey.recording') : requested ?? '—'}
      aria-label={t('settings.hotkey')}
      title={recording ? '' : reason ?? ''}
      readOnly
      onClick={onRecord}
    />
  )
}

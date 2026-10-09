// The summon-shortcut field: click it, press the chord, done.

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
  return (
    <input
      id="q-shortcut"
      className={recording ? 'q-shortcut q-recording' : 'q-shortcut'}
      value={recording ? '按下组合键…' : requested ?? '—'}
      aria-label="呼出快捷键"
      title={recording ? '' : reason ?? ''}
      readOnly
      onClick={onRecord}
    />
  )
}

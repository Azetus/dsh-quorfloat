// The keep-open switch: stay up when the user moves to another app.
//
// The setting's name is the *user's* ("失焦时保持展开"); the preference it writes is the
// process's (`hideOnBlur`), and `keepOpenChecked` is the one place they are converted —
// writing both spellings everywhere would make every reader invert it themselves.

import { useEffect, useRef } from 'react'
import { api } from '../../api'
import { keepOpenChecked } from '../../lib/settings'

/** Props for {@link KeepOpenSwitch}. */
export interface KeepOpenSwitchProps {
  /** The shell's `hideOnBlur`. */
  readonly hideOnBlur: boolean
}

/**
 * Render the switch.
 *
 * The live DOM's own state is put on the record whenever it changes (the `frontend switch
 * aria-checked=…` marker): it is the last link in the ground-truth chain — if the marker
 * says one thing and the screen shows another, the divergence is in rendering, not binding.
 *
 * @param props - the shell's setting.
 * @returns the switch.
 */
export function KeepOpenSwitch({ hideOnBlur }: KeepOpenSwitchProps) {
  const checked = keepOpenChecked(hideOnBlur)
  const previous = useRef<string | null>(null)

  useEffect(() => {
    if (previous.current === checked) return
    previous.current = checked
    void api.log(`frontend switch aria-checked=${checked} hideOnBlur=${hideOnBlur}`)
  }, [checked, hideOnBlur])

  return (
    <button
      type="button"
      id="q-keep-open"
      className="q-switch"
      role="switch"
      aria-checked={checked === 'true'}
      onClick={() => { void api.setPreferences({ keepOpen: checked !== 'true' }) }}
    >
      <span className="q-knob" />
    </button>
  )
}

// The appearance chips: follow the system, or pin one palette.

import { api } from '../../api'
import { useT } from '../../hooks/useI18n'
import type { MessageKey } from '../../lib/i18n'
import type { Snapshot } from '../../lib/state'

/** Props for {@link ThemeChips}. */
export interface ThemeChipsProps {
  /** The theme in force. */
  readonly theme: Snapshot['settings']['theme']
}

const CHOICES: readonly { readonly value: Snapshot['settings']['theme']; readonly label: MessageKey }[] = [
  { value: 'system', label: 'theme.system' },
  { value: 'light', label: 'theme.light' },
  { value: 'dark', label: 'theme.dark' },
]

/**
 * Render the theme chips.
 *
 * A pick writes the preference through the shell, which validates and stores it — the
 * panel never keeps its own copy of a setting the host owns.
 *
 * @param props - the current theme.
 * @returns the chip group.
 */
export function ThemeChips({ theme }: ThemeChipsProps) {
  const t = useT()
  return (
    <span className="q-chips" id="q-theme-chips">
      {CHOICES.map(choice => (
        <button
          key={choice.value}
          type="button"
          className="q-chip"
          data-theme={choice.value}
          aria-pressed={choice.value === theme}
          onClick={() => { void api.setPreferences({ theme: choice.value }) }}
        >
          {t(choice.label)}
        </button>
      ))}
    </span>
  )
}

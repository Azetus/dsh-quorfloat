// A pin toggle (`.q-pin`) used by the session and workspace rows.

import { useT } from '../../hooks/useI18n'
import { Icon } from '../common/Icon'

/** Props for {@link PinButton}. */
export interface PinButtonProps {
  /** What is being pinned, for the accessible label. */
  readonly name: string
  /** Whether it is pinned now. */
  readonly pinned: boolean
  /** Called with the requested new state. */
  readonly onPick: () => void
}

/**
 * Render the pin toggle.
 *
 * @param props - subject name, current state, and handler.
 * @returns the pin button.
 */
export function PinButton({ name, pinned, onPick }: PinButtonProps) {
  const t = useT()
  return (
    <button
      type="button"
      className="q-pin"
      aria-label={t(pinned ? 'pin.unpinName' : 'pin.pinName', { name })}
      aria-pressed={pinned}
      onClick={onPick}
    >
      <Icon name={pinned ? 'pin-off' : 'pin'} />
    </button>
  )
}

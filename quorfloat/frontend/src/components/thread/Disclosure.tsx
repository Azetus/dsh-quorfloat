// The per-turn disclosure row: caret, label, and — while the turn is being worked on —
// the panel's own turning mark.

import { useT } from '../../hooks/useI18n'
import { Icon } from '../common/Icon'

/** Props for {@link Disclosure}. */
export interface DisclosureProps {
  /** Whether the folded part is open. */
  readonly open: boolean
  /** Whether the turn is still being worked on (label and mark). */
  readonly working: boolean
  /** Toggle request. */
  readonly onToggle: () => void
}

/**
 * Render the disclosure row.
 *
 * The caret is always present — it is the row's affordance, and the row is clickable
 * while the turn runs too. The turning mark is appended *after* the label rather than
 * taking the caret's cell, so switching between working and settled moves nothing.
 *
 * @param props - open state, working state, and the toggle handler.
 * @returns the row.
 */
export function Disclosure({ open, working, onToggle }: DisclosureProps) {
  const t = useT()
  return (
    <button
      type="button"
      className="q-disclosure"
      aria-expanded={open}
      onClick={onToggle}
    >
      <span className="q-caret" style={{ transform: open ? 'rotate(90deg)' : '' }}>
        <Icon name="caret-right" />
      </span>
      <span>{working ? t('disclosure.working') : t('disclosure.done')}</span>
      {working && (
        <span className="q-spin">
          <svg viewBox="0 0 256 256" width="11" height="11" aria-hidden="true">
            <path
              fill="currentColor"
              d="M232 128a104 104 0 0 1-208 0c0-6.2 5-11.2 11.2-11.2s11.2 5 11.2 11.2a81.6 81.6 0 0 0 163.2 0c0-6.2 5-11.2 11.2-11.2S232 121.8 232 128Z"
            />
          </svg>
        </span>
      )}
    </button>
  )
}

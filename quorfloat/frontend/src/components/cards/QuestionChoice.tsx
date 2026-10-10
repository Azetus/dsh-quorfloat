// One option of a question: a drawn radio (single-select) or check (multi-select).
//
// A plain button with `role`/`aria-checked`, like the settings switch: the panel draws its
// own marks from the design's tokens, and a native input cannot be styled to match them.

import { Icon } from '../common/Icon'

/** Props for {@link QuestionChoice}. */
export interface QuestionChoiceProps {
  /** The option's label — what an answer sends back. */
  readonly label: string
  /** Optional extra context from the asker. */
  readonly description: string | null
  /** Whether this option is picked. */
  readonly selected: boolean
  /** Whether more than one may be picked (decides the mark and the role). */
  readonly multiSelect: boolean
  /** Whether the card is frozen (submitting, or already resolved). */
  readonly disabled: boolean
  /** Pick or unpick this option. */
  readonly onPick: () => void
}

/**
 * Render one choice.
 *
 * @param props - the option, its state, and the handler.
 * @returns the choice row.
 */
export function QuestionChoice({
  label, description, selected, multiSelect, disabled, onPick,
}: QuestionChoiceProps) {
  return (
    <button
      type="button"
      className={selected ? 'q-choice q-choice-on' : 'q-choice'}
      role={multiSelect ? 'checkbox' : 'radio'}
      aria-checked={selected}
      disabled={disabled}
      onClick={onPick}
    >
      <span className="q-choice-mark">
        {selected && (multiSelect ? <Icon name="check" className="q-choice-check" /> : <span className="q-choice-dot" />)}
      </span>
      <span className="q-choice-label">
        <span>{label}</span>
        {description !== null && description !== '' && <small>{description}</small>}
      </span>
    </button>
  )
}

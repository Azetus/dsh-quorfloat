// One row in a picker: the design's `.q-option` (icon, label, description, check).

import { Icon } from '../common/Icon'

/** Props for {@link MenuOption}. */
export interface MenuOptionProps {
  /** What the user reads. */
  readonly label: string
  /** Whether this row is the one in force. */
  readonly selected: boolean
  /** Called when the row is picked. */
  readonly onPick: () => void
  /** Optional second line, when the catalog carries one. */
  readonly description?: string
  /** Optional leading glyph key. */
  readonly symbol?: string | null
  /** Native tooltip, e.g. a workspace path too long for the row. */
  readonly title?: string
}

/**
 * Render one pickable row.
 *
 * @param props - label, selection, handler, and optional glyph/description.
 * @returns the row button.
 */
export function MenuOption({ label, selected, onPick, description = '', symbol = null, title }: MenuOptionProps) {
  return (
    <button type="button" className="q-option" aria-pressed={selected} onClick={onPick} title={title}>
      {symbol !== null && <Icon name={symbol} />}
      <span className="q-option-main">
        <span>{label}</span>
        {description !== '' && <small>{description}</small>}
      </span>
      {selected && <Icon name="check" className="q-check" />}
    </button>
  )
}

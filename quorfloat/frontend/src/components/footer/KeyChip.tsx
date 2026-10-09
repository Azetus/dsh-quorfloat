// One key chip in the footer's hint line.

import { Icon } from '../common/Icon'

/** Props for {@link KeyChip}. */
export interface KeyChipProps {
  /** Which key the chip spells. */
  readonly kind: 'enter' | 'shift-enter' | 'esc'
}

/**
 * Render one `<kbd>`.
 *
 * The design's own markup: the ↵ is a drawn icon (the bundled font has no U+21B5), while
 * ⇧ and `esc` are text — exactly as the design writes them.
 *
 * @param props - which chip.
 * @returns the chip.
 */
export function KeyChip({ kind }: KeyChipProps) {
  if (kind === 'esc') return <kbd>esc</kbd>
  return (
    <kbd>
      {kind === 'shift-enter' && '⇧ '}
      <Icon name="corner-down-left" />
    </kbd>
  )
}

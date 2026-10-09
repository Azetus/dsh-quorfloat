// One `.q-option-row`: a menu option next to its pin button.

import type { ReactNode } from 'react'

/** Props for {@link MenuRow}. */
export interface MenuRowProps {
  /** The row's controls (the option and, when it has one, its pin). */
  readonly children: ReactNode
}

/**
 * Render the row wrapper.
 *
 * @param props - the row's controls.
 * @returns the row.
 */
export function MenuRow({ children }: MenuRowProps) {
  return <div className="q-option-row">{children}</div>
}

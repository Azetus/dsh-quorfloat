// The panel footer: key hints, the status line, and the conversation's numbers.

import type { Snapshot } from '../lib/state'
import { statusText } from '../lib/view-text'
import { StatsRow } from './StatsRow'
import { StatusLine } from './StatusLine'

/** Props for {@link Footer}. */
export interface FooterProps {
  /** The snapshot. */
  readonly state: Snapshot
  /** A locally produced status message (a rejected chord), if any. */
  readonly transientError: string | null
}

/**
 * Render the footer.
 *
 * @param props - snapshot and the transient message.
 * @returns the footer.
 */
export function Footer({ state, transientError }: FooterProps) {
  return (
    <div className="q-footer">
      <StatusLine status={statusText(state, transientError)} />
      <StatsRow stats={state.session.stats} />
    </div>
  )
}

// The panel footer: key hints, the status line, and the conversation's numbers.

import { useT } from '../../hooks/useI18n'
import type { Snapshot } from '../../logic/state'
import { statusText } from '../../logic/view-text'
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
 * The status is resolved here rather than inside {@link StatusLine}: `statusText` returns a
 * token (host data to show verbatim, or a dictionary key plus its data), and this component is
 * the one that already has the language in scope through `useT`.
 *
 * @param props - snapshot and the transient message.
 * @returns the footer.
 */
export function Footer({ state, transientError }: FooterProps) {
  const t = useT()
  const status = statusText(state, transientError)
  return (
    <div className="q-footer">
      <StatusLine status={status === null ? null : t(status)} />
      <StatsRow stats={state.session.stats} />
    </div>
  )
}

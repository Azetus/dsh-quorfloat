// The footer's right half: the conversation's numbers, each only when it can be computed.

import { ring } from '../../lib/icons'
import type { Stats } from '../../lib/state'
import { contextLabel, roundsLabel, tokensLabel } from '../../lib/stats'
import { Icon } from '../common/Icon'

/** Props for {@link StatsRow}. */
export interface StatsRowProps {
  /** The host's figures, or null before it has reported any. */
  readonly stats: Stats | null
}

/**
 * Render the statistics row.
 *
 * Every number is conditional: a figure the host did not report is left out rather than
 * shown as zero, and the context meter only exists when a limit is known.
 *
 * @param props - the statistics.
 * @returns the row, or nothing when there are none.
 */
export function StatsRow({ stats }: StatsRowProps) {
  if (stats === null) return null
  const tokens = tokensLabel(stats)
  const context = contextLabel(stats)
  const percent = stats.contextTokens !== null && stats.contextLimit !== null && stats.contextLimit > 0
    ? Math.min(100, (stats.contextTokens / stats.contextLimit) * 100)
    : 0
  return (
    <div id="q-session-stats" className="q-session-stats" aria-label="会话统计">
      <span className="q-stat" id="q-stat-rounds">
        <span id="q-stat-speed-icon"><Icon name="gauge" /></span>
        <span id="q-stat-speed">{roundsLabel(stats)}</span>
      </span>
      <span className="q-stat" id="q-stat-tokens">
        <span id="q-stat-token-icon"><Icon name="database" /></span>
        <span>
          <span id="q-stat-token-count">{tokens.count}</span>
          <span className="q-stat-separator"> · </span>
          {`缓存命中 `}
          <span id="q-stat-cache">{tokens.cache}</span>
        </span>
      </span>
      {context !== null && (
        <span className="q-stat" id="q-stat-context">
          <span id="q-stat-context-icon" dangerouslySetInnerHTML={{ __html: ring(percent) }} />
          <span id="q-stat-context-value">{context}</span>
        </span>
      )}
    </div>
  )
}

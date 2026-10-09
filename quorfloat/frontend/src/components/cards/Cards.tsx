// The card strip above the thread: pending interactions and the handoff notice.

import type { Snapshot } from '../../lib/state'
import { ApprovalCard } from './ApprovalCard'
import { HandoffBanner } from './HandoffBanner'
import { QuestionCard } from './QuestionCard'

/** Props for {@link Cards}. */
export interface CardsProps {
  /** The snapshot. */
  readonly state: Snapshot
  /** A field in a card grew: the panel has to remeasure. */
  readonly onGrow: () => void
}

/**
 * Render every card the panel currently owes the user.
 *
 * Approvals and questions are different mechanisms with different answers, so they get
 * different cards; the store never mixes them.
 *
 * @param props - the snapshot.
 * @returns the card strip.
 */
export function Cards({ state, onGrow }: CardsProps) {
  return (
    <div id="q-cards" className="q-cards">
      {state.interactions.map(interaction => (
        interaction.kind === 'approval'
          ? <ApprovalCard key={interaction.id} interaction={interaction} />
          : <QuestionCard key={interaction.id} interaction={interaction} onGrow={onGrow} />
      ))}
      {state.handoff !== null && <HandoffBanner handoff={state.handoff} />}
    </div>
  )
}

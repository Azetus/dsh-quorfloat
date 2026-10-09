// One turn: the question, the folded working, and the answer.

import type { Turn as TurnModel } from '../lib/turns'
import { AnswerBar } from './AnswerBar'
import { AnswerBody } from './AnswerBody'
import { Disclosure } from './Disclosure'
import { WorkingLine } from './WorkingLine'

/** Props for {@link Turn}. */
export interface TurnProps {
  /** The shaped turn. */
  readonly turn: TurnModel
  /** Whether the folded part is open. */
  readonly open: boolean
  /** Whether this turn follows another turn (the design's separator). */
  readonly sep?: boolean
  /** Toggle the fold. */
  readonly onToggle: () => void
}

/**
 * Render one turn.
 *
 * The panel's own rule (2026-10-09): a turn is the final answer, and everything else the
 * model produced — reasoning, tool calls and results, narration — is packed into the
 * single `已完成` fold. There is no second disclosure and no tool-specific one.
 *
 * @param props - the turn and its fold state.
 * @returns the turn article.
 */
export function Turn({ turn, open, onToggle, sep = false }: TurnProps) {
  const raw = turn.answer?.blocks.flatMap(block => (block.kind === 'text' ? [block.text] : [])).join('\n\n') ?? ''
  return (
    <article className={sep ? 'q-turn q-turn-sep' : 'q-turn'}>
      <div className="q-question">{`你 · ${turn.question}`}</div>
      {(turn.working.length > 0 || turn.inProgress) && (
        <>
          <Disclosure open={open} working={turn.inProgress} onToggle={onToggle} />
          <div className={open ? 'q-working q-open' : 'q-working'}>
            {turn.working.map((line, index) => (
              <WorkingLine key={index} line={line} />
            ))}
          </div>
        </>
      )}
      {turn.answer !== null && (
        <>
          <AnswerBody markdown={raw} />
          <AnswerBar working={turn.inProgress} markdown={raw} />
        </>
      )}
    </article>
  )
}

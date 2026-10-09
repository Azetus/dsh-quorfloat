// A question the model asked, which this build reports but cannot answer yet.
//
// The answer shape upstream is a list of selected option *labels* plus optional custom
// text, so it needs option widgets this panel does not have (P4). Until then the card is
// honest about where to answer, and the capability list omits `question`.

import { api } from '../../api'
import type { Interaction } from '../../lib/state'
import { Icon } from '../common/Icon'

/** Props for {@link QuestionCard}. */
export interface QuestionCardProps {
  /** The pending question. */
  readonly interaction: Interaction
}

/**
 * Render the question notice.
 *
 * @param props - the interaction.
 * @returns the card.
 */
export function QuestionCard({ interaction }: QuestionCardProps) {
  return (
    <div className="q-card">
      <div className="q-card-title">
        <Icon name="message-square" />
        <strong>需要你的回答</strong>
      </div>
      {interaction.questions.map(([header, text], index) => (
        <div className="q-card-reason" key={index}>{header !== null ? `${header}：${text}` : text}</div>
      ))}
      <div className="q-card-state">这个问题请在 Harness 窗口回答</div>
      <div className="q-card-actions">
        <button type="button" onClick={() => { void api.dismissInteraction(interaction.id) }}>
          知道了
        </button>
      </div>
    </div>
  )
}

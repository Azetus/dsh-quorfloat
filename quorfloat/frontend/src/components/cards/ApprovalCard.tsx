// An approval card: the tool, its reason, and the two verdicts.

import { api } from '../../api'
import type { Interaction } from '../../lib/state'
import { Icon } from '../common/Icon'

/** Props for {@link ApprovalCard}. */
export interface ApprovalCardProps {
  /** The pending (or settled) approval. */
  readonly interaction: Interaction
}

/**
 * Render an approval.
 *
 * Pending: allow once / reject. Settled: the verdict, and a dismiss for the card. A
 * refused card carries the host's own reason — an approval that could not be applied is
 * not the same as one that was declined.
 *
 * @param props - the interaction.
 * @returns the card.
 */
export function ApprovalCard({ interaction }: ApprovalCardProps) {
  return (
    <div className="q-card">
      <div className="q-card-title">
        <Icon name="shield-check" />
        <strong>需要批准</strong>
      </div>
      {interaction.toolName !== null && (
        <div className="q-card-title">{`工具 · ${interaction.toolName}`}</div>
      )}
      {interaction.detail !== null && <div className="q-card-reason">{interaction.detail}</div>}
      <div className={interaction.state === 'refused' ? 'q-card-state q-refused' : 'q-card-state'}>
        {interaction.state === 'submitting' && '提交中…'}
        {interaction.state === 'applied' && (interaction.verdict === 'allowed-once' ? '已允许一次' : '已拒绝')}
        {interaction.state === 'refused' && (interaction.refusalReason ?? '请求已失效')}
      </div>
      <div className="q-card-actions">
        {interaction.state === 'pending' ? (
          <>
            <button type="button" className="q-allow" onClick={() => { void api.answerApproval(interaction.id, true) }}>
              允许一次
            </button>
            <button type="button" onClick={() => { void api.answerApproval(interaction.id, false) }}>
              拒绝
            </button>
          </>
        ) : (
          <button type="button" onClick={() => { void api.dismissInteraction(interaction.id) }}>
            关闭
          </button>
        )}
      </div>
    </div>
  )
}

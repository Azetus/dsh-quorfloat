// An approval card: the tool, its reason, and the two verdicts.

import { api } from '../../api'
import { useT } from '../../hooks/useI18n'
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
  const t = useT()
  return (
    <div className="q-card">
      <div className="q-card-title">
        <Icon name="shield-check" />
        <strong>{t('approval.title')}</strong>
      </div>
      {interaction.toolName !== null && (
        <div className="q-card-title">{t('approval.tool', { tool: interaction.toolName })}</div>
      )}
      {interaction.detail !== null && <div className="q-card-reason">{interaction.detail}</div>}
      <div className={interaction.state === 'refused' ? 'q-card-state q-refused' : 'q-card-state'}>
        {interaction.state === 'submitting' && t('common.submitting')}
        {interaction.state === 'applied' && (interaction.verdict === 'allowed-once' ? t('approval.allowedOnce') : t('approval.rejected'))}
        {interaction.state === 'refused' && (interaction.refusalReason ?? t('common.requestExpired'))}
      </div>
      <div className="q-card-actions">
        {interaction.state === 'pending' ? (
          <>
            <button type="button" className="q-allow" onClick={() => { void api.answerApproval(interaction.id, true) }}>
              {t('approval.allowOnce')}
            </button>
            <button type="button" onClick={() => { void api.answerApproval(interaction.id, false) }}>
              {t('approval.reject')}
            </button>
          </>
        ) : (
          <button type="button" onClick={() => { void api.dismissInteraction(interaction.id) }}>
            {t('common.close')}
          </button>
        )}
      </div>
    </div>
  )
}

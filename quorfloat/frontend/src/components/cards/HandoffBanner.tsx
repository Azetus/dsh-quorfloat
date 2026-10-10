// The banner that says an interaction went to the Harness window instead of here.

import { api } from '../../api'
import { useT } from '../../hooks/useI18n'
import type { Snapshot } from '../../lib/state'

/** Props for {@link HandoffBanner}. */
export interface HandoffBannerProps {
  /** The handoff notice. */
  readonly handoff: NonNullable<Snapshot['handoff']>
}

/**
 * Render the handoff banner.
 *
 * The wording depends on whether any Harness surface is still active: "please switch to
 * the Harness window" is only sayable when there *is* one.
 *
 * @param props - the notice.
 * @returns the banner.
 */
export function HandoffBanner({ handoff }: HandoffBannerProps) {
  const t = useT()
  const hasSurface = handoff.surfaces.length > 0
  const text = handoff.kind === 'approval'
    ? (hasSurface ? t('handoff.approvalHandled') : t('handoff.approvalNoSurface'))
    : (hasSurface ? t('handoff.questionHandled') : t('handoff.questionNoSurface'))
  return (
    <div className="q-handoff">
      <span>{text}</span>
      <button type="button" onClick={() => { void api.dismissHandoff() }}>{t('common.gotIt')}</button>
    </div>
  )
}

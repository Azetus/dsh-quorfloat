// The banner that says an interaction went to the Harness window instead of here.

import { api } from '../api'
import type { Snapshot } from '../lib/state'

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
  const hasSurface = handoff.surfaces.length > 0
  const text = handoff.kind === 'approval'
    ? (hasSurface ? '审批已转交 Harness 窗口处理' : '当前没有可以处理它的 Harness 窗口')
    : (hasSurface ? '这个问题已转交 Harness 窗口回答' : '当前没有可以回答它的 Harness 窗口')
  return (
    <div className="q-handoff">
      <span>{text}</span>
      <button type="button" onClick={() => { void api.dismissHandoff() }}>知道了</button>
    </div>
  )
}

/**
 * Presence tracking and the approval-authority verdict.
 *
 * The platform facts this file implements:
 *
 * - `document.visibilityState` alone cannot answer "is the user looking at
 *   Harness?". On macOS a fully occluded window reports `hidden`, while on
 *   Windows the same situation reports `visible` but unfocused. Only the pair
 *   `visible AND hasFocus()` means "the user is looking at it", and that pair
 *   gives the same answer on both platforms.
 * - A surface therefore reports **both** facts, and the verdict follows:
 *   if nobody is looking at Harness, the panel decides; if nobody is looking at
 *   either surface, the request fails closed immediately instead of hanging.
 *
 * Everything here is pure state and pure functions. The caller supplies "now",
 * so expiry is testable without waiting for wall-clock time.
 */

/** A surface of the Harness user interface that can report its presence. */
export type PresenceSurface = 'desktop' | 'web'

/** What one surface last reported about itself. */
export interface PresenceReport {
  /** The page is composited (macOS: not occluded and on the current space). */
  readonly visible: boolean
  /** The window is the keyboard target, i.e. the user is looking at it. */
  readonly focused: boolean
  /** Caller-local ordering; a stale or reordered report is rejected. */
  readonly seq: number
  /** Caller-local time of the report. */
  readonly at: number
}

/** Presence of every surface, plus the panel's own visibility. */
export interface AuthorityInputs {
  readonly desktop: PresenceReport | undefined
  readonly web: PresenceReport | undefined
  /** The panel reported itself visible over the protocol. */
  readonly panelVisible: boolean
  /** Reports older than this are treated as "not looking". */
  readonly maxAgeMs: number
  readonly now: number
}

/** Who decides the next interaction. */
export type Authority = 'harness' | 'panel' | 'none'

/** The verdict, with the reason that produced it (for diagnostics and logs). */
export interface AuthorityVerdict {
  readonly authority: Authority
  /**
   * `harness-visible` — a surface is visible and focused.
   * `harness-open-but-idle` — a surface is visible but unfocused, so the panel
   *   takes over and should tell the user to switch to the Harness window.
   * `harness-not-visible` — no surface is visible; the panel takes over silently.
   * `nobody-looking` — neither surface can answer; fail closed.
   */
  readonly reason: 'harness-visible' | 'harness-open-but-idle' | 'harness-not-visible' | 'nobody-looking'
  /** Surfaces whose reports were fresh, for diagnostics. */
  readonly fresh: readonly PresenceSurface[]
}

/** Why a report was rejected, or `undefined` when it was accepted. */
export type ReportRejection = 'stale-sequence' | 'not-newer'

/**
 * Track the reported presence of every surface.
 *
 * Ordering is enforced per surface: a report whose `seq` is not greater than the
 * last accepted one is dropped, because reports can be delivered out of order
 * after a reconnect or a sleep/wake cycle, and an old "hidden" arriving after a
 * new "visible" would otherwise make the plugin take over an approval the user
 * was in fact looking at.
 */
export class PresenceTracker {
  readonly #reports = new Map<PresenceSurface, PresenceReport>()

  /**
   * Record one surface's report.
   *
   * @param surface - which surface is reporting.
   * @param report - its current visible/focused pair, sequence, and time.
   * @returns `undefined` when accepted, otherwise why it was rejected.
   */
  report(surface: PresenceSurface, report: PresenceReport): ReportRejection | undefined {
    const previous = this.#reports.get(surface)
    if (previous !== undefined && report.seq <= previous.seq) return 'stale-sequence'
    if (previous !== undefined && report.at < previous.at) return 'not-newer'
    this.#reports.set(surface, report)
    return undefined
  }

  /** Forget one surface, e.g. when its page unloads. */
  forget(surface: PresenceSurface): void {
    this.#reports.delete(surface)
  }

  /** Forget every surface, e.g. when the plugin restarts its client half. */
  forgetAll(): void {
    this.#reports.clear()
  }

  /** Last accepted report for one surface, fresh or not; for diagnostics. */
  raw(surface: PresenceSurface): PresenceReport | undefined {
    return this.#reports.get(surface)
  }

  /**
   * Reports that are still fresh.
   *
   * @param maxAgeMs - how long a report stays valid.
   * @param now - the caller's clock.
   * @returns the surfaces with a fresh report.
   */
  fresh(maxAgeMs: number, now: number): readonly PresenceSurface[] {
    const surfaces: PresenceSurface[] = []
    for (const [surface, report] of this.#reports) {
      if (now - report.at <= maxAgeMs) surfaces.push(surface)
    }
    return surfaces
  }
}

/**
 * Decide who answers the next interaction.
 *
 * @param inputs - both surfaces' reports, the panel's visibility, and the clock.
 * @returns the authority and the reason for it.
 */
export function evaluateAuthority(inputs: AuthorityInputs): AuthorityVerdict {
  const fresh: PresenceSurface[] = []
  let anyVisible = false
  let anyVisibleFocused = false
  for (const surface of ['desktop', 'web'] as const) {
    const report = inputs[surface]
    if (report === undefined) continue
    if (inputs.now - report.at > inputs.maxAgeMs) continue
    fresh.push(surface)
    if (!report.visible) continue
    anyVisible = true
    if (report.focused) anyVisibleFocused = true
  }

  if (anyVisibleFocused) {
    // The user is looking at a Harness window: it owns the answer, and the panel
    // must not claim (claiming would hide the request from the window they see).
    return { authority: 'harness', reason: 'harness-visible', fresh }
  }

  if (inputs.panelVisible) {
    // The panel is the surface the user can see. When a Harness window is open
    // but unfocused, the panel also has to say so — that user is one keystroke
    // away from the approval and would otherwise wonder where it went.
    return {
      authority: 'panel',
      reason: anyVisible ? 'harness-open-but-idle' : 'harness-not-visible',
      fresh,
    }
  }

  // Neither surface can answer. Claiming here would hold the turn until the
  // deadline for no benefit, so the caller must fail closed instead.
  return { authority: 'none', reason: 'nobody-looking', fresh }
}

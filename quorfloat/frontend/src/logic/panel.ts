// The panel's own box: how tall it wants to be, and how a swap moves it there.
//
// Framework-neutral on purpose: this is the policy that decides the panel's box, and
// it is the same whether React or anything else paints. The DOM half
// (`measureAndReport`'s reads and writes) lives in `hooks/usePanelDynamics.ts`.

/** The design's shadow room: the native window is the panel plus this. Mirrors
 *  `quorfloat/src/app/height.rs` (`SHADOW_TOP + SHADOW_BOTTOM`). */
export const SHADOW_ROOM = 60

/** The class that plays the entrance; its keyframes live in `tokens.css`. */
export const SWAP_CLASS = 'q-swap'

/** Extra room after a transition's own duration before giving up on its event. */
export const SETTLE_MARGIN_MS = 150

/** What one paint showed, so the next one can tell a switch from a repaint. */
export interface Painted {
  /** Which page was up. */
  readonly page: 'main' | 'settings'
  /** The conversation on screen, or null in new-conversation state. */
  readonly session: string | null
}

/**
 * Which content this paint swaps in.
 *
 * The snapshots arrive on every transcript event while a turn streams, so the fade
 * cannot be tied to "a render happened" — it has to be tied to the content moving. A
 * page change outranks a conversation change: one fade can only say one thing, and the
 * settings page is the bigger move.
 *
 * @param previous - what the last paint showed, or null before the first one.
 * @param next - what this paint is about to show.
 * @returns the content to animate, or null when nothing moved.
 */
export function swapFor(previous: Painted | null, next: Painted): 'page' | 'session' | null {
  if (previous === null) return null
  if (previous.page !== next.page) return 'page'
  if (previous.session !== next.session) return 'session'
  return null
}

/**
 * Deciding half of the panel's height animation: what to report, and whether the box
 * has to be pinned for a transition.
 *
 * The panel is content-sized, so its height follows the DOM instantly — a page or a
 * conversation swap therefore *snaps* unless it is pinned between two explicit heights
 * (`height: auto` is not interpolable). Two rules come from the shell's side of the
 * contract (`quorfloat/src/app/height.rs`):
 *
 * - **growing** reports the new height at once: the shell makes room first and the panel
 *   reveals itself into it;
 * - **shrinking** keeps reporting the room it still occupies until the collapse has
 *   finished — the shell shrinks the native window *immediately*, and a window that
 *   shrank first would trim the panel instead of letting it be seen to close.
 *
 * @param from - the panel height the swap started at, or null when this is not a swap.
 * @param desired - the panel height the content now wants.
 * @param animate - whether the panel may animate at all: not under reduceMotion, and not
 *  while it is on its way out.
 * @returns whether to pin the box for the transition, and the height to report.
 */
export function heightPlan(
  { from, desired, animate }:
  { from: number | null; desired: number; animate: boolean },
): { pin: boolean; report: number } {
  if (from === null || !animate || Math.abs(from - desired) < 1) {
    return { pin: false, report: desired }
  }
  return { pin: true, report: Math.max(from, desired) }
}

/**
 * The duration the stylesheet gives one token, in milliseconds.
 *
 * The panel's timings live in `tokens.css`; this reads them so a deadline cannot drift
 * from the transition it is a deadline for.
 *
 * @param name - the custom property, e.g. `--q-height`.
 * @param fallback - what to use when the token cannot be read (no stylesheet, as in a
 *  test page).
 * @returns the duration in milliseconds.
 */
export function tokenMs(name: string, fallback: number): number {
  const raw = getComputedStyle(document.documentElement).getPropertyValue(name).trim()
  const value = raw.endsWith('ms') ? Number.parseFloat(raw) : Number.parseFloat(raw) * 1000
  return Number.isFinite(value) && value > 0 ? value : fallback
}

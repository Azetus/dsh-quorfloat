// The panel's box: measure it, tell the shell, and carry a swap's height change.
//
// The DOM half of `lib/panel.ts`'s policy. It runs after every commit, because every
// snapshot can change the panel's height; the *decision* to animate is `swapFor` (did the
// page or the conversation move?) and `heightPlan` (pin the box, and what to report).
//
// The callbacks below are deliberately stable (they read the latest snapshot and paint
// through refs): the transition and resize listeners must stay subscribed once, and a
// stale closure here would measure the wrong pane.

import { useCallback, useEffect, useLayoutEffect, useRef, type RefObject } from 'react'
import { api } from '../api'
import {
  heightPlan, SETTLE_MARGIN_MS, SWAP_CLASS, swapFor, tokenMs, type Painted,
} from '../lib/panel'
import type { Snapshot } from '../lib/state'

/**
 * Keep the panel's height coordinated with the shell, and animate a swap.
 *
 * @param panel - the panel element.
 * @param state - the snapshot this commit renders.
 * @param arrived - what this commit puts on screen (page and conversation).
 * @param available - the room the native window offers, which caps the panel below the
 *  product limit when the window is short.
 */
export function usePanelDynamics(
  panel: RefObject<HTMLElement | null>,
  state: Snapshot,
  arrived: Painted,
  available: number,
): { remeasure: () => void } {
  /** The height the panel is pinned to while a swap's transition runs, or null. */
  const pinned = useRef<number | null>(null)
  /** The height the panel was showing at the end of the last commit. */
  const shown = useRef<number | null>(null)
  /** What the last commit put on screen. */
  const painted = useRef<Painted | null>(null)
  const settleTimer = useRef<number | undefined>(undefined)
  const stateRef = useRef(state)
  stateRef.current = state

  const settleNow = useCallback((): void => {
    if (settleTimer.current !== undefined) {
      window.clearTimeout(settleTimer.current)
      settleTimer.current = undefined
    }
    const element = panel.current
    if (element === null || pinned.current === null) return
    pinned.current = null
    element.style.height = ''
    // One fresh report — which is also where a *shrink* finally tells the shell it may
    // shrink the window.
    measureRef.current(null)
  }, [panel])

  /** Give the height transition an end even when its own event never arrives. */
  const settleLater = useCallback((): void => {
    if (settleTimer.current !== undefined) window.clearTimeout(settleTimer.current)
    settleTimer.current = window.setTimeout(
      () => { settleNow() },
      tokenMs('--q-height', 220) + SETTLE_MARGIN_MS,
    )
  }, [settleNow])

  const measure = useCallback((from: number | null): void => {
    const element = panel.current
    if (element === null) return
    const settings = stateRef.current.settings
    // While a swap's transition runs the panel carries an explicit height. It is lifted
    // for the read below and put straight back: the specified value never changes between
    // two style recalcs, so the transition is not restarted, and nothing is painted in
    // between, so nothing flashes.
    const running = pinned.current
    if (running !== null) element.style.height = ''
    // The probe grows the panel by a pixel so a capped panel can still say what it wants. For a
    // reader at the very bottom that pixel comes out of the scrollable extent: the browser
    // clamps the offset to the new maximum, and nothing gives it back — because a child's
    // layout effect (the thread's own pin) runs *before* this one, the clamp lands after the
    // pin and survives. The result was a one-pixel up-and-down on every keystroke, visible only
    // at the bottom, where the offset is at the maximum (2026-10-09). Put the reader back
    // exactly where they were; where they should be is the pin's decision, not this one's.
    const thread = element.querySelector<HTMLElement>('#q-thread-scroll')
    const keptScrollTop = thread?.scrollTop ?? null
    element.style.maxHeight = `${settings.maxHeight + 1}px`
    const natural = element.offsetHeight
    const desired = Math.max(1, Math.min(natural, settings.maxHeight))
    // Present at the smaller of the product cap and the room the native window provides:
    // content reveals itself once the shell has made room. `available` is the same number
    // the panel is styled with, so the probe below measures the real thing.
    element.style.maxHeight = `${Math.min(settings.maxHeight, available)}px`
    if (thread !== null && keptScrollTop !== null) thread.scrollTop = keptScrollTop

    // A panel on its way out has nobody watching it, and a transition left running across
    // the native hide is exactly the one whose end event never arrives (2026-10-09: the
    // new-session switch was pinned, the panel was hidden 90ms later, and the panel stayed
    // at the old height). A panel that is leaving goes straight to the new height.
    const leaving = element.classList.contains('q-away')
    const plan = heightPlan({
      from,
      desired,
      animate: !settings.reduceMotion && !leaving,
    })
    if (plan.pin && from !== null) {
      element.style.height = `${from}px`
      void element.offsetHeight
      element.style.height = `${desired}px`
      pinned.current = desired
      settleLater()
    } else if (running !== null) {
      // No new swap, and the panel is still mid-collapse: put its own pin back.
      element.style.height = `${running}px`
    }
    shown.current = desired
    void api.reportContentHeight(Math.round(plan.report))
  }, [panel, settleLater, available])
  const measureRef = useRef(measure)
  measureRef.current = measure

  // The transition's own end is the fast path; the deadline armed with the pin is what
  // makes settling a guarantee.
  useEffect(() => {
    const element = panel.current
    if (element === null) return
    const onEnd = (event: TransitionEvent): void => {
      if (event.propertyName === 'height') settleNow()
    }
    element.addEventListener('transitionend', onEnd)
    element.addEventListener('transitioncancel', onEnd)
    return () => {
      element.removeEventListener('transitionend', onEnd)
      element.removeEventListener('transitioncancel', onEnd)
    }
  }, [panel, settleNow])

  // The swap class *is* the animation: take it off when the animation is over, so that
  // showing an element later cannot replay it without a swap.
  useEffect(() => {
    const element = panel.current
    if (element === null) return
    const cleanups: (() => void)[] = []
    for (const selector of ['#q-settings', '#q-main', '#q-thread-scroll', '#q-cards']) {
      const target = element.querySelector<HTMLElement>(selector)
      if (target === null) continue
      const onEnd = (event: AnimationEvent): void => {
        if (event.animationName === 'q-swap-in') target.classList.remove(SWAP_CLASS)
      }
      target.addEventListener('animationend', onEnd)
      cleanups.push(() => { target.removeEventListener('animationend', onEnd) })
    }
    return () => { for (const cleanup of cleanups) cleanup() }
  }, [panel])

  /** Play the entrance on the content a swap brought in. */
  const playSwap = useCallback((where: 'page' | 'session'): void => {
    const element = panel.current
    if (element === null) return
    const settings = element.querySelector('#q-settings')
    const main = element.querySelector('#q-main')
    const scroll = element.querySelector('#q-thread-scroll')
    const cards = element.querySelector('#q-cards')
    const arriving = where === 'page'
      ? [arrived.page === 'settings' ? settings : main]
      : [scroll, cards]
    for (const candidate of [settings, main, scroll, cards]) candidate?.classList.remove(SWAP_CLASS)
    // The class has to leave and come back for a second swap to replay it, and the style
    // change has to be flushed before it does.
    void (arriving[0] as HTMLElement | null)?.offsetWidth
    for (const target of arriving) target?.classList.add(SWAP_CLASS)
  }, [panel, arrived.page])

  useLayoutEffect(() => {
    const swap = swapFor(painted.current, arrived)
    painted.current = arrived
    // The height a swap's transition starts from is the one the panel was *showing*: at
    // this point the DOM already holds the new content, so the old height only exists as
    // the memory of the last commit.
    const from = swap === null ? null : shown.current
    if (swap !== null) playSwap(swap)
    measure(from)
  })

  // The window's own size can change under us (the shell resizing, a display change):
  // remeasure on the way in, exactly as the old renderer did.
  useEffect(() => {
    const onResize = (): void => { measure(null) }
    window.addEventListener('resize', onResize)
    return () => { window.removeEventListener('resize', onResize) }
  }, [measure])

  // The font finishing loading moves every metric in the panel: the first measurement can
  // land before it, and the height it reported would then belong to the fallback face.
  useEffect(() => {
    let live = true
    void document.fonts.ready.then(() => { if (live) measure(null) })
    return () => { live = false }
  }, [measure])

  // The composer grows as the user types, without a snapshot to render from: it asks for
  // a remeasure instead of forcing a tree re-render per keystroke.
  const remeasure = useCallback((): void => { measure(null) }, [measure])
  return { remeasure }
}

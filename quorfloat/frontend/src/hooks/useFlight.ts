// The panel's fade-out flight: the `.q-away` class, the request to hide the native
// window, and the guards that keep a quick "hide then show" from closing a reopened panel.
//
// Ported from the old `main.ts` unchanged in behaviour — including the marker lines, which
// are the only observable record of a hide that the shell swallowed.

import { useCallback, useEffect, useRef, useState, type RefObject } from 'react'
import { listen } from '@tauri-apps/api/event'
import { api } from '../api'
import { hidesOnBlur } from '../lib/settings'
import type { Snapshot } from '../lib/state'
import { hotkeyAction, stepVisibility, type VisibilityState } from '../lib/visibility'

/** What the panel's visibility flight exposes to the tree. */
export interface Flight {
  /** Whether the panel is fading away (or already faded) — the `.q-away` class. */
  readonly hiding: boolean
  /** Ask the panel to fade out and hide. */
  readonly requestHide: () => void
  /** Bring the panel back (and cancel any fade in flight). */
  readonly requestShow: () => void
}

/** How long a hide may wait for the shell's acknowledgement before the guards give up:
 *  the flags are guards, not the truth, and a lost acknowledgement must not leave the
 *  panel invisible and unresponsive. */
const ACK_TIMEOUT_MS = 2000

/** How long a fade may take before the fallback hides anyway (the CSS transition is
 *  180ms; this is the "the transition never ran" path, e.g. reduced motion). */
const FADE_FALLBACK_MS = 300

/**
 * Own the panel's show/hide flight.
 *
 * @param panel - the panel element the fade runs on.
 * @param state - the latest snapshot, or null before the first one.
 * @param onShown - called when the panel comes back (focus the composer).
 * @returns the hiding flag and the two requests.
 */
export function useFlight(
  panel: RefObject<HTMLElement | null>,
  state: Snapshot | null,
  onShown: () => void,
): Flight {
  // The markup's own starting state is away: the panel appears when the shell first
  // says it is visible.
  const [hiding, setHiding] = useState(true)
  // The machine itself is not render output: only `hiding` is, and it mirrors the flag.
  const flight = useRef<VisibilityState>({ visible: false, hiding: false, hideSent: false })
  const generation = useRef(0)
  const ackTimer = useRef<number | undefined>(undefined)
  const stateRef = useRef<Snapshot | null>(null)
  const onShownRef = useRef(onShown)
  onShownRef.current = onShown

  const clearAck = useCallback((): void => {
    if (ackTimer.current !== undefined) {
      window.clearTimeout(ackTimer.current)
      ackTimer.current = undefined
    }
  }, [])

  const armAck = useCallback((): void => {
    clearAck()
    ackTimer.current = window.setTimeout(() => {
      if (!flight.current.hiding || !flight.current.hideSent) return
      flight.current = { visible: flight.current.visible, hiding: false, hideSent: false }
      void api.log('frontend hide ack timeout: resetting the hide state')
    }, ACK_TIMEOUT_MS)
  }, [clearAck])

  const requestShow = useCallback((): void => {
    generation.current += 1
    flight.current = stepVisibility(flight.current, 'show')
    clearAck()
    setHiding(false)
    void api.log('frontend show')
    onShownRef.current()
  }, [clearAck])

  const requestHide = useCallback((): void => {
    const visible = stateRef.current?.window.visible ?? false
    const next = stepVisibility({ ...flight.current, visible }, 'request-hide')
    if (next.hiding === flight.current.hiding) {
      // Either a hide is already in flight, or the shell says we are hidden — most
      // importantly the webview's own blur after a programmatic hide must not arm a
      // second fade-out.
      if (flight.current.hiding || !visible) {
        void api.log(`frontend hide ignored: hiding=${flight.current.hiding} visible=${visible}`)
      }
      return
    }
    flight.current = next
    const started = generation.current
    setHiding(true)
    void api.log(`frontend hide requested (generation ${started})`)

    const finish = (): void => {
      if (started !== generation.current) return
      flight.current = stepVisibility(flight.current, 'hide-sent')
      void api.setVisible(false)
      void api.log(`frontend hide finished (generation ${started})`)
      armAck()
    }

    // Reduced motion removes the transition, so there is nothing to wait for.
    if (stateRef.current?.settings.reduceMotion === true) {
      finish()
      return
    }
    const element = panel.current
    const onEnd = (event: TransitionEvent): void => {
      if (event.propertyName !== 'opacity' && event.propertyName !== 'transform') return
      element?.removeEventListener('transitionend', onEnd)
      if (started === generation.current) finish()
    }
    element?.addEventListener('transitionend', onEnd)
    window.setTimeout(() => {
      if (started === generation.current) finish()
    }, FADE_FALLBACK_MS)
  }, [armAck, panel])

  // The shell's snapshots drive the flight's facts: a becoming-visible snapshot is an
  // entrance, a hidden one acknowledges whatever hide was in flight.
  useEffect(() => {
    const wasVisible = stateRef.current?.window.visible ?? false
    stateRef.current = state
    if (state === null) return
    if (state.window.visible && !wasVisible) {
      generation.current += 1
      flight.current = stepVisibility(flight.current, 'show')
      clearAck()
      setHiding(false)
      void api.log('frontend show')
      onShownRef.current()
    }
    if (!state.window.visible) {
      flight.current = stepVisibility(flight.current, 'hidden-ack')
      generation.current += 1
      clearAck()
    }
  }, [state, clearAck])

  // The Rust hotkey fired while the panel was up: hide, cancel a fade, or recover a
  // request that already went out (the dispatcher applies hide-then-show in order).
  useEffect(() => {
    const pending = listen('quorfloat/hotkey-hide', () => {
      const action = hotkeyAction(flight.current)
      if (action === 'cancel-fade') {
        void api.log('frontend hotkey: fade cancelled, showing again')
        requestShow()
      } else if (action === 'recover-show') {
        void api.log('frontend hotkey: hide already sent, recovering with show')
        void api.setVisible(true)
        requestShow()
      } else {
        requestHide()
      }
    })
    return () => { void pending.then(unlisten => { unlisten() }) }
  }, [requestHide, requestShow])

  // Losing focus is the panel's ordinary way of going away — the setting decides whether it
  // does. The switch in the settings page is the *inversion* of this flag (the design labels
  // it by what the user gets: 失焦时保持展开), and `hidesOnBlur` is the one place that is
  // undone. A programmatic hide blurs the webview too; `requestHide`'s guards swallow that
  // (with its own marker), which is why this handler can stay this simple.
  useEffect(() => {
    const onBlur = (): void => {
      const state = stateRef.current
      if (state === null) return
      if (!hidesOnBlur(state.settings.hideOnBlur)) {
        void api.log('frontend blur: keeping the panel open (hideOnBlur=false)')
        return
      }
      void api.log('frontend blur: hiding (hideOnBlur=true)')
      requestHide()
    }
    window.addEventListener('blur', onBlur)
    return () => { window.removeEventListener('blur', onBlur) }
  }, [requestHide])

  return { hiding, requestHide, requestShow }
}

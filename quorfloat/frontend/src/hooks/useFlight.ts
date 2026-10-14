// The panel's fade-out flight: the `.q-away` class, the request to hide the native
// window, and the guards that keep a quick "hide then show" from closing a reopened panel.
//
// The marker lines are the only observable record of a hide that the shell swallowed.

import { useCallback, useEffect, useRef, useState, type RefObject } from 'react'
import { listen } from '@tauri-apps/api/event'
import { api } from '../api'
import { hidesOnBlur } from '../logic/settings'
import type { Snapshot } from '../logic/state'
import { hotkeyAction, stepVisibility, type VisibilityState } from '../logic/visibility'

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

/** How long after a press inside the panel a blur still counts as that press's own.
 *
 *  Starting the native drag (the top bar's `data-tauri-drag-region`, which Tauri answers
 *  with `start_dragging` on mousedown) hands the mouse to the system's move loop, and the
 *  webview loses focus as it does — a blur that says nothing about the user having gone
 *  somewhere else. The press is the only evidence that this is what happened, and it stays
 *  valid for about as long as a click can take. */
const DRAG_BLUR_WINDOW_MS = 500

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
  // When the panel was last pressed, for the guard below. A timestamp rather than a flag
  // because it has to expire: the press explains the *next* blur, not any blur ever after.
  const dragPressedAt = useRef<number | null>(null)
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
  //
  // Starting the native drag blurs the webview as well, and that blur is not the user
  // leaving. A press inside the panel is what tells the two apart, and it is armed where
  // the press happens rather than where the drag region is: Tauri decides that from its own
  // injected script, which is not ours to restate — and a press on one of the bar's buttons
  // blurs nothing, so arming widely can only excuse the drag that a press may have started.
  useEffect(() => {
    const onPress = (event: MouseEvent): void => {
      if (event.button !== 0) return
      dragPressedAt.current = performance.now()
    }
    const onBlur = (): void => {
      // This blur is the one the press can explain. Reading and clearing together keeps the
      // guard single-use: the webview blurs again when the panel is really hidden, and that
      // one belongs to whoever asked for the hide.
      const pressedAt = dragPressedAt.current
      dragPressedAt.current = null
      const state = stateRef.current
      if (state === null) return
      if (!hidesOnBlur(state.settings.hideOnBlur)) {
        void api.log('frontend blur: keeping the panel open (hideOnBlur=false)')
        return
      }
      if (pressedAt !== null && performance.now() - pressedAt <= DRAG_BLUR_WINDOW_MS) {
        // The move loop is the likelier reading, and the two failures are not equal: a hide
        // the user did not ask for takes the panel away mid-gesture, while staying up for a
        // press the user meant as a click away costs one more click.
        void api.log('frontend blur: ignoring drag startup (guard consumed)')
        return
      }
      void api.log('frontend blur: hiding (hideOnBlur=true)')
      requestHide()
    }
    // Capture, so the press is on the record before Tauri's own mousedown listener asks for
    // the drag; the system's move loop may swallow the matching mouseup, so nothing here
    // waits for one.
    window.addEventListener('mousedown', onPress, true)
    window.addEventListener('blur', onBlur)
    return () => {
      window.removeEventListener('mousedown', onPress, true)
      window.removeEventListener('blur', onBlur)
    }
  }, [requestHide])

  return { hiding, requestHide, requestShow }
}

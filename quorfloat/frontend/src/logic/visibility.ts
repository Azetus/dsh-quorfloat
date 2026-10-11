// The panel's visibility guards, as a tiny state machine.
//
// A quick "hide, then show" must not let the fade-out complete twice: the
// completion listener stays armed, and the *entrance* transition's transitionend
// must never be mistaken for the fade-out completing. The facts that make that
// unrepresentable here:
//
// - a hide request is ignored while one is already in flight (`hiding`) or
//   while the shell says the panel is hidden — a programmatic hide's own blur,
//   or a re-delivered hotkey-hide, can no longer arm a second fade-out;
// - `hiding` stays true until the shell *acknowledges* the hide (the state
//   event with `visible:false`) — the window between "fade finished" and
//   "native window really hidden" is part of the hide, not a gap. A separate
//   timeout (`ACK_TIMEOUT_MS` in `hooks/useFlight.ts`) resets the flags if the
//   acknowledgement never comes (hazard #1: the flags are guards, not the truth);
// - `hideSent` splits the flight in two phases for the **instant toggle**: a
//   hotkey during the fade cancels it (no request was ever sent, so Rust's
//   `visible` never disagrees); a hotkey after the request was sent recovers
//   with an explicit show (the dispatcher applies hide-then-show in order).

export interface VisibilityState {
  /** Whether the shell's last state says the window is visible. */
  visible: boolean
  /** Whether a hide is in flight: requested, not yet acknowledged. */
  hiding: boolean
  /** Whether the hide request has been sent to the shell (fade finished). */
  hideSent: boolean
}

export type VisibilityEvent = 'request-hide' | 'hide-sent' | 'show' | 'hidden-ack'

/**
 * One step of the visibility state machine.
 *
 * @param state - the current state.
 * @param event - what just happened.
 * @returns the next state.
 */
export function stepVisibility(state: VisibilityState, event: VisibilityEvent): VisibilityState {
  switch (event) {
    case 'request-hide':
      // Already hiding, or already hidden: nothing to start.
      if (state.hiding || !state.visible) return state
      return { visible: true, hiding: true, hideSent: false }
    case 'hide-sent':
      // The fade finished and the request went out.
      if (!state.hiding) return state
      return { visible: true, hiding: true, hideSent: true }
    case 'show':
      return { visible: true, hiding: false, hideSent: false }
    case 'hidden-ack':
      return { visible: false, hiding: false, hideSent: false }
  }
}

/** What a hotkey press means for the panel's visibility. */
export type HotkeyAction = 'hide' | 'cancel-fade' | 'recover-show'

/**
 * The hotkey's meaning, from the flight state.
 *
 * - idle → hide, the normal toggle-down;
 * - fading (no request sent yet) → cancel the fade and show again: the native
 *   window never left, so nothing on the Rust side needs correcting;
 * - request sent, acknowledgement pending → recover with an explicit show
 *   (the dispatcher applies the queued hide and this show in order).
 *
 * @param state - the flight state.
 * @returns what the hotkey should do.
 */
export function hotkeyAction(state: Pick<VisibilityState, 'hiding' | 'hideSent'>): HotkeyAction {
  if (!state.hiding) return 'hide'
  return state.hideSent ? 'recover-show' : 'cancel-fade'
}

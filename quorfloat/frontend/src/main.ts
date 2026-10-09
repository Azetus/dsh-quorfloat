// The page's wiring: the shell's snapshots in, the user's events out.
//
// - `quorfloat/state` — the bridge's snapshot; the panel re-renders from it.
// - `quorfloat/hotkey-hide` — the Rust hotkey fired while visible: the page
//   plays the exit transition and asks the shell to hide when it finishes.
// - every render measures the panel (bounded, clamped — see
//   `render.measureAndReport`) and reports it to the shell's height machine.
// - the 180ms fade-in/out is the design's `.q-away` transition, verbatim.

import { listen } from '@tauri-apps/api/event'
import { api } from './api'
import { chordFromEvent, validateChord } from './lib/hotkey'
import { hidesOnBlur } from './lib/settings'
import type { Snapshot } from './lib/state'
import { hotkeyAction, stepVisibility, type VisibilityState } from './lib/visibility'
import * as render from './render'

const windowEl = document.getElementById('q-window') as HTMLElement
const input = document.getElementById('q-input') as HTMLTextAreaElement

let lastSnapshot: Snapshot | null = null
// The flight state, driven exclusively by the machine in lib/visibility.ts.
// `hideGeneration` invalidates any fade-out wait that outlived its hide (a show
// bumps it, so an old transitionend or fallback timer can never close a
// reopened panel).
let flight: VisibilityState = { visible: false, hiding: false, hideSent: false }
let hideGeneration = 0
let ackTimer: number | undefined

/** How long a hide may wait for the shell's acknowledgement before the guards
 *  give up: the flags are guards, not the truth, and a lost acknowledgement
 *  must not leave the panel invisible and unresponsive (hazard #1). */
const ACK_TIMEOUT_MS = 2000

function armAckTimeout(): void {
  clearAckTimeout()
  ackTimer = window.setTimeout(() => {
    if (!flight.hiding || !flight.hideSent) return
    flight = { visible: flight.visible, hiding: false, hideSent: false }
    void api.log('frontend hide ack timeout: resetting the hide state')
  }, ACK_TIMEOUT_MS)
}

function clearAckTimeout(): void {
  if (ackTimer !== undefined) {
    window.clearTimeout(ackTimer)
    ackTimer = undefined
  }
}

/** Fade the panel away, then ask the shell to hide the native window. */
function hidePanel(): void {
  const visible = lastSnapshot?.window.visible ?? false
  const next = stepVisibility({ ...flight, visible }, 'request-hide')
  if (next.hiding === flight.hiding) {
    // Either a hide is already in flight, or the shell says we are hidden —
    // most importantly the webview's own blur after a programmatic hide must
    // not arm a second fade-out.
    if (flight.hiding || !visible) {
      void api.log(`frontend hide ignored: hiding=${flight.hiding} visible=${visible}`)
    }
    return
  }
  flight = next
  const generation = hideGeneration
  windowEl.classList.add('q-away')
  void api.log(`frontend hide requested (generation ${generation})`)
  // Reduced motion means no transition to wait for: hide immediately instead
  // of holding the window for the fallback timer. `hiding` stays true until
  // the shell acknowledges, so no blur or re-delivered hotkey can re-enter.
  if (lastSnapshot?.settings.reduceMotion === true) {
    flight = stepVisibility(flight, 'hide-sent')
    void api.setVisible(false)
    armAckTimeout()
    return
  }
  const finish = () => {
    if (generation !== hideGeneration) return
    windowEl.removeEventListener('transitionend', onEnd)
    flight = stepVisibility(flight, 'hide-sent')
    void api.setVisible(false)
    void api.log(`frontend hide finished (generation ${generation})`)
    armAckTimeout()
  }
  const onEnd = (event: TransitionEvent) => {
    const relevant = event.propertyName === 'opacity' || event.propertyName === 'transform'
    if (!relevant) return
    // A stale listener (its hide was overtaken by a show) removes itself on the
    // first transition it sees and never acts.
    windowEl.removeEventListener('transitionend', onEnd)
    if (generation === hideGeneration) finish()
  }
  windowEl.addEventListener('transitionend', onEnd)
  // The transitionend fallback: if the transition never runs (reduced motion
  // removes it), the hide still happens.
  window.setTimeout(() => {
    if (generation === hideGeneration && windowEl.classList.contains('q-away')) finish()
  }, 300)
}

/** Play the entrance transition. */
function showPanel(): void {
  hideGeneration += 1
  flight = stepVisibility(flight, 'show')
  clearAckTimeout()
  windowEl.classList.remove('q-away')
  void api.log('frontend show')
  render.focusComposer()
}

render.bindCloseHandler(hidePanel)
render.bindStatic()

// ── the shell's events ─────────────────────────────────────────────────────

void listen<Snapshot>('quorfloat/state', event => {
  const state = event.payload
  const wasVisible = lastSnapshot?.window.visible ?? false
  lastSnapshot = state
  render.renderState(state)
  if (state.window.visible && !wasVisible) showPanel()
  if (!state.window.visible) {
    // The hide was acknowledged: the flight is over, and nothing outstanding
    // may act afterwards.
    flight = stepVisibility(flight, 'hidden-ack')
    hideGeneration += 1
    clearAckTimeout()
  }
})

void listen('quorfloat/hotkey-hide', () => {
  const action = hotkeyAction(flight)
  if (action === 'cancel-fade') {
    // 反悔：fade 还没发请求，取消即可——Rust 侧的 visible 从头到尾没有变过。
    void api.log('frontend hotkey: fade cancelled, showing again')
    showPanel()
  } else if (action === 'recover-show') {
    // 请求已发出：补一个 show，dispatcher 按序执行 hide→show，最终可见。
    void api.log('frontend hotkey: hide already sent, recovering with show')
    void api.setVisible(true)
    showPanel()
  } else {
    hidePanel()
  }
})

// ── height reporting ───────────────────────────────────────────────────────

// The panel measures itself when it renders (bounded measure + clamped
// report — see render.measureAndReport); those two events re-measure because
// they change layout without a render: the native window resized (the room
// the cap allows changed), and the font finished loading (every metric moved).
window.addEventListener('resize', () => {
  if (lastSnapshot !== null) render.measureAndReport(lastSnapshot)
})
void document.fonts.ready.then(() => {
  if (lastSnapshot !== null) render.measureAndReport(lastSnapshot)
})

// ── keyboard ───────────────────────────────────────────────────────────────

document.addEventListener('keydown', event => {
  // IME composition is the user's, not the panel's: Enter during composition
  // confirms a candidate, it does not send.
  if (event.isComposing || event.keyCode === 229) return

  if (render.isRecording()) {
    event.preventDefault()
    const chord = chordFromEvent(event)
    if (chord === null) return
    const verdict = validateChord(chord)
    if (!verdict.ok) {
      render.setTransientError(verdict.reason)
      if (lastSnapshot !== null) render.renderState(lastSnapshot)
      return
    }
    render.setTransientError(null)
    render.setRecording(false)
    void api.setPreferences({ hotkey: chord })
    return
  }

  if (event.key === 'Escape') {
    event.preventDefault()
    if (render.hasOpenMenu()) {
      render.closeMenu()
      return
    }
    if (render.isSettingsPage() && lastSnapshot !== null) {
      render.backFromSettings(lastSnapshot)
      return
    }
    hidePanel()
    return
  }

  if (event.key === 'Enter' && !event.shiftKey && event.target === input) {
    event.preventDefault()
    if (lastSnapshot !== null) render.submitDraft(lastSnapshot)
  }
})

window.addEventListener('blur', () => {
  if (lastSnapshot === null) return
  if (!hidesOnBlur(lastSnapshot.settings.hideOnBlur)) return
  // On the record: a blur after a programmatic hide must read as "ignored",
  // not as a second hide.
  void api.log(`frontend blur: hiding=${flight.hiding} visible=${lastSnapshot.window.visible}`)
  hidePanel()
})

// ── development bridge ─────────────────────────────────────────────────────

// When the page runs in a plain browser (no Tauri), `invoke` has nothing to
// reach; the log is a diagnostic for that case and nothing more.
if (!('__TAURI__' in window)) {
  void api.log('frontend running without the shell').catch(() => {})
}

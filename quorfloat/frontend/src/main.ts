// The page's wiring: the shell's snapshots in, the user's events out.
//
// - `quorfloat/state` — the bridge's snapshot; the panel re-renders from it.
// - `quorfloat/hotkey-hide` — the Rust hotkey fired while visible: the page
//   plays the exit transition and asks the shell to hide when it finishes.
// - a ResizeObserver reports the panel height, which the shell's height
//   machine coordinates with the native window.
// - the 180ms fade-in/out is the design's `.q-away` transition, verbatim.

import { listen } from '@tauri-apps/api/event'
import { api } from './api'
import { chordFromEvent, validateChord } from './lib/hotkey'
import type { Snapshot } from './lib/state'
import * as render from './render'

const windowEl = document.getElementById('q-window') as HTMLElement
const input = document.getElementById('q-input') as HTMLTextAreaElement

let lastSnapshot: Snapshot | null = null
let hiding = false

/** Fade the panel away, then ask the shell to hide the native window. */
function hidePanel(): void {
  if (hiding) return
  hiding = true
  windowEl.classList.add('q-away')
  const finish = () => {
    windowEl.removeEventListener('transitionend', onEnd)
    void api.setVisible(false)
    hiding = false
  }
  const onEnd = (event: TransitionEvent) => {
    if (event.propertyName === 'opacity' || event.propertyName === 'transform') finish()
  }
  windowEl.addEventListener('transitionend', onEnd)
  // The transitionend fallback: if the transition never runs (reduced motion
  // removes it), the hide still happens.
  window.setTimeout(() => {
    if (windowEl.classList.contains('q-away')) finish()
  }, 300)
}

/** Play the entrance transition. */
function showPanel(): void {
  windowEl.classList.remove('q-away')
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
})

void listen('quorfloat/hotkey-hide', () => {
  hidePanel()
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
  if (lastSnapshot !== null && !lastSnapshot.settings.hideOnBlur) hidePanel()
})

// ── development bridge ─────────────────────────────────────────────────────

// When the page runs in a plain browser (no Tauri), `invoke` has nothing to
// reach; the log is a diagnostic for that case and nothing more.
if (!('__TAURI__' in window)) {
  void api.log('frontend running without the shell').catch(() => {})
}

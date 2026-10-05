/**
 * Presence and authority tests.
 *
 * These pin the routing rules that were derived from measurement rather than
 * assumption (docs/prototype.md §18): `visibilityState` alone cannot answer "is
 * the user looking at Harness?", because a fully occluded window reports
 * `hidden` on macOS and `visible` on Windows. Only `visible AND hasFocus()` gives
 * the same answer on both, and the verdict follows from that pair.
 *
 * The failure this suite exists to prevent is a silent one: claiming an approval
 * for a window the user is actually looking at hides it from them, and claiming
 * when nobody can see any surface holds the turn for the whole deadline.
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'

import { loadModule } from './helpers.mjs'

const { PresenceTracker, evaluateAuthority } = await loadModule('harness/presence.js')

/** Build the inputs for one verdict. */
function inputs({ desktop, web, panelVisible = true, maxAgeMs = 30000, now = 1_000_000 } = {}) {
  return { desktop, web, panelVisible, maxAgeMs, now }
}

/** A presence report at a given time. */
function report({ visible, focused, at = 1_000_000, seq = 1 }) {
  return { visible, focused, seq, at }
}

test('a focused, visible Harness window keeps the authority', () => {
  const verdict = evaluateAuthority(inputs({ desktop: report({ visible: true, focused: true }) }))
  assert.equal(verdict.authority, 'harness')
  assert.equal(verdict.reason, 'harness-visible')
  assert.deepEqual(verdict.fresh, ['desktop'])
})

test('a visible but unfocused window hands over to the panel', () => {
  // The macOS/Windows divergence lands here: occluded reports either
  // `hidden` or `visible`+unfocused, and both must reach the same verdict.
  const verdict = evaluateAuthority(inputs({ desktop: report({ visible: true, focused: false }) }))
  assert.equal(verdict.authority, 'panel')
  assert.equal(verdict.reason, 'harness-open-but-idle', 'the panel must tell the user to switch windows')
})

test('an occluded window (hidden) hands over silently', () => {
  const verdict = evaluateAuthority(inputs({ desktop: report({ visible: false, focused: false }) }))
  assert.equal(verdict.authority, 'panel')
  assert.equal(verdict.reason, 'harness-not-visible')
})

test('both platforms agree once focus is part of the condition', () => {
  // Same physical situation, different visibility: macOS reports hidden,
  // Windows reports visible-but-unfocused. The verdict must be identical.
  const macos = evaluateAuthority(inputs({ desktop: report({ visible: false, focused: false }) }))
  const windows = evaluateAuthority(inputs({ desktop: report({ visible: true, focused: false }) }))
  assert.equal(macos.authority, windows.authority)
  assert.equal(macos.reason, 'harness-not-visible')
  assert.equal(windows.reason, 'harness-open-but-idle', 'the only difference is which hint the panel shows')
})

test('a stale report does not keep the authority with Harness', () => {
  // A page killed without reporting must not lock the plugin into deferring.
  const verdict = evaluateAuthority(
    inputs({ desktop: report({ visible: true, focused: true, at: 1_000_000 }), now: 1_000_000 + 30_001 }),
  )
  assert.equal(verdict.authority, 'panel')
  assert.deepEqual(verdict.fresh, [], 'an expired report is not fresh')
})

test('a report exactly at the age limit is still fresh', () => {
  const verdict = evaluateAuthority(
    inputs({ desktop: report({ visible: true, focused: true, at: 1_000_000 }), now: 1_000_000 + 30_000 }),
  )
  assert.equal(verdict.authority, 'harness')
})

test('the web surface can keep the authority on its own', () => {
  const verdict = evaluateAuthority(inputs({ web: report({ visible: true, focused: true }) }))
  assert.equal(verdict.authority, 'harness')
  assert.deepEqual(verdict.fresh, ['web'])
})

test('a focused web page wins even while a desktop window is idle', () => {
  // Two surfaces disagree; the one the user is looking at decides.
  const verdict = evaluateAuthority(
    inputs({
      desktop: report({ visible: true, focused: false, seq: 1 }),
      web: report({ visible: true, focused: true, seq: 2 }),
    }),
  )
  assert.equal(verdict.authority, 'harness')
  assert.deepEqual(verdict.fresh, ['desktop', 'web'])
})

test('no surface at all fails closed instead of claiming', () => {
  const verdict = evaluateAuthority(inputs({ panelVisible: false }))
  assert.equal(verdict.authority, 'none')
  assert.equal(verdict.reason, 'nobody-looking')
})

test('a hidden panel with a visible Harness window still defers to Harness', () => {
  const verdict = evaluateAuthority(
    inputs({ desktop: report({ visible: true, focused: true }), panelVisible: false }),
  )
  assert.equal(verdict.authority, 'harness')
})

test('an open but unfocused window with a hidden panel fails closed', () => {
  // Nobody can answer: the window is behind something and the panel is not shown.
  // Holding the turn would gain nothing, so the verdict must be `none`.
  const verdict = evaluateAuthority(
    inputs({ desktop: report({ visible: true, focused: false }), panelVisible: false }),
  )
  assert.equal(verdict.authority, 'none')
  assert.equal(verdict.reason, 'nobody-looking')
})

test('the tracker rejects a replayed sequence number', () => {
  const tracker = new PresenceTracker()
  assert.equal(tracker.report('web', report({ visible: true, focused: true, seq: 5 })), undefined)
  assert.equal(tracker.report('web', report({ visible: false, focused: false, seq: 5 })), 'stale-sequence')
  assert.equal(tracker.raw('web').visible, true, 'the older report did not overwrite the newer one')
})

test('the tracker rejects a report that travels backwards in time', () => {
  const tracker = new PresenceTracker()
  tracker.report('web', report({ visible: true, focused: true, seq: 2, at: 5000 }))
  assert.equal(tracker.report('web', report({ visible: false, focused: false, seq: 3, at: 4000 })), 'not-newer')
})

test('the tracker keeps surfaces independent', () => {
  const tracker = new PresenceTracker()
  tracker.report('desktop', report({ visible: true, focused: true, seq: 9 }))
  assert.equal(tracker.report('web', report({ visible: false, focused: false, seq: 1 })), undefined)
  assert.equal(tracker.raw('desktop').visible, true)
  assert.equal(tracker.raw('web').visible, false)
})

test('forgetting a surface makes it unrepresented rather than stale-but-trusted', () => {
  const tracker = new PresenceTracker()
  tracker.report('web', report({ visible: true, focused: true, seq: 1 }))
  tracker.forget('web')
  assert.equal(tracker.raw('web'), undefined)
  const verdict = evaluateAuthority(
    inputs({ web: tracker.raw('web'), panelVisible: false, now: 1_000_000 }),
  )
  assert.equal(verdict.authority, 'none')
})

test('a fresh report from one surface cannot vouch for another', () => {
  const tracker = new PresenceTracker()
  tracker.report('desktop', report({ visible: true, focused: true, seq: 1, at: 1_000_000 }))
  tracker.report('web', report({ visible: true, focused: true, seq: 1, at: 900_000 }))
  const verdict = evaluateAuthority(
    inputs({ desktop: tracker.raw('desktop'), web: tracker.raw('web'), now: 1_000_000 }),
  )
  assert.deepEqual(verdict.fresh, ['desktop'], 'the expired surface is excluded but does not poison the verdict')
  assert.equal(verdict.authority, 'harness')
})

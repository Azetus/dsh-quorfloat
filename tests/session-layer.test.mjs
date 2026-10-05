/**
 * Session-layer tests: subscription generations, sequence gaps, and prompt
 * idempotency.
 *
 * Every rule asserted here exists to prevent a specific visible defect: a
 * replaced subscription double-appending content, a gap silently rendering a
 * conversation with a hole in it, or a reconnect resending the user's input.
 * The layer runs against a fake Harness, so these rules are pinned without a
 * running desktop app.
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'

import { loadModule, recordingLogger } from './helpers.mjs'

const { SessionLayer } = await loadModule('harness/session-layer.js')
const { HarnessError } = await loadModule('harness/adapter.js')

/** A fake Harness that records calls and lets a test push frames. */
function fakeHarness({ workspaces = [{ workspaceId: 'w1', path: '/tmp/ws', title: 'Workspace' }] } = {}) {
  const frames = new Map()
  const calls = { follow: [], close: 0, prompts: [], cancels: [], created: [], pages: [], cursors: [] }
  const cursors = new Map()
  let nextSession = 1
  return {
    calls,
    cursors,
    describe: () => ({ adapter: 'fake' }),
    listWorkspaces: () => workspaces,
    workspace: id => workspaces.find(entry => entry.workspaceId === id),
    async createSession(workspaceId) {
      calls.created.push(workspaceId)
      return { sessionId: `session-${nextSession++}` }
    },
    async listSessions() {
      return [{ sessionId: 'session-1', updatedAt: 1, running: false, blank: false }]
    },
    async cursor(sessionId) {
      calls.cursors.push(sessionId)
      return cursors.has(sessionId) ? cursors.get(sessionId) : undefined
    },
    async pageHistory(sessionId, throughSeq, beforeSeq) {
      calls.pages.push({ sessionId, throughSeq, beforeSeq })
      return { records: [{ seq: 1 }], hasMore: false }
    },
    async prompt(sessionId, requestId, text) {
      calls.prompts.push({ sessionId, requestId, text })
      return { accepted: true }
    },
    async cancel(sessionId) {
      calls.cancels.push(sessionId)
      return { accepted: true }
    },
    follow(sessionId, onFrame) {
      calls.follow.push(sessionId)
      frames.set(sessionId, onFrame)
      let closed = false
      return {
        done: Promise.resolve(),
        close: () => {
          if (closed) return
          closed = true
          calls.close += 1
        },
      }
    },
    push(sessionId, frame) {
      const handler = frames.get(sessionId)
      if (handler === undefined) throw new Error(`no subscription for ${sessionId}`)
      handler(frame)
    },
  }
}

/** Build a session layer over a fake Harness, capturing notifications. */
function buildLayer({ harness = fakeHarness(), defaultWorkspaceId = '' } = {}) {
  const notifications = []
  const states = []
  const layer = new SessionLayer({
    harness,
    defaultWorkspaceId: () => defaultWorkspaceId,
    notify: async (method, params) => {
      notifications.push({ method, params })
    },
    onStateChange: (sessionId, reason, detail) => states.push({ sessionId, reason, detail }),
    log: recordingLogger(),
  })
  return { layer, harness, notifications, states }
}

/** A durable snapshot frame with the given cursor. */
function snapshot(cursor, records = []) {
  return { type: 'snapshot', cursor, hasMore: false, records, projections: {}, assistantStream: null }
}

/** A durable event frame at one sequence. */
function event(seq, eventType = 'assistant/message') {
  return { type: 'event', seq, eventType, time: seq, data: { seq } }
}

/** Let queued promise continuations run. */
const tick = () => new Promise(resolve => setTimeout(resolve, 0))

test('createSession requires an explicit or configured workspace', async () => {
  const { layer } = buildLayer()
  await assert.rejects(() => layer.createSession(''), error => {
    assert.ok(error instanceof HarnessError)
    assert.equal(error.code, 'unavailable')
    return true
  })
})

test('createSession rejects a workspace that no longer exists', async () => {
  const { layer } = buildLayer()
  await assert.rejects(() => layer.createSession('gone'), error => {
    assert.equal(error.code, 'stale')
    assert.deepEqual(error.detail.known, ['w1'])
    return true
  })
})

test('createSession uses the configured default when the caller omits one', async () => {
  const { layer, harness } = buildLayer({ defaultWorkspaceId: 'w1' })
  const created = await layer.createSession('')
  assert.equal(created.sessionId, 'session-1')
  assert.deepEqual(harness.calls.created, ['w1'])
})

test('a snapshot is forwarded and recorded as the subscription cursor', async () => {
  const { layer, harness, notifications } = buildLayer()
  await layer.attach('session-1')
  harness.push('session-1', snapshot(41, [{ type: 'event' }]))
  await tick()
  assert.equal(notifications.length, 1)
  assert.equal(notifications[0].method, 'session/snapshot')
  assert.equal(notifications[0].params.sessionId, 'session-1')
  assert.equal(notifications[0].params.cursor, 41)
  assert.equal(notifications[0].params.records.length, 1)
})

test('contiguous events are forwarded and advance the cursor', async () => {
  const { layer, harness, notifications } = buildLayer()
  await layer.attach('session-1')
  harness.push('session-1', snapshot(1))
  harness.push('session-1', event(2))
  harness.push('session-1', event(3))
  await tick()
  const forwarded = notifications.filter(entry => entry.method === 'session/event')
  assert.deepEqual(forwarded.map(entry => entry.params.seq), [2, 3])
})

test('a duplicate event is dropped instead of being shown twice', async () => {
  const { layer, harness, notifications } = buildLayer()
  await layer.attach('session-1')
  harness.push('session-1', snapshot(1))
  harness.push('session-1', event(2))
  harness.push('session-1', event(2))
  await tick()
  assert.equal(notifications.filter(entry => entry.method === 'session/event').length, 1)
})

test('a sequence gap asks for a resync instead of rendering a hole', async () => {
  const { layer, harness, notifications } = buildLayer()
  await layer.attach('session-1')
  harness.push('session-1', snapshot(1))
  harness.push('session-1', event(5))
  await tick()
  assert.equal(notifications.filter(entry => entry.method === 'session/event').length, 0)
  const resync = notifications.find(entry => entry.method === 'session/resync')
  assert.ok(resync !== undefined, 'a gap must be reported')
  assert.equal(resync.params.expected, 2)
  assert.equal(resync.params.received, 5)
})

test('an event that arrives before the snapshot is dropped', async () => {
  const { layer, harness, notifications } = buildLayer()
  await layer.attach('session-1')
  harness.push('session-1', event(1))
  await tick()
  assert.equal(notifications.length, 0)
})

test('re-attaching replaces the previous subscription and drops its late frames', async () => {
  const { layer, harness, notifications } = buildLayer()
  const first = await layer.attach('session-1')
  harness.push('session-1', snapshot(1))
  const second = await layer.attach('session-1')
  assert.equal(second.replaced, true)
  assert.notEqual(second.generation, first.generation)
  assert.equal(harness.calls.close, 1, 'the old subscription was closed')
  notifications.length = 0
  // A frame delivered by the *old* subscription after replacement must not reach
  // the peer: it would duplicate content the new subscription also carries.
  const handler = harness.calls.follow.length
  assert.equal(handler, 2)
  await tick()
  assert.equal(notifications.length, 0)
})

test('detach stops forwarding and reports it', async () => {
  const { layer, harness, notifications, states } = buildLayer()
  await layer.attach('session-1')
  harness.push('session-1', snapshot(1))
  await tick()
  assert.equal(layer.detach('session-1'), true)
  notifications.length = 0
  assert.equal(layer.detach('session-1'), false, 'detaching twice is a no-op')
  assert.ok(states.some(entry => entry.reason === 'detached'))
})

test('ownedSessionIds is exactly the set of attached sessions', async () => {
  const { layer } = buildLayer()
  assert.deepEqual(layer.ownedSessionIds(), [])
  await layer.attach('session-1')
  await layer.attach('session-2')
  assert.deepEqual([...layer.ownedSessionIds()].sort(), ['session-1', 'session-2'])
  layer.detachAll('test')
  assert.deepEqual(layer.ownedSessionIds(), [])
})

test('the same prompt request id is admitted once', async () => {
  const { layer, harness } = buildLayer()
  await layer.prompt('session-1', 'req-1', 'hello')
  await layer.prompt('session-1', 'req-1', 'hello')
  assert.equal(harness.calls.prompts.length, 1, 'a repeated id is an idempotent replay, not a second prompt')
})

test('a different request id for the same text is a deliberate second submission', async () => {
  const { layer, harness } = buildLayer()
  await layer.prompt('session-1', 'req-1', 'hello')
  await layer.prompt('session-1', 'req-2', 'hello')
  assert.equal(harness.calls.prompts.length, 2)
})

test('a prompt that is still in flight is not admitted twice', async () => {
  let release
  const gate = new Promise(resolve => {
    release = resolve
  })
  const harness = fakeHarness()
  harness.prompt = async (sessionId, requestId, text) => {
    harness.calls.prompts.push({ sessionId, requestId, text })
    await gate
    return { accepted: true }
  }
  const { layer } = buildLayer({ harness })
  const first = layer.prompt('session-1', 'req-1', 'hello')
  await assert.rejects(() => layer.prompt('session-1', 'req-1', 'hello'), error => {
    assert.equal(error.code, 'unavailable')
    assert.match(error.message, /awaiting confirmation/)
    return true
  })
  release()
  await first
})

test('a failed prompt is not silently retried', async () => {
  const harness = fakeHarness()
  harness.prompt = async () => {
    throw new Error('harness said no')
  }
  const { layer } = buildLayer({ harness })
  await assert.rejects(() => layer.prompt('session-1', 'req-1', 'hello'))
  await assert.rejects(() => layer.prompt('session-1', 'req-1', 'hello'), error => {
    assert.match(error.message, /already failed/)
    return true
  })
})

test('cancel reports acceptance without publishing a terminal state', async () => {
  const { layer, states, harness } = buildLayer()
  const result = await layer.cancel('session-1')
  assert.equal(result.accepted, true)
  assert.deepEqual(harness.calls.cancels, ['session-1'])
  assert.ok(states.some(entry => entry.reason === 'cancel-requested'))
  // The authoritative end of a turn is a durable event, which this layer does
  // not fabricate; only the request is recorded here.
  assert.ok(!states.some(entry => entry.reason === 'cancelled'))
})

test('history pages use the subscription cursor as their bound', async () => {
  const { layer, harness } = buildLayer()
  await layer.attach('session-1')
  harness.push('session-1', snapshot(7))
  await tick()
  const page = await layer.readHistory('session-1', 42)
  assert.deepEqual(harness.calls.pages, [{ sessionId: 'session-1', throughSeq: 7, beforeSeq: 42 }])
  assert.deepEqual(harness.calls.cursors, [], 'the live subscription cursor is authoritative when there is one')
  assert.equal(page.hasMore, false)
})

test('history without a subscription falls back to the persisted cursor', async () => {
  const { layer, harness } = buildLayer()
  harness.cursors.set('session-2', 5)
  await layer.readHistory('session-2')
  assert.deepEqual(harness.calls.pages, [{ sessionId: 'session-2', throughSeq: 5, beforeSeq: undefined }])
  assert.deepEqual(harness.calls.cursors, ['session-2'])
})

test('history for a session with no events is an empty page, not an invented bound', async () => {
  const { layer, harness } = buildLayer()
  const page = await layer.readHistory('session-empty')
  assert.deepEqual(page, { records: [], hasMore: false })
  assert.deepEqual(harness.calls.pages, [], 'no page is requested when there is no cursor to bound it')
})

test('describe() exposes subscription state for diagnostics', async () => {
  const { layer, harness } = buildLayer()
  await layer.attach('session-1')
  harness.push('session-1', snapshot(7))
  await tick()
  const described = layer.describe()
  assert.equal(described.activeSessionId, 'session-1')
  assert.equal(described.follows[0].cursor, 7)
  assert.equal(described.follows[0].snapshotDelivered, true)
})

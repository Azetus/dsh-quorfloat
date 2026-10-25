// Category 1 (cross-process protocol): the session push stream's frames, filters, and lifetime.
/**
 * The host half of the session list push.
 *
 * The panel's conversation list used to be whatever `sessions/list` said when it was last
 * asked. This stream is what replaces that poll: the Harness's own `api-session/*` events,
 * projected into the few frame shapes the sidecar parses, plus the title projection as its
 * own increment.
 *
 * Everything here is the wire contract, so the assertions are about shapes rather than about
 * implementation: a frame the sidecar cannot read is the same defect as a frame never sent.
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'

import { loadModule } from './helpers.mjs'

const { startSessionStream } = await loadModule('harness/session-stream.js')

/** A Cordis-shaped context: the two methods this module uses, and an emit for the test. */
function fakeContext(services = {}) {
  const listeners = new Map()
  return {
    on(name, listener) {
      const list = listeners.get(name) ?? []
      list.push(listener)
      listeners.set(name, list)
      return () => {
        const current = listeners.get(name) ?? []
        const index = current.indexOf(listener)
        if (index >= 0) current.splice(index, 1)
      }
    },
    get(name) {
      return services[name]
    },
    emit(name, ...args) {
      for (const listener of [...(listeners.get(name) ?? [])]) listener(...args)
    },
    count(name) {
      return (listeners.get(name) ?? []).length
    },
  }
}

/** The one method of `sessionProjections` this module subscribes to. */
function fakeProjections() {
  const listeners = []
  return {
    onChanged(listener) {
      listeners.push(listener)
      return () => {
        const index = listeners.indexOf(listener)
        if (index >= 0) listeners.splice(index, 1)
      }
    },
    count() {
      return listeners.length
    },
    change(session, key, value, seq = 1) {
      for (const listener of [...listeners]) listener(session, key, value, seq)
    },
  }
}

/** A logger that records warnings instead of printing them. */
function recordingLog() {
  const warnings = []
  return {
    warnings,
    warn: (...args) => warnings.push(args.map(String).join(' ')),
    debug: () => {},
  }
}

/** Start the stream over a fake context, collecting the frames it forwards. */
function open(services = {}, { onFrame } = {}) {
  const frames = []
  const log = recordingLog()
  const ctx = fakeContext(services)
  const stream = startSessionStream({
    ctx,
    log,
    onFrame: onFrame ?? (frame => frames.push(frame)),
  })
  return { ctx, frames, log, stream }
}

test('an added summary becomes one whole-row upsert, and subagents never do', async () => {
  const { ctx, frames, stream } = open()
  try {
    ctx.emit('api-session/added', {
      sessionId: 's1',
      updatedAt: 10,
      running: true,
      blank: false,
      cwd: '/work/one',
      projections: { kind: 'sequenced', asOfSeq: 3, values: { title: 'One' } },
    })
    // The picker must not offer a subagent's conversation, and the host's list drops these
    // before the title pass; the push uses the same projection so it cannot disagree.
    ctx.emit('api-session/added', { sessionId: 'child', origin: 'subagent', updatedAt: 11, running: false, blank: false })
    // An identity is what makes a row usable; a nameless one is not a row.
    ctx.emit('api-session/added', { sessionId: '', updatedAt: 1, running: false, blank: false })
    await stream.settled()

    assert.deepEqual(frames, [{
      type: 'upsert',
      session: {
        sessionId: 's1',
        title: 'One',
        cwd: '/work/one',
        updatedAt: 10,
        running: true,
        blank: false,
      },
    }])
  } finally {
    stream.close()
  }
})

test('status, activity and removal each become their own frame, and malformed ones are dropped', async () => {
  const { ctx, frames, stream } = open()
  try {
    ctx.emit('api-session/status', 's1', true)
    ctx.emit('api-session/status', '', true)
    ctx.emit('api-session/status', 's1', 'yes')
    ctx.emit('api-session/activity', 's1', 42)
    ctx.emit('api-session/activity', 's1', 'later')
    // A removal is not a deletion: the sidecar asks for the authoritative list instead, so the
    // payload is a request to reconcile rather than a row to drop.
    ctx.emit('api-session/removed', 's1')
    ctx.emit('api-session/removed', 7)
    await stream.settled()

    assert.deepEqual(frames, [
      { type: 'status', sessionId: 's1', running: true },
      { type: 'activity', sessionId: 's1', updatedAt: 42 },
      { type: 'refresh', reason: 'removed' },
    ])
  } finally {
    stream.close()
  }
})

test('a title projection change becomes a title frame; other keys and subagents do not', async () => {
  const projections = fakeProjections()
  const { ctx, frames, stream } = open({ sessionProjections: projections })
  try {
    assert.equal(projections.count(), 1, 'the stream subscribes to the projection feed')
    projections.change({ id: 's1', header: { origin: 'primary' } }, 'title', 'Renamed')
    // The same feed carries token usage and turn boundaries; only the name is a list fact.
    projections.change({ id: 's1', header: { origin: 'primary' } }, 'tokenUsage', 1234)
    projections.change({ id: 'child', header: { origin: 'subagent' } }, 'title', 'Child')
    projections.change({ id: 's1', header: { origin: 'primary' } }, 'title', null)
    await stream.settled()

    assert.deepEqual(frames, [
      { type: 'title', sessionId: 's1', title: 'Renamed' },
      { type: 'title', sessionId: 's1', title: null },
    ])
    assert.equal(ctx.count('api-session/added'), 1, 'the event subscriptions are still in place')
  } finally {
    stream.close()
  }
})

test('a Harness without the projection feed still forwards events, and says why titles lag', async () => {
  const { ctx, frames, log, stream } = open()
  try {
    ctx.emit('api-session/status', 's1', false)
    await stream.settled()
    assert.deepEqual(frames, [{ type: 'status', sessionId: 's1', running: false }])
    assert.ok(
      log.warnings.some(line => line.includes('sessionProjections')),
      `the missing service is reported, not silent: ${JSON.stringify(log.warnings)}`,
    )
  } finally {
    stream.close()
  }
})

test('close removes every subscription, including the projection feed', async () => {
  const projections = fakeProjections()
  const { ctx, frames, stream } = open({ sessionProjections: projections })
  stream.close()

  ctx.emit('api-session/added', { sessionId: 's1', updatedAt: 1, running: false, blank: false })
  projections.change({ id: 's1', header: { origin: 'primary' } }, 'title', 'After close')
  await stream.settled()

  assert.deepEqual(frames, [])
  assert.equal(projections.count(), 0)
  assert.equal(ctx.count('api-session/added'), 0)
})

test('a failing forward is reported and never escapes the host event dispatch', async () => {
  const log = recordingLog()
  const ctx = fakeContext()
  const stream = startSessionStream({
    ctx,
    log,
    onFrame: () => {
      throw new Error('boom')
    },
  })
  try {
    assert.doesNotThrow(() => ctx.emit('api-session/status', 's1', true))
    await stream.settled()
    assert.ok(log.warnings.some(line => line.includes('could not forward')))
  } finally {
    stream.close()
  }

  // The asynchronous half has the same rule: a rejected write must not become an unhandled
  // rejection in the host process.
  const asyncLog = recordingLog()
  const asyncCtx = fakeContext()
  const second = startSessionStream({
    ctx: asyncCtx,
    log: asyncLog,
    onFrame: async () => {
      throw new Error('async boom')
    },
  })
  try {
    asyncCtx.emit('api-session/status', 's1', true)
    await second.settled()
    assert.ok(asyncLog.warnings.some(line => line.includes('could not forward')))
  } finally {
    second.close()
  }
})

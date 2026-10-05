/**
 * Interaction tests: ownership filtering, one-shot settlement, and withdrawal.
 *
 * The waterfall contract these tests pin down is the one the design document
 * calls a hard requirement: the panel answers only for sessions it owns, an
 * answer takes effect once, a second answer is refused rather than re-executed,
 * and a cancelled request never leaves the turn hanging.
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'

import { loadModule, recordingLogger } from './helpers.mjs'

const { Interactions } = await loadModule('harness/interactions.js')

/** A minimal Cordis-like context that records listeners and can dispatch them. */
function fakeContext() {
  const listeners = new Map()
  const disposers = []
  return {
    listeners,
    effect: () => () => {},
    inject: () => () => {},
    get: () => undefined,
    on(name, listener) {
      listeners.set(name, listener)
      const dispose = () => listeners.delete(name)
      disposers.push(dispose)
      return dispose
    },
    emit: () => {},
    logger: () => recordingLogger(),
    dispatch(name, ...args) {
      const listener = listeners.get(name)
      if (listener === undefined) throw new Error(`no listener for ${name}`)
      return listener(...args)
    },
    has(name) {
      return listeners.has(name)
    },
  }
}

/** Build interactions with a fixed ownership set and a captured notification log. */
function build({ owned = ['session-owned'], notify } = {}) {
  const ctx = fakeContext()
  const notifications = []
  const ownerSet = new Set(owned)
  const interactions = new Interactions({
    ctx,
    ownedSessionIds: () => [...ownerSet],
    notify: async (method, params) => {
      notifications.push({ method, params })
      await notify?.(method, params)
    },
    log: recordingLogger(),
  })
  interactions.register()
  return { ctx, interactions, notifications, ownerSet }
}

/** The `next` delegate used by waterfall listeners in these tests. */
const delegated = Symbol('delegated')
const next = async () => delegated

test('both answerers register on the owning context', () => {
  const { ctx, interactions } = build()
  assert.equal(interactions.registered, true)
  assert.ok(ctx.has('approval/request'))
  assert.ok(ctx.has('user-questions/request'))
})

test('a request for a session this host does not own is delegated untouched', async () => {
  const { ctx } = build({ owned: ['session-owned'] })
  const outcome = await ctx.dispatch('approval/request', { agent: { id: 'session-other' }, toolName: 'bash' }, next)
  assert.equal(outcome, delegated)
})

test('an agentless request is delegated rather than claimed', async () => {
  const { ctx } = build()
  const outcome = await ctx.dispatch('approval/request', { toolName: 'bash' }, next)
  assert.equal(outcome, delegated)
})

test('an owned approval is published to the peer and settles with its answer', async () => {
  const { ctx, interactions, notifications } = build()
  const pending = ctx.dispatch(
    'approval/request',
    { agent: { id: 'session-owned' }, toolName: 'bash', callId: 'call-1', reason: 'needs network' },
    next,
  )
  await new Promise(resolve => setImmediate(resolve))
  const open = notifications.find(entry => entry.method === 'interaction/open')
  assert.ok(open !== undefined, 'the peer must be told about the request')
  assert.equal(open.params.kind, 'approval')
  assert.equal(open.params.payload.toolName, 'bash')
  assert.equal(open.params.payload.reason, 'needs network')

  const result = interactions.answer(open.params.interactionId, { kind: 'approval', outcome: 'allowed-once' })
  assert.equal(result.accepted, true)
  assert.equal(await pending, 'allowed-once')
})

test('a second answer for the same request is refused, not applied twice', async () => {
  const { ctx, interactions, notifications } = build()
  const pending = ctx.dispatch('approval/request', { agent: { id: 'session-owned' }, toolName: 'bash' }, next)
  await new Promise(resolve => setImmediate(resolve))
  const interactionId = notifications.find(entry => entry.method === 'interaction/open').params.interactionId

  assert.equal(interactions.answer(interactionId, { kind: 'approval', outcome: 'rejected' }).accepted, true)
  assert.equal(await pending, 'rejected')
  const second = interactions.answer(interactionId, { kind: 'approval', outcome: 'allowed-once' })
  assert.equal(second.accepted, false)
  assert.match(String(second.reason), /already settled/)
})

test('an answer of the wrong kind is refused with a reason', async () => {
  const { ctx, interactions, notifications } = build()
  ctx.dispatch('approval/request', { agent: { id: 'session-owned' }, toolName: 'bash' }, next)
  await new Promise(resolve => setImmediate(resolve))
  const interactionId = notifications.find(entry => entry.method === 'interaction/open').params.interactionId
  const result = interactions.answer(interactionId, { kind: 'question', answers: [] })
  assert.equal(result.accepted, false)
  assert.match(String(result.reason), /does not match an approval/)
})

test('an unknown interaction id is refused without throwing', () => {
  const { interactions } = build()
  const result = interactions.answer('approval-does-not-exist', { kind: 'approval', outcome: 'rejected' })
  assert.equal(result.accepted, false)
})

test('an aborted request settles as cancelled and cannot be answered later', async () => {
  const { ctx, interactions, notifications } = build()
  const controller = new AbortController()
  const pending = ctx.dispatch(
    'approval/request',
    { agent: { id: 'session-owned' }, toolName: 'bash', signal: controller.signal },
    next,
  )
  await new Promise(resolve => setImmediate(resolve))
  const interactionId = notifications.find(entry => entry.method === 'interaction/open').params.interactionId
  controller.abort(new Error('turn cancelled'))
  assert.equal(await pending, 'cancelled')
  const late = interactions.answer(interactionId, { kind: 'approval', outcome: 'allowed-once' })
  assert.equal(late.accepted, false, 'a late answer must never re-execute a cancelled action')
})

test('abortAll withdraws every pending request', async () => {
  const { ctx, interactions } = build()
  const first = ctx.dispatch('approval/request', { agent: { id: 'session-owned' }, toolName: 'bash' }, next)
  const second = ctx.dispatch('approval/request', { agent: { id: 'session-owned' }, toolName: 'read' }, next)
  await new Promise(resolve => setImmediate(resolve))
  interactions.abortAll('peer disconnected')
  assert.equal(await first, 'cancelled')
  assert.equal(await second, 'cancelled')
  assert.equal(Object.keys(interactions.describe().counters).length > 0, true)
})

test('an owned question is published with its options and answered as a batch', async () => {
  const { ctx, interactions, notifications } = build()
  const pending = ctx.dispatch(
    'user-questions/request',
    {
      agent: { id: 'session-owned' },
      questions: [
        { id: 'q1', question: 'Which preset?', options: [{ label: 'read-only' }, { label: 'write' }] },
        { id: 'q2', question: 'Anything else?', multiSelect: true },
      ],
    },
    next,
  )
  await new Promise(resolve => setImmediate(resolve))
  const open = notifications.find(entry => entry.method === 'interaction/open')
  assert.equal(open.params.kind, 'question')
  assert.equal(open.params.payload.questions.length, 2)
  assert.equal(open.params.payload.questions[0].options.length, 2)

  const result = interactions.answer(open.params.interactionId, {
    kind: 'question',
    answers: [
      { id: 'q1', selected: ['read-only'] },
      { id: 'q2', selected: [], custom: 'no' },
    ],
  })
  assert.equal(result.accepted, true)
  assert.deepEqual(await pending, {
    answers: [
      { id: 'q1', selected: ['read-only'] },
      { id: 'q2', selected: [], custom: 'no' },
    ],
  })
})

test('an empty question list is delegated instead of claimed', async () => {
  const { ctx } = build()
  const outcome = await ctx.dispatch('user-questions/request', { agent: { id: 'session-owned' }, questions: [] }, next)
  assert.equal(outcome, delegated)
})

test('a question for a session owned elsewhere is delegated', async () => {
  const { ctx } = build({ owned: ['session-owned'] })
  const outcome = await ctx.dispatch(
    'user-questions/request',
    { agent: { id: 'session-other' }, questions: [{ id: 'q1', question: 'x' }] },
    next,
  )
  assert.equal(outcome, delegated)
})

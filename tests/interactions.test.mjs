// Category 1 (cross-process protocol): the parameters our `interaction/answer` carries to the Rust peer.
//
// Pinned literally, because it is a cross-language contract with no compiler between the
// two halves: the Rust side builds `{"interactionId":…,"answer":{"outcome":"allowed-once"}}`
// by hand. A second field is not required, and the closed vocabulary is what decides the
// outcome — anything else is refused rather than read as "no".
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { loadModule, recordingLogger } from './helpers.mjs'

const { Interactions } = await loadModule('harness/interactions.js')

/**
 * A minimal Cordis-like context that models the two properties this file's
 * ordering test depends on: listeners for one event form an ordered *list*, and
 * `{ prepend: true }` inserts at the front.
 */
function fakeContext() {
  const listeners = new Map()
  return {
    effect: () => () => {},
    inject: () => () => {},
    get: () => undefined,
    on(name, listener, options) {
      const list = listeners.get(name) ?? []
      if (options?.prepend === true) list.unshift(listener)
      else list.push(listener)
      listeners.set(name, list)
      return () => {}
    },
    emit: () => {},
    logger: () => recordingLogger(),
    dispatch(name, ...args) {
      // Waterfall semantics: each listener receives `next`, which continues the
      // chain. A listener that never calls it ends the chain, exactly like the
      // browser forwarder it stands in for.
      const list = listeners.get(name) ?? []
      // An empty chain is not an error: a real waterfall falls through to the
      // fallback its caller supplied, which is how `unavailable` is produced.
      const step = index => (...rest) => {
        const next = () => Promise.resolve(step(index + 1)(...rest))
        const listener = list[index]
        if (listener === undefined) return Promise.resolve('unavailable')
        return listener(...rest, next)
      }
      return step(0)(...args)
    },
  }
}

/**
 * Authority used by the kept test: the panel decides, silently.
 *
 * Routing is not this file's subject any more; the test only needs the claim to
 * succeed so that an answerable card is published.
 */
const PANEL_DECIDES = { authority: 'panel', reason: 'harness-not-visible', fresh: [] }

/** A peer that answers approvals only, as `hello` declares. */
const PANEL_ANSWERS = kind => kind === 'approval'

/** Build interactions with a captured notification log. */
function build() {
  const ctx = fakeContext()
  const notifications = []
  const interactions = new Interactions({
    ctx,
    ownedSessionIds: () => ['session-owned'],
    panelSession: () => 'session-owned',
    authority: () => PANEL_DECIDES,
    canAnswer: PANEL_ANSWERS,
    notify: async (method, params) => {
      notifications.push({ method, params })
    },
    log: recordingLogger(),
  })
  interactions.register()
  return { ctx, interactions, notifications }
}

/** The `next` delegate used by waterfall listeners in this file. */
const delegated = Symbol('delegated')
const next = async () => delegated

test('the answer shape the panel writes is the one the host validates', async () => {
  const { ctx, interactions, notifications } = build()
  const pending = ctx.dispatch('approval/request', { agent: { id: 'session-owned' }, toolName: 'bash' }, next)
  await new Promise(resolve => setImmediate(resolve))
  const open = notifications.find(entry => entry.method === 'interaction/open')

  assert.deepEqual(interactions.answer(open.params.interactionId, { outcome: 'allowed-once' }), {
    accepted: true,
  })
  assert.equal(await pending, 'allowed-once')
})

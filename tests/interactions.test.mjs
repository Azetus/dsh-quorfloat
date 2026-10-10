/**
 * Interaction tests: ownership filtering, one-shot settlement, and withdrawal.
 *
 * The waterfall contract these tests pin down: the panel answers only for sessions it
 * owns, an answer takes effect once, a second answer is refused rather than
 * re-executed, and a cancelled request never leaves the turn hanging.
 */

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
  const disposers = []
  return {
    listeners,
    effect: () => () => {},
    inject: () => () => {},
    get: () => undefined,
    on(name, listener, options) {
      const list = listeners.get(name) ?? []
      if (options?.prepend === true) list.unshift(listener)
      else list.push(listener)
      listeners.set(name, list)
      const dispose = () => {
        const current = listeners.get(name) ?? []
        const index = current.indexOf(listener)
        if (index >= 0) current.splice(index, 1)
      }
      disposers.push(dispose)
      return dispose
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
      const step = index => (...args2) => {
        const next = () => Promise.resolve(step(index + 1)(...args2))
        const listener = list[index]
        if (listener === undefined) return Promise.resolve('unavailable')
        return listener(...args2, next)
      }
      return step(0)(...args)
    },
    has(name) {
      return (listeners.get(name)?.length ?? 0) > 0
    },
    count(name) {
      return listeners.get(name)?.length ?? 0
    },
  }
}

/**
 * Authority used by the behaviour tests: the panel decides, silently.
 *
 * Routing is covered separately in `presence.test.mjs` and by the ordering tests
 * below; these tests are about what happens *after* a claim.
 */
const PANEL_DECIDES = { authority: 'panel', reason: 'harness-not-visible', fresh: [] }

/**
 * What a peer declared in `hello`, as the behaviour tests assume it: one that answers
 * approvals only. A panel that claims a question it cannot render hides it from the
 * Harness window for the whole claim deadline, so claiming is gated on this — and the
 * narrower peer is the case that must stay safe.
 */
const PANEL_ANSWERS = kind => kind === 'approval'

/** Build interactions with a fixed ownership set and a captured notification log. */
function build({
  owned = ['session-owned'],
  notify,
  authority = () => PANEL_DECIDES,
  canAnswer = PANEL_ANSWERS,
  panel = 'session-owned',
} = {}) {
  const ctx = fakeContext()
  const notifications = []
  const ownerSet = new Set(owned)
  const interactions = new Interactions({
    ctx,
    ownedSessionIds: () => [...ownerSet],
    // Which conversation the panel is showing. The default matches the requests these tests
    // make; `panel: undefined` is the panel being open on nothing, and another id is the panel
    // being open on something else — the case the rule added for it is about.
    // `null` means the panel is open and attached to nothing: passing `undefined` would take
    // the default above instead, which would test the opposite of what this says.
    panelSession: () => panel ?? undefined,
    authority,
    canAnswer,
    notify: async (method, params) => {
      notifications.push({ method, params })
      await notify?.(method, params)
    },
    log: recordingLogger(),
  })
  interactions.register()
  lastInteractions = interactions
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

test('a panel that is showing another conversation defers to the window', async () => {
  // The rule this test was added for: an open panel is not enough. The panel answers for the
  // conversation it is displaying, and for nothing else — a card whose context is not on
  // screen is worse than the window answering, which shows the request in its transcript.
  const { ctx, notifications } = build({ panel: 'session-elsewhere' })
  const outcome = await ctx.dispatch(
    'approval/request',
    { agent: { id: 'session-owned' }, toolName: 'bash' },
    next,
  )

  assert.equal(outcome, delegated, 'the next listener gets it')
  assert.deepEqual(notifications, [], 'and the panel was told nothing: it is not the answerer')
})

test('a panel that is open on nothing at all defers too', async () => {
  // The state a freshly opened panel is in before it is attached to anything: it has no
  // context for any request, so every request belongs to the window.
  const { ctx } = build({ panel: null })
  const outcome = await ctx.dispatch(
    'approval/request',
    { agent: { id: 'session-owned' }, toolName: 'bash' },
    next,
  )
  assert.equal(outcome, delegated, 'nothing is claimed')
})

test('the panel answers for its own conversation, and only then', async () => {
  // Both halves in one place, so that "open" and "showing this one" cannot drift apart again.
  // A claimed dispatch does not resolve until somebody answers it — that is what claiming
  // means — so the claimed half is observed through what the panel was told.
  const mine = build({ panel: 'session-owned' })
  const pending = mine.ctx.dispatch(
    'approval/request',
    { agent: { id: 'session-owned' }, toolName: 'bash', callId: 'call-1' },
    next,
  )
  await new Promise(resolve => setImmediate(resolve))
  const open = mine.notifications.find(entry => entry.method === 'interaction/open')
  assert.ok(open !== undefined, 'the panel was given its own conversation')
  mine.interactions.answer(open.params.interactionId, { kind: 'approval', outcome: 'allowed-once' })
  assert.equal(await pending, 'allowed-once', 'and its answer settles the request')

  const other = build({ panel: 'session-elsewhere' })
  const deferred = await other.ctx.dispatch(
    'approval/request',
    { agent: { id: 'session-owned' }, toolName: 'bash' },
    next,
  )
  assert.equal(deferred, delegated, 'while a request for another conversation is left alone')
  assert.deepEqual(other.notifications, [], 'and the panel was not even told about it')
})

test('an owned approval is published to the peer and settles with its answer', async () => {
  const { ctx, interactions, notifications } = build()
  const pending = ctx.dispatch(
    'approval/request',
    {
      agent: { id: 'session-owned' },
      toolName: 'bash',
      callId: 'call-1',
      reason: 'needs network',
      displayReason: { en: 'Allow this with network access?', zh: '允许访问网络吗？', fr: 7 },
    },
    next,
  )
  await new Promise(resolve => setImmediate(resolve))
  const open = notifications.find(entry => entry.method === 'interaction/open')
  assert.ok(open !== undefined, 'the peer must be told about the request')
  assert.equal(open.params.kind, 'approval')
  assert.equal(open.params.payload.toolName, 'bash')
  assert.equal(open.params.payload.reason, 'needs network')
  // The card shows the text written to be read, not the audit string; a locale
  // filled with something that is not text is dropped rather than coerced.
  assert.deepEqual(open.params.payload.displayReason, {
    en: 'Allow this with network access?',
    zh: '允许访问网络吗？',
  })

  const result = interactions.answer(open.params.interactionId, { kind: 'approval', outcome: 'allowed-once' })
  assert.equal(result.accepted, true)
  assert.equal(await pending, 'allowed-once')
})

test('an approval without usable display text carries none rather than junk', async () => {
  const { ctx, notifications } = build()
  ctx.dispatch(
    'approval/request',
    { agent: { id: 'session-owned' }, toolName: 'bash', displayReason: { zh: '   ' } },
    next,
  )
  await new Promise(resolve => setImmediate(resolve))
  const open = notifications.find(entry => entry.method === 'interaction/open')
  assert.equal(open.params.payload.displayReason, null, 'whitespace is not text to show a user')
})

test('the answer shape the panel writes is the one the host validates', async () => {
  // Pinned literally, because it is a cross-language contract with no compiler
  // between the two halves: the Rust side builds `{"interactionId":…,"answer":
  // {"outcome":"allowed-once"}}` by hand. A second field is not required, and the
  // closed vocabulary is what decides the outcome — anything else is refused rather
  // than read as "no".
  const { ctx, interactions, notifications } = build()
  const pending = ctx.dispatch('approval/request', { agent: { id: 'session-owned' }, toolName: 'bash' }, next)
  await new Promise(resolve => setImmediate(resolve))
  const open = notifications.find(entry => entry.method === 'interaction/open')

  assert.deepEqual(interactions.answer(open.params.interactionId, { outcome: 'allowed-once' }), {
    accepted: true,
  })
  assert.equal(await pending, 'allowed-once')
})

test('an outcome outside the closed vocabulary is refused, not read as a refusal', async () => {
  // `allowed-forever` must never be mistaken for a rejection, and must never be
  // mistaken for a grant: it is not in the vocabulary, so it settles nothing.
  const { ctx, interactions, notifications } = build()
  const pending = ctx.dispatch('approval/request', { agent: { id: 'session-owned' }, toolName: 'bash' }, next)
  await new Promise(resolve => setImmediate(resolve))
  const open = notifications.find(entry => entry.method === 'interaction/open')

  const refused = interactions.answer(open.params.interactionId, { outcome: 'allowed-forever' })
  assert.equal(refused.accepted, false)
  assert.match(refused.reason, /does not match an approval interaction/)
  // Still pending afterwards: the request can still be answered properly.
  assert.deepEqual(interactions.answer(open.params.interactionId, { outcome: 'rejected' }), { accepted: true })
  assert.equal(await pending, 'rejected')
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
  // Only for a peer that declared it can answer questions (see `canAnswer`): a peer
  // that declares approvals only never reaches this path, so this exercises the
  // machinery a question-capable build needs.
  const { ctx, interactions, notifications } = build({ canAnswer: () => true })
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


/**
 * A listener that behaves like the browser forwarder: it takes the request, hands
 * it to a client that may not exist, and never calls `next()`. Reaching it means
 * the turn hangs.
 */
function blockingForwarder(hits) {
  return function forwarder() {
    hits.push('forwarder')
    return new Promise(() => {})
  }
}

test('an answerer registered later still runs first (prepend)', async () => {
  // `dsh-api-remotes` registers this event first and never delegates, so a plugin
  // that registers afterwards is never reached — the approval is logged as asked
  // and never decided. `prepend` is what puts this plugin in front.
  const ctx = fakeContext()
  const notifications = []
  const hits = []
  ctx.on('approval/request', blockingForwarder(hits), {})
  const interactions = new Interactions({
    ctx,
    ownedSessionIds: () => ['session-owned'],
    // The panel is showing the conversation the request belongs to, which is the case these
    // tests are about; the mismatch has its own test below.
    panelSession: () => 'session-owned',
    authority: () => PANEL_DECIDES,
    canAnswer: PANEL_ANSWERS,
    notify: async (method, params) => {
      notifications.push({ method, params })
    },
    log: recordingLogger(),
  })
  assert.equal(interactions.register(), true)

  const pending = ctx.dispatch('approval/request', { agent: { id: 'session-owned' }, toolName: 'bash' })
  const open = await waitForOpen(notifications)
  assert.equal(interactions.answer(open.params.interactionId, { kind: 'approval', outcome: 'allowed-once' }).accepted, true)
  const outcome = await Promise.race([
    pending,
    new Promise(resolve => setTimeout(() => resolve('HUNG'), 500)),
  ])
  assert.equal(outcome, 'allowed-once')
  assert.deepEqual(hits, [], 'the non-delegating forwarder must never be reached')
})

test('a listener placed after ours is not reached either', async () => {
  const { ctx, notifications } = build()
  const hits = []
  ctx.on('approval/request', blockingForwarder(hits), {})
  const pending = ctx.dispatch('approval/request', { agent: { id: 'session-owned' }, toolName: 'bash' })
  const open = await waitForOpen(notifications)
  interactionsAnswer(ctx, open.params.interactionId, { kind: 'approval', outcome: 'rejected' })
  const outcome = await Promise.race([pending, new Promise(resolve => setTimeout(() => resolve('HUNG'), 500))])
  assert.equal(outcome, 'rejected')
  assert.deepEqual(hits, [])
})

/** Wait for the peer-facing `interaction/open` notification. */
async function waitForOpen(notifications) {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    const found = notifications.find(entry => entry.method === 'interaction/open')
    if (found !== undefined) return found
    await new Promise(resolve => setTimeout(resolve, 5))
  }
  throw new Error('the interaction was never published to the peer')
}

/**
 * Answer through the interactions instance the dispatching test built.
 *
 * The instance is reachable from the context because `build()` keeps the
 * listener closures alive; this indirection only exists to keep the two ordering
 * tests readable.
 */
let lastInteractions
function interactionsAnswer(ctx, interactionId, answer) {
  assert.ok(lastInteractions !== undefined, 'no interactions instance was recorded')
  return lastInteractions.answer(interactionId, answer)
}

test('an unanswered claim settles instead of holding the turn forever', async () => {
  // A claim excludes every other answerer, so the wait has to be bounded: an
  // abandoned panel must not reproduce the hang this plugin exists to avoid.
  const ctx = fakeContext()
  const notifications = []
  const interactions = new Interactions(
    {
      ctx,
      ownedSessionIds: () => ['session-owned'],
    // The panel is showing the conversation the request belongs to, which is the case these
    // tests are about; the mismatch has its own test below.
    panelSession: () => 'session-owned',
      authority: () => PANEL_DECIDES,
      canAnswer: PANEL_ANSWERS,
      notify: async (method, params) => {
        notifications.push({ method, params })
      },
      log: recordingLogger(),
    },
    { deadlineMs: 80 },
  )
  interactions.register()
  const outcome = await ctx.dispatch('approval/request', { agent: { id: 'session-owned' }, toolName: 'bash' })
  assert.equal(outcome, 'cancelled', 'an expired claim fails closed as cancelled')
  assert.equal(notifications.filter(entry => entry.method === 'interaction/open').length, 1)
})

test('an answer arriving after the deadline is refused, not applied', async () => {
  const ctx = fakeContext()
  const notifications = []
  const interactions = new Interactions(
    {
      ctx,
      ownedSessionIds: () => ['session-owned'],
    // The panel is showing the conversation the request belongs to, which is the case these
    // tests are about; the mismatch has its own test below.
    panelSession: () => 'session-owned',
      authority: () => PANEL_DECIDES,
      canAnswer: PANEL_ANSWERS,
      notify: async (method, params) => {
        notifications.push({ method, params })
      },
      log: recordingLogger(),
    },
    { deadlineMs: 60 },
  )
  interactions.register()
  await ctx.dispatch('approval/request', { agent: { id: 'session-owned' }, toolName: 'bash' })
  const interactionId = notifications.find(entry => entry.method === 'interaction/open').params.interactionId
  const late = interactions.answer(interactionId, { kind: 'approval', outcome: 'allowed-once' })
  assert.equal(late.accepted, false, 'a late approval must never grant elevated permissions')
})


test('unregister removes both listeners so nothing outlives the plugin', () => {
  // A prepended listener that survived unload would keep claiming requests for a
  // panel that no longer exists — worse than hanging, because nothing would ever
  // answer.
  const ctx = fakeContext()
  const interactions = new Interactions({
    ctx,
    ownedSessionIds: () => ['session-owned'],
    // The panel is showing the conversation the request belongs to, which is the case these
    // tests are about; the mismatch has its own test below.
    panelSession: () => 'session-owned',
    authority: () => PANEL_DECIDES,
    canAnswer: PANEL_ANSWERS,
    notify: async () => {},
    log: recordingLogger(),
  })
  interactions.register()
  assert.equal(ctx.count('approval/request'), 1)
  assert.equal(ctx.count('user-questions/request'), 1)
  interactions.unregister()
  assert.equal(ctx.count('approval/request'), 0)
  assert.equal(ctx.count('user-questions/request'), 0)
})

test('an unregistered plugin never claims, and nothing hangs on its behalf', async () => {
  const ctx = fakeContext()
  const interactions = new Interactions({
    ctx,
    ownedSessionIds: () => ['session-owned'],
    // The panel is showing the conversation the request belongs to, which is the case these
    // tests are about; the mismatch has its own test below.
    panelSession: () => 'session-owned',
    authority: () => PANEL_DECIDES,
    canAnswer: PANEL_ANSWERS,
    notify: async () => {},
    log: recordingLogger(),
  })
  interactions.register()
  interactions.unregister()
  const outcome = await ctx.dispatch('approval/request', { agent: { id: 'session-owned' }, toolName: 'bash' })
  assert.equal(outcome, 'unavailable', 'with no listeners the waterfall falls through to its default')
})


/** Build interactions with a fixed authority verdict, for routing tests. */
function buildWithAuthority(verdict, canAnswer = PANEL_ANSWERS) {
  const ctx = fakeContext()
  const notifications = []
  const interactions = new Interactions({
    ctx,
    ownedSessionIds: () => ['session-owned'],
    // The panel is showing the conversation the request belongs to, which is the case these
    // tests are about; the mismatch has its own test below.
    panelSession: () => 'session-owned',
    authority: () => verdict,
    canAnswer,
    notify: async (method, params) => {
      notifications.push({ method, params })
    },
    log: recordingLogger(),
  })
  interactions.register()
  lastInteractions = interactions
  return { ctx, interactions, notifications }
}

test('a visible Harness window keeps the decision, and the panel only gets a hint', async () => {
  // Claiming here would hide the approval from the window the user is looking at.
  const { ctx, notifications } = buildWithAuthority({
    authority: 'harness',
    reason: 'harness-visible',
    fresh: ['desktop'],
  })
  const outcome = await ctx.dispatch('approval/request', { agent: { id: 'session-owned' }, toolName: 'bash' })
  assert.equal(outcome, 'unavailable', 'the request was passed on, not claimed')
  assert.deepEqual(notifications, [], 'a focused window needs no hint: it shows the card itself')
})

test('a claimed approval needs no pointer to the Harness window', async () => {
  // Claiming is exclusive, so an open-but-unfocused Harness window has nothing to
  // show for this request: the card is on the panel the user is looking at, and a
  // hint sending them elsewhere is noise at best.
  const { ctx, notifications } = buildWithAuthority({
    authority: 'panel',
    reason: 'harness-open-but-idle',
    fresh: ['desktop'],
  })
  const pending = ctx.dispatch('approval/request', { agent: { id: 'session-owned' }, toolName: 'bash' })
  const open = await waitForNotification(notifications, 'interaction/open')
  assert.equal(
    notifications.filter(entry => entry.method === 'interaction/hint').length,
    0,
    'the card is right there',
  )
  interactionsAnswer(ctx, open.params.interactionId, { kind: 'approval', outcome: 'allowed-once' })
  assert.equal(await pending, 'allowed-once')
})

test('a question the panel cannot answer is announced and handed over at once', async () => {
  // Claiming a question this build has no widget for hides it from the Harness
  // window for the entire claim deadline and then hands back a request that was
  // invisible the whole time. The user is told where it went instead, and the
  // request reaches the answerer that can actually take it.
  const { ctx, notifications } = buildWithAuthority(
    { authority: 'panel', reason: 'harness-not-visible', fresh: ['web'] },
    () => false,
  )
  const outcome = await ctx.dispatch(
    'user-questions/request',
    { agent: { id: 'session-owned' }, questions: [{ id: 'q1', question: 'Which preset?' }] },
  )
  assert.equal(outcome, 'unavailable', 'passed on immediately, not held until the deadline')
  assert.equal(
    notifications.filter(entry => entry.method === 'interaction/open').length,
    0,
    'nothing is published to a panel that cannot render it',
  )
  const hint = await waitForNotification(notifications, 'interaction/hint')
  assert.equal(hint.params.kind, 'question', 'the panel learns what kind it is being told about')
  assert.deepEqual(hint.params.surfaces, ['web'], 'and which surface will show it')
})

test('a hidden window hands over without a hint', async () => {
  const { ctx, notifications } = buildWithAuthority({
    authority: 'panel',
    reason: 'harness-not-visible',
    fresh: [],
  })
  const pending = ctx.dispatch('approval/request', { agent: { id: 'session-owned' }, toolName: 'bash' })
  const open = await waitForNotification(notifications, 'interaction/open')
  assert.equal(notifications.filter(entry => entry.method === 'interaction/hint').length, 0,
    'nothing to point at when the window is hidden')
  interactionsAnswer(ctx, open.params.interactionId, { kind: 'approval', outcome: 'rejected' })
  assert.equal(await pending, 'rejected')
})

test('when no surface can answer, the request is deferred rather than held', async () => {
  // Claiming with nobody able to answer would keep the turn open for the whole
  // deadline and then fail anyway, so the plugin passes it on immediately.
  const { ctx, notifications } = buildWithAuthority({
    authority: 'none',
    reason: 'nobody-looking',
    fresh: [],
  })
  const outcome = await ctx.dispatch('approval/request', { agent: { id: 'session-owned' }, toolName: 'bash' })
  assert.equal(outcome, 'unavailable')
  assert.deepEqual(notifications, [])
})

test('the authority is consulted for questions too, not just approvals', async () => {
  const { ctx, notifications } = buildWithAuthority({
    authority: 'harness',
    reason: 'harness-visible',
    fresh: ['web'],
  })
  const outcome = await ctx.dispatch(
    'user-questions/request',
    { agent: { id: 'session-owned' }, questions: [{ id: 'q1', question: 'x' }] },
  )
  assert.equal(outcome, 'unavailable')
  assert.deepEqual(notifications, [])
})

test('a session this host does not own is deferred regardless of the authority', async () => {
  // Ownership is checked first: the authority must not widen what the panel claims.
  const { ctx, notifications } = buildWithAuthority({
    authority: 'panel',
    reason: 'harness-not-visible',
    fresh: [],
  })
  const outcome = await ctx.dispatch('approval/request', { agent: { id: 'session-other' }, toolName: 'bash' })
  assert.equal(outcome, 'unavailable')
  assert.deepEqual(notifications, [])
})

test('a hint that cannot be delivered does not fail the request', async () => {
  // The hint is advisory. If the peer is gone the question still has to reach the
  // answerer that can take it, and a failed courtesy must not become a failed turn.
  const ctx = fakeContext()
  const notifications = []
  const interactions = new Interactions({
    ctx,
    ownedSessionIds: () => ['session-owned'],
    // The panel is showing the conversation the request belongs to, which is the case these
    // tests are about; the mismatch has its own test below.
    panelSession: () => 'session-owned',
    authority: () => ({ authority: 'panel', reason: 'harness-not-visible', fresh: [] }),
    canAnswer: () => false,
    notify: async (method, params) => {
      if (method === 'interaction/hint') throw new Error('peer went away')
      notifications.push({ method, params })
    },
    log: recordingLogger(),
  })
  interactions.register()
  lastInteractions = interactions
  const outcome = await ctx.dispatch(
    'user-questions/request',
    { agent: { id: 'session-owned' }, questions: [{ id: 'q1', question: 'x' }] },
  )
  assert.equal(outcome, 'unavailable', 'the request still reached the next answerer')
})

/** Wait for one protocol notification to reach the peer. */
async function waitForNotification(notifications, method) {
  for (let attempt = 0; attempt < 200; attempt += 1) {
    const found = notifications.find(entry => entry.method === method)
    if (found !== undefined) return found
    await new Promise(resolve => setTimeout(resolve, 5))
  }
  throw new Error(`the ${method} notification was never published`)
}


test('a peer that declares it can answer questions gets them published and answered', async () => {
  // The claim path for questions stays live for the build that grows the widget,
  // which is why it is exercised here with a peer that declares the capability.
  const { ctx, notifications } = buildWithAuthority(
    { authority: 'panel', reason: 'harness-not-visible', fresh: [] },
    () => true,
  )
  const pending = ctx.dispatch(
    'user-questions/request',
    { agent: { id: 'session-owned' }, questions: [{ id: 'q1', question: 'which?' }] },
  )
  // No hint: a hint means "this went somewhere else", and here the panel holds it.
  assert.equal(
    notifications.filter(entry => entry.method === 'interaction/hint').length,
    0,
    'a claimed request is not announced as a hand-off',
  )
  const open = await waitForNotification(notifications, 'interaction/open')
  interactionsAnswer(ctx, open.params.interactionId, {
    kind: 'question',
    answers: [{ id: 'q1', selected: ['a'] }],
  })
  assert.deepEqual(await pending, { answers: [{ id: 'q1', selected: ['a'] }] })
})

test('a claimed approval stays silent while the window is hidden', async () => {
  // Nothing to point at, and the card is in front of the user: a hint would only
  // add noise.
  const { ctx, notifications } = buildWithAuthority({
    authority: 'panel',
    reason: 'harness-not-visible',
    fresh: [],
  })
  const pending = ctx.dispatch('approval/request', { agent: { id: 'session-owned' }, toolName: 'bash' })
  const open = await waitForNotification(notifications, 'interaction/open')
  assert.equal(notifications.filter(entry => entry.method === 'interaction/hint').length, 0)
  interactionsAnswer(ctx, open.params.interactionId, { kind: 'approval', outcome: 'allowed-once' })
  assert.equal(await pending, 'allowed-once')
})

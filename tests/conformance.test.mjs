/**
 * Conformance against the installed DeepSeek Harness contract.
 *
 * The plugin calls `sessionController` directly, so the payloads it produces must
 * match the shapes the running Harness validates. Those validators are generated
 * into the app's `typert.host.js`; `tests/fixtures/extract-harness-contract.mjs`
 * copies that one file into `.tooling/conformance/`, and this test validates the
 * adapter's **real** outgoing payloads against it.
 *
 * This is the strongest verification available without a running dsh: it cannot
 * prove the Harness behaves as documented, but it does prove that every request
 * this plugin sends would pass the Harness' own parameter validation, and that
 * every response shape it assumes is the response shape the Harness declares.
 *
 * The test skips itself when the contract has not been extracted, so a fresh
 * clone can run the rest of the suite without a Harness installation.
 */

import assert from 'node:assert/strict'
import { existsSync } from 'node:fs'
import { test } from 'node:test'
import { join } from 'node:path'

import { loadModule, repoRoot } from './helpers.mjs'

const CONTRACT = join(repoRoot, '.tooling/conformance/typert.host.js')
const hasContract = existsSync(CONTRACT)
const skip = hasContract ? false : 'run `node tests/fixtures/extract-harness-contract.mjs` first (needs DeepSeek Harness installed)'

/** The generated manifest, loaded once. */
const contract = hasContract ? (await import(CONTRACT)).TYPERT : undefined

/** Find the invocation descriptor for one Remote method. */
function invocation(method) {
  const found = contract.invocations.find(entry => entry.id.endsWith(`#${method}`))
  assert.ok(found !== undefined, `the Harness contract has no ${method}`)
  return found
}

/** Validate one value against a Remote method's parameter codec. */
function checkParameter(method, value) {
  const descriptor = invocation(method)
  const codec = descriptor.parameters[0].codec.create()
  const result = codec.safeParse(value)
  return result.success ? { ok: true } : { ok: false, issues: result.error.issues.map(issue => `${issue.path.join('.')}: ${issue.message}`) }
}

/** Validate one value against a Remote method's result codec. */
function checkResult(method, value) {
  const descriptor = invocation(method)
  const codec = descriptor.result.create()
  const result = codec.safeParse(value)
  return result.success ? { ok: true } : { ok: false, issues: result.error.issues.map(issue => `${issue.path.join('.')}: ${issue.message}`) }
}

/**
 * Build the real adapter over a capturing fake `sessionController`.
 *
 * The fake records exactly what the adapter sends and replies with the shapes
 * the Harness declares, so the assertions below are about the adapter's own
 * behaviour rather than about the fake.
 */
async function buildAdapter(overrides = {}) {
  const { createHarnessFromContext } = await loadModule('harness/adapter.js')
  const calls = []
  const controller = {
    async list(request, signal) {
      calls.push({ method: 'session/list', request })
      assert.ok(signal instanceof AbortSignal, 'list must pass an AbortSignal')
      // The declared SessionSummary requires `agentAvailable` on every row; the
      // adapter reads it defensively, but the fixture must describe the real shape.
      return { items: [{ sessionId: 'session-1', agentAvailable: true, updatedAt: 1, running: false, blank: true, cwd: '/tmp/ws' }] }
    },
    async create(request) {
      calls.push({ method: 'session/create', request })
      return { sessionId: 'session-created' }
    },
    async prompt(request, signal) {
      calls.push({ method: 'session/prompt', request })
      assert.ok(signal instanceof AbortSignal, 'prompt must pass an AbortSignal')
      return { accepted: true }
    },
    cancel(request) {
      calls.push({ method: 'session/cancel', request })
      return { accepted: true }
    },
    async inspect(sessionId, signal) {
      calls.push({ method: 'session/inspect', sessionId })
      assert.ok(signal instanceof AbortSignal, 'inspect must pass an AbortSignal')
      // Two committed events, so the cursor is 1.
      return { events: [{ seq: 0 }, { seq: 1 }] }
    },
    async page(request, signal) {
      calls.push({ method: 'session/page', request })
      assert.ok(signal instanceof AbortSignal, 'page must pass an AbortSignal')
      return { records: [{ type: 'event', event: { type: 'turn/start', seq: 1, time: 1, data: { turn: 1 } } }], hasMore: false }
    },
    follow(request, signal) {
      calls.push({ method: 'session/follow', request, signal })
      const frames = overrides.frames ?? [
        {
          type: 'snapshot',
          header: { version: 4, id: 'session-1', createdAt: 1, isSeeded: false },
          cursor: 0,
          records: [],
          hasMore: false,
          projections: { asOfSeq: 0, values: {} },
        },
        { type: 'event', event: { type: 'turn/start', seq: 1, time: 2, data: { turn: 1 } } },
      ]
      return {
        async *[Symbol.asyncIterator]() {
          for (const frame of frames) yield frame
          // Stay open, like a real subscription, until the caller aborts.
          while (!signal.aborted) await new Promise(resolve => setTimeout(resolve, 5))
        },
      }
    },
  }
  const registry = { list: () => [], get: () => undefined }
  const services = {
    sessionController: controller,
    workspaceRegistry: registry,
    ...(overrides.services ?? {}),
  }
  const logger = { warn: () => {}, debug: () => {} }
  const built = createHarnessFromContext(name => services[name], logger)
  assert.ok(built.harness !== undefined, 'the adapter must build with the controller present')
  return { harness: built.harness, calls, controller }
}

test('the contract covers every method this plugin calls', { skip }, () => {
  for (const method of ['session/create', 'session/list', 'session/page', 'session/prompt', 'session/cancel', 'session/follow']) {
    const descriptor = invocation(method)
    assert.equal(descriptor.service, 'sessionController', `${method} belongs to the expected service`)
    assert.equal(descriptor.invocation.kind, 'direct', `${method} is callable in-process`)
  }
})

test('the create request the adapter sends passes Harness validation', { skip }, async () => {
  const { harness, calls } = await buildAdapter()
  await harness.createSession('w1')
  const call = calls.find(entry => entry.method === 'session/create')
  assert.deepEqual(call.request, { workspaceId: 'w1' }, 'only the workspace id is sent; cwd would be a conflict')
  const checked = checkParameter('session/create', call.request)
  assert.ok(checked.ok, `Harness rejected the create request: ${JSON.stringify(checked.issues)}`)
})

test('the prompt request the adapter sends passes Harness validation', { skip }, async () => {
  const { harness, calls } = await buildAdapter()
  await harness.prompt('session-1', 'req-1', 'hello 世界')
  const call = calls.find(entry => entry.method === 'session/prompt')
  assert.deepEqual(call.request, {
    requestId: 'req-1',
    sessionId: 'session-1',
    mode: 'queue',
    content: [{ type: 'text', text: 'hello 世界' }],
  })
  const checked = checkParameter('session/prompt', call.request)
  assert.ok(checked.ok, `Harness rejected the prompt request: ${JSON.stringify(checked.issues)}`)
})

test('the cancel request the adapter sends passes Harness validation', { skip }, async () => {
  const { harness, calls } = await buildAdapter()
  await harness.cancel('session-1')
  const call = calls.find(entry => entry.method === 'session/cancel')
  assert.deepEqual(call.request, { sessionId: 'session-1' })
  const checked = checkParameter('session/cancel', call.request)
  assert.ok(checked.ok, `Harness rejected the cancel request: ${JSON.stringify(checked.issues)}`)
})

test('the list request the adapter sends passes Harness validation', { skip }, async () => {
  const { harness, calls } = await buildAdapter()
  await harness.listSessions()
  const call = calls.find(entry => entry.method === 'session/list')
  const checked = checkParameter('session/list', call.request)
  assert.ok(checked.ok, `Harness rejected the list request: ${JSON.stringify(checked.issues)}`)
})

test('the cursor the adapter reads is the last committed sequence', { skip }, async () => {
  const { harness } = await buildAdapter()
  assert.equal(await harness.cursor('session-1'), 1)
})

test('the page request the adapter sends passes Harness validation', { skip }, async () => {
  const { harness, calls } = await buildAdapter()
  const throughSeq = await harness.cursor('session-1')
  await harness.pageHistory('session-1', throughSeq, 42)
  const call = calls.find(entry => entry.method === 'session/page')
  assert.deepEqual(call.request, {
    address: { kind: 'session', sessionId: 'session-1' },
    // A real sequence, at or below the session cursor. Harness rejects anything
    // past the cursor, which is what an invented maximum bound would be.
    throughSeq: 1,
    beforeSeq: 42,
    maxMessages: 50,
  })
  const checked = checkParameter('session/page', call.request)
  assert.ok(checked.ok, `Harness rejected the page request: ${JSON.stringify(checked.issues)}`)
})

test('the follow request the adapter sends passes Harness validation', { skip }, async () => {
  const { harness, calls } = await buildAdapter()
  const handle = harness.follow('session-1', () => {})
  await new Promise(resolve => setTimeout(resolve, 20))
  handle.close()
  await handle.done
  const call = calls.find(entry => entry.method === 'session/follow')
  assert.deepEqual(call.request, {
    address: { kind: 'session', sessionId: 'session-1' },
    assistantStream: true,
    maxMessages: 50,
  })
  const checked = checkParameter('session/follow', call.request)
  assert.ok(checked.ok, `Harness rejected the follow request: ${JSON.stringify(checked.issues)}`)
})

test('the frames the adapter consumes are frames the Harness declares', { skip }, async () => {
  // A snapshot and a durable event, in the exact shape the 0.2.0-rc.2 follow
  // stream declares. If the Harness changes these, the adapter's projection is
  // what has to change — this test is where that shows up.
  const frames = [
    {
      type: 'snapshot',
      header: { version: 4, id: 'session-1', createdAt: 1, isSeeded: false, cwd: '/tmp/ws' },
      cursor: 7,
      records: [{ type: 'event', event: { type: 'turn/start', seq: 7, time: 3, data: { turn: 1 } } }],
      hasMore: false,
      projections: { asOfSeq: 7, values: {} },
      assistantStream: { revision: 0 },
    },
    { type: 'event', event: { type: 'assistant/message', seq: 8, time: 4, data: { turn: 1, step: 1, message: { id: 'm1', role: 'assistant', source: { kind: 'model', provider: 'p', model: 'm' }, content: [] }, stream: [] }, surfaceOp: 'append' } },
    { type: 'assistant-stream', frame: { type: 'start', attemptId: 'a1', revision: 0, startedAfterSeq: 7, turn: 1, step: 1 } },
  ]
  for (const frame of frames) {
    const checked = checkResult('session/follow', frame)
    assert.ok(checked.ok, `the Harness does not declare this frame: ${JSON.stringify(checked.issues)}\n${JSON.stringify(frame)}`)
  }

  // The adapter must project each of them rather than silently dropping them.
  const { projectFrame } = await loadModule('harness/adapter.js')
  assert.equal(projectFrame(frames[0]).type, 'snapshot')
  assert.equal(projectFrame(frames[1]).type, 'event')
  assert.equal(projectFrame(frames[1]).seq, 8)
  assert.equal(projectFrame(frames[2]).type, 'assistant-stream')
})

test('an unknown frame type is dropped rather than forwarded', { skip }, async () => {
  const { projectFrame } = await loadModule('harness/adapter.js')
  assert.equal(projectFrame({ type: 'something-new-from-a-nightly-build' }), undefined)
  assert.equal(projectFrame({ type: 'event' }), undefined, 'an event frame without an event is not projectable')
})

test('the responses the adapter assumes are the declared response shapes', { skip }, () => {
  // These are the exact values the adapter reads out of Harness replies; each is
  // validated against the declared result codec so a shape change is caught here
  // rather than as a mysterious `undefined` at runtime.
  const cases = [
    ['session/create', { sessionId: 'session-1' }],
    ['session/prompt', { accepted: true }],
    ['session/cancel', { accepted: true }],
    ['session/list', { items: [{ sessionId: 's1', agentAvailable: false, updatedAt: 1, running: false, blank: true }] }],
    ['session/page', { records: [], hasMore: false }],
  ]
  for (const [method, value] of cases) {
    const checked = checkResult(method, value)
    assert.ok(checked.ok, `${method} result mismatch: ${JSON.stringify(checked.issues)}`)
  }
})

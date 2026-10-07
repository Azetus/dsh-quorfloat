// The protocol's method table, and the one failure it is prone to.
//
// A method is answered only if three things agree: the router's dispatch, the method list the
// supervisor registers channel handlers from, and the peer's own idea of what it may ask. The first
// two were written twice, drifted the first time a method was added, and produced the worst possible
// symptom — `unsupported method: session/options` for a method that was implemented, in the panel's
// own settings menu, with nothing in the log to say which side was wrong.
//
// So the list lives in one place (`METHODS`, in `src/bridge/router.ts`) and these tests walk it: every
// entry must be answerable, and a method that is not in it must not be answerable either.
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { loadModule } from './helpers.mjs'

const { HostRouter, METHODS } = await loadModule('bridge/router.js')

/** A router host whose every method returns a recognisable value. */
function stubHost() {
  return {
    config: () => ({}),
    hostVersion: () => 'test',
    channelSessionId: () => 'channel-1',
    listWorkspaces: async () => [],
    listSessions: async () => ({ items: [] }),
    createSession: async () => ({ sessionId: 'session-1' }),
    attachSession: async () => ({ sessionId: 'session-1' }),
    readHistory: async () => ({ records: [], hasMore: false }),
    readOptions: async () => ({ groups: [], permissions: [] }),
    selectModel: async () => undefined,
    setPermission: async () => undefined,
    prompt: async () => ({ accepted: true }),
    cancel: async () => ({ accepted: true }),
    answerInteraction: async () => ({ accepted: true }),
    reportPresence: async () => ({ accepted: true }),
    diagnostics: () => ({}),
  }
}

/** Parameters that get past each method's validation, so the dispatch itself is what is tested. */
const PARAMS = {
  hello: { protocol: 'quorfloat/1', quorfloatVersion: '0.0.1', platform: 'darwin', arch: 'arm64' },
  'window/visibility': { visible: true },
  'session/create': { workspaceId: 'ws-1' },
  'session/attach': { sessionId: 'session-1' },
  'session/history': { sessionId: 'session-1' },
  'session/prompt': { sessionId: 'session-1', requestId: 'r-1', text: 'hi' },
  'session/cancel': { sessionId: 'session-1' },
  'session/options': {},
  'session/select': { sessionId: 'session-1', provider: 'deepseek', model: 'v41' },
  'session/permission': { sessionId: 'session-1', value: 'read-only' },
  'interaction/answer': { interactionId: 'i-1', answer: {} },
  'presence/report': { surface: 'panel', visible: true, focused: true, seq: 1 },
}

test('every method in the table is answered by the router', async () => {
  // The test that would have caught the drift: a method listed but not dispatched fails here, in the
  // suite, rather than in the panel with an error nobody can place.
  const router = new HostRouter(stubHost())
  for (const method of METHODS) {
    try {
      await router.handle(method, PARAMS[method] ?? {})
    } catch (error) {
      assert.notEqual(
        error?.message,
        `unsupported method: ${method}`,
        `${method} is in METHODS but the router does not answer it`,
      )
      // Any other failure is this test's parameters being incomplete, which is a test problem: the
      // validation ran, so the method is dispatched.
    }
  }
})

test('a method that is not in the table is refused', async () => {
  // The other direction: the table is the whole protocol, so a peer asking for something else is told
  // so rather than reaching a handler.
  const router = new HostRouter(stubHost())
  await assert.rejects(
    () => router.handle('session/invented', {}),
    (error) => {
      assert.equal(error.code, 'unavailable')
      assert.match(error.message, /unsupported method/)
      return true
    },
  )
})

test('the table names no method twice', () => {
  assert.equal(new Set(METHODS).size, METHODS.length, 'a duplicate would register one handler twice')
})

test('the new session methods are in the table', () => {
  // Named explicitly so a future edit that removes one is a deliberate act rather than an oversight:
  // these three are what the panel's footer is built on.
  for (const method of ['session/options', 'session/select', 'session/permission']) {
    assert.ok(METHODS.includes(method), `${method} must be answerable`)
  }
})

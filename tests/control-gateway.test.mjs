/**
 * Tests for the browser-facing control endpoints (`quorfloat/status`, `quorfloat/panel`).
 *
 * The sibling file for the presence endpoint exists because the gateway's
 * source-mode descriptor reads parameter names off the registered method's
 * signature and then requires the request's `args` keys to match them one for
 * one. These two endpoints are exposed the same way, so they carry the same
 * failure mode — plus one of their own: an action word is a process-level
 * effect, so an unknown one must be refused here rather than reach the
 * supervisor.
 *
 * The second half of the file drives the real `QuorfloatSupervisor` over the
 * mock sidecar. The pure mapping could be asserted against hand-written
 * snapshots, but that would only prove the test agrees with itself; the pid,
 * the handshake flag and the restart behaviour are facts about a live process,
 * so they are read from one.
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'

import { buildSupervisor, loadModule, waitFor } from './helpers.mjs'

const {
  registerControlGateway,
  buildStatus,
  validatePanelAction,
  CONTROL_SERVICE_KEY,
  CONTROL_NAMESPACE,
  STATUS_ENDPOINT_METHOD,
  PANEL_ENDPOINT_METHOD,
} = await loadModule('host/control-gateway.js')

const { REMOTE_METHOD_DESCRIPTOR } = await loadModule('host/remote-gateway.js')

/** A context that records what was registered and hands back a disposer. */
function fakeContext() {
  const provided = []
  return {
    provided,
    reflect: {
      provide(name, value) {
        provided.push({ name, value })
        return () => {}
      },
    },
  }
}

/** A logger that swallows everything. */
const silentLog = { info: () => {}, warn: () => {}, debug: () => {}, error: () => {} }

/** The supervisor snapshot fields `buildStatus` reads, with honest defaults. */
const STOPPED_SNAPSHOT = {
  state: 'stopped',
  pid: undefined,
  restarts: 0,
  restartExhausted: false,
  handshaken: false,
  panelVisible: false,
  lastError: undefined,
}
const RUNNING_SNAPSHOT = {
  state: 'running',
  pid: 4242,
  restarts: 2,
  restartExhausted: false,
  handshaken: true,
  panelVisible: true,
  lastError: undefined,
}

/**
 * A host that records actions and answers the snapshot each one settles into.
 *
 * @param options.running - force the reported state regardless of the actions.
 */
function fakeHost({ running = false } = {}) {
  let current = running ? RUNNING_SNAPSHOT : STOPPED_SNAPSHOT
  const calls = []
  return {
    calls,
    snapshot: () => current,
    async start() {
      calls.push('start')
      current = RUNNING_SNAPSHOT
      return current
    },
    async stop() {
      calls.push('stop')
      current = STOPPED_SNAPSHOT
      return { exited: true, code: 0, signal: null, escalated: false }
    },
    async restart() {
      calls.push('restart')
      current = RUNNING_SNAPSHOT
      return current
    },
  }
}

/** Register the control gateway and return what the gateway resolves. */
function register(host = fakeHost()) {
  const ctx = fakeContext()
  const dispose = registerControlGateway({ ctx, host, log: silentLog })
  assert.equal(typeof dispose, 'function')
  const service = ctx.provided[0].value
  return { service, prototype: Object.getPrototypeOf(service), host, ctx }
}

test('the service registers under its key with a self-consistent binding', () => {
  const { service, ctx } = register()
  assert.equal(ctx.provided[0].name, CONTROL_SERVICE_KEY)
  // `validateBinding` in the gateway only checks these three fields, and the
  // `service` field must be the same object the gateway resolved.
  assert.equal(service.typertRemote.namespace, CONTROL_NAMESPACE)
  assert.equal(service.typertRemote.serviceKey, CONTROL_SERVICE_KEY)
  assert.equal(service.typertRemote.service, service)
})

test('both methods carry the marker the gateway discovers', () => {
  // Namespaced with the presence endpoint on purpose: the gateway addresses
  // `quorfloat/status` and finds it by scanning services whose `typertRemote`
  // namespace matches, so the marker is what makes the endpoint reachable.
  const { prototype } = register()
  const descriptor = prototype[REMOTE_METHOD_DESCRIPTOR]
  assert.equal(descriptor.version, 1, 'the version the gateway reads')
  assert.deepEqual(descriptor.methods, [
    { method: 'status', invocation: { kind: 'direct' } },
    { method: 'panel', invocation: { kind: 'direct' } },
  ])
})

test('the signatures declare the wire fields the browser half sends', () => {
  // THE regression this pair is prone to. The gateway builds its `args`
  // expectations from these names, so collapsing `panel(action)` into
  // `panel(payload)` would make the browser half's natural
  // `args: { action }` fail with `gateway/arguments-invalid`. `status` takes no
  // parameters at all, so its expectation is an empty `args: {}`.
  const names = fn =>
    fn
      .toString()
      .replace(/^[^(]*\(/, '')
      .replace(/\).*$/s, '')
      .split(',')
      .map(part => part.trim().replace(/:.*$/s, '').replace(/^this$/, '').trim())
      .filter(part => part !== '' && part !== '...')
  const { prototype } = register()
  assert.deepEqual(names(prototype[STATUS_ENDPOINT_METHOD]), [], 'status takes no parameters')
  assert.deepEqual(names(prototype[PANEL_ENDPOINT_METHOD]), ['action'], 'the one field panel sends')
})

test('status answers a running supervisor truthfully', () => {
  const { prototype, host } = register(fakeHost({ running: true }))
  assert.deepEqual(prototype[STATUS_ENDPOINT_METHOD](), {
    state: 'running',
    pid: 4242,
    restarts: 2,
    restartExhausted: false,
    seen: true,
    visible: true,
    lastError: null,
  })
  assert.deepEqual(prototype[STATUS_ENDPOINT_METHOD](), buildStatus(host.snapshot()))
})

test('status reports a stopped supervisor without inventing a pid', () => {
  // JSON has no `undefined`: a caller that renders `pid` must get `null`, and a
  // process that has never handshaken must not read as seen.
  const { prototype } = register()
  assert.deepEqual(prototype[STATUS_ENDPOINT_METHOD](), {
    state: 'stopped',
    pid: null,
    restarts: 0,
    restartExhausted: false,
    seen: false,
    visible: false,
    lastError: null,
  })
})

test('panel forwards each action and answers the state it settled into', async () => {
  for (const action of ['start', 'stop', 'restart']) {
    const host = fakeHost()
    const { prototype } = register(host)
    const result = await prototype[PANEL_ENDPOINT_METHOD](action)
    assert.deepEqual(host.calls, [action], `${action} reached the host exactly once`)
    assert.deepEqual(result, buildStatus(host.snapshot()), `${action} answered the post-action snapshot`)
  }
})

test('a missing or unknown action is refused before it reaches the host', async () => {
  // A closed set, because the effect is a process-level one no caller can take
  // back: anything that is not one of the three words must stop at validation.
  const host = fakeHost()
  const { prototype } = register(host)
  for (const action of ['reboot', '', 'STOP', 1, undefined, null, {}]) {
    await assert.rejects(
      () => prototype[PANEL_ENDPOINT_METHOD](action),
      error => {
        assert.ok(error instanceof TypeError, 'the same refusal shape as the presence endpoint')
        assert.match(error.message, /quorfloat\/panel: action must be start, stop or restart/)
        return true
      },
      `${JSON.stringify(action)} must be refused`,
    )
  }
  assert.deepEqual(host.calls, [], 'nothing reached the host')
  // And the validator itself is the one place that decision lives.
  assert.equal(validatePanelAction('restart'), 'restart')
  assert.throws(() => validatePanelAction('start '), /action must be start, stop or restart/)
})

test('status reads the real supervisor, running and then stopped', async () => {
  const { supervisor, restore } = await buildSupervisor({ mode: 'normal' })
  try {
    const { prototype } = register(supervisor)
    // Registered while stopped: an unwatched sidecar must not report a handshake
    // it never completed.
    assert.equal(prototype[STATUS_ENDPOINT_METHOD]().state, 'stopped')
    assert.equal(prototype[STATUS_ENDPOINT_METHOD]().seen, false)

    await supervisor.start()
    await waitFor('the mock peer to handshake', async () => supervisor.snapshot().state === 'running')
    const running = prototype[STATUS_ENDPOINT_METHOD]()
    assert.equal(running.state, 'running')
    assert.equal(running.pid, supervisor.snapshot().pid, 'the pid is the process the supervisor owns')
    assert.equal(typeof running.pid, 'number')
    assert.equal(running.seen, true, 'the handshake is what "seen" reports')
    assert.equal(running.visible, false, 'the panel has not reported a window')
    assert.equal(running.restarts, 0)
    assert.equal(running.restartExhausted, false)
    assert.equal(running.lastError, null)

    await supervisor.stop()
    const stopped = prototype[STATUS_ENDPOINT_METHOD]()
    assert.equal(stopped.state, 'stopped')
    assert.equal(stopped.pid, null, 'a stopped supervisor has no pid, not a stale one')
    assert.equal(stopped.seen, false)
  } finally {
    await supervisor.stop()
    restore()
  }
})

test('panel drives the real supervisor through start, restart and stop', async () => {
  const { supervisor, restore } = await buildSupervisor({ mode: 'normal' })
  try {
    const { prototype } = register(supervisor)

    const started = await prototype[PANEL_ENDPOINT_METHOD]('start')
    assert.equal(typeof started.pid, 'number', 'start spawned a process')
    await waitFor('the started peer to handshake', async () => supervisor.snapshot().state === 'running')
    const firstPid = started.pid

    const restarted = await prototype[PANEL_ENDPOINT_METHOD]('restart')
    assert.equal(typeof restarted.pid, 'number')
    assert.notEqual(restarted.pid, firstPid, 'restart replaced the process')
    await waitFor(
      'the replacement to handshake',
      async () => supervisor.snapshot().state === 'running' && supervisor.snapshot().pid === restarted.pid,
    )

    const stopped = await prototype[PANEL_ENDPOINT_METHOD]('stop')
    assert.equal(stopped.state, 'stopped')
    assert.equal(stopped.pid, null)
    assert.equal(supervisor.snapshot().state, 'stopped', 'the supervisor agrees, not just the endpoint')
  } finally {
    await supervisor.stop()
    restore()
  }
})

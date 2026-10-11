// Category 1 (cross-process interface): the `quorfloat/status` and `quorfloat/panel` endpoints the client calls.
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
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'

import { loadModule } from './helpers.mjs'

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


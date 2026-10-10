/**
 * Gateway service registration tests.
 *
 * The failure this file guards against: the browser half reaches the host and the
 * gateway finds the endpoint, but rejects the arguments with
 * `gateway/arguments-invalid: unexpected "surface", "visible", "focused", "seq", "at"`.
 *
 * The cause is easy to reintroduce: the gateway's source-mode descriptor reads
 * parameter names off the registered method's signature and then requires the
 * request's `args` keys to match them one for one. A method declared as
 * `reportPresence(payload)` therefore expects `args: { payload: {...} }`, while
 * every reader (and the browser half) naturally sends the report's own fields.
 * These tests pin the wire shape so that mistake fails here instead of in a live
 * page.
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'

import { loadModule } from './helpers.mjs'

const {
  registerPresenceGateway,
  validatePresenceReport,
  REMOTE_METHOD_DESCRIPTOR,
  PRESENCE_SERVICE_KEY,
} = await loadModule('host/presence-gateway.js')

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

/** Register the gateway and return what the service looks like to the gateway. */
function register(sink = () => ({ accepted: true })) {
  const ctx = fakeContext()
  const dispose = registerPresenceGateway({ ctx, sink, log: silentLog })
  assert.equal(typeof dispose, 'function')
  const service = ctx.provided[0].value
  const prototype = Object.getPrototypeOf(service)
  return { service, prototype, sink, ctx }
}

test('the service registers under its key with a self-consistent binding', () => {
  const { service, ctx } = register()
  assert.equal(ctx.provided[0].name, PRESENCE_SERVICE_KEY)
  // `validateBinding` in the gateway only checks these three fields, and the
  // `service` field must be the same object the gateway resolved.
  assert.equal(service.typertRemote.namespace, PRESENCE_SERVICE_KEY)
  assert.equal(service.typertRemote.serviceKey, PRESENCE_SERVICE_KEY)
  assert.equal(service.typertRemote.service, service)
})

test('the method carries the marker the gateway discovers', () => {
  const { prototype } = register()
  const descriptor = prototype[REMOTE_METHOD_DESCRIPTOR]
  assert.equal(descriptor.version, 1, 'the version the gateway reads')
  assert.deepEqual(descriptor.methods, [{ method: 'reportPresence', invocation: { kind: 'direct' } }])
})

test('the signature declares one parameter per wire field', () => {
  // THE regression test. The gateway builds `args` expectations from these names,
  // so collapsing them into a single object parameter breaks the live browser
  // half even though the router path (which object-checks `params`) still passes.
  const { prototype } = register()
  const names = prototype.reportPresence
    .toString()
    .replace(/^[^(]*\(/, '')
    .replace(/\).*$/s, '')
    .split(',')
    .map(part => part.trim().replace(/:.*$/s, '').replace(/^this$/, '').trim())
    .filter(part => part !== '' && part !== '...')
  assert.deepEqual(
    names,
    ['surface', 'visible', 'focused', 'seq', 'at'],
    'the wire fields the browser half sends, in order',
  )
})

test('a request shaped like the browser half sends is accepted', () => {
  const calls = []
  const { prototype } = register(report => {
    calls.push(report)
    return { accepted: true }
  })
  // Exactly what `src/client/index.ts` passes as `args`.
  const result = prototype.reportPresence('desktop', true, false, 4, 1791207320291)
  assert.deepEqual(result, { accepted: true })
  assert.deepEqual(calls, [{
    surface: 'desktop',
    visible: true,
    focused: false,
    seq: 4,
    at: 1791207320291,
  }])
})

test('an omitted timestamp is passed through as absent, not as undefined', () => {
  // The gateway may omit an SRC json field; the sink must not see a key holding
  // undefined, because `Object.hasOwn` checks downstream would then differ from
  // a genuinely absent field.
  const calls = []
  const { prototype } = register(report => {
    calls.push(report)
    return { accepted: true }
  })
  prototype.reportPresence('web', true, true, 1)
  assert.deepEqual(calls, [{ surface: 'web', visible: true, focused: true, seq: 1 }])
  assert.equal(Object.hasOwn(calls[0], 'at'), false)
})

test('a malformed report is rejected with a legible reason', () => {
  const { prototype } = register()
  assert.throws(() => prototype.reportPresence('tablet', true, true, 1), /surface must be "desktop" or "web"/)
  assert.throws(() => prototype.reportPresence('web', 'yes', true, 1), /visible must be a boolean/)
  assert.throws(() => prototype.reportPresence('web', true, undefined, 1), /focused must be a boolean/)
  assert.throws(() => prototype.reportPresence('web', true, true, 1.5), /seq must be an integer/)
  assert.throws(() => prototype.reportPresence('web', true, true, 1, Number.NaN), /at must be a finite number/)
})

test('the validator refuses the flat object the router passes as one parameter', () => {
  // Guards the two entry points against being confused for one another: the
  // router validates an object, the gateway spreads positional arguments.
  assert.deepEqual(validatePresenceReport({ surface: 'web', visible: false, focused: false, seq: 2 }), {
    surface: 'web',
    visible: false,
    focused: false,
    seq: 2,
  })
  assert.throws(() => validatePresenceReport(undefined), /params must be an object/)
})

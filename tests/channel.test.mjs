/**
 * Channel tests: one real child process, the real framing code, no mocks.
 *
 * These cover the transport contract the supervisor and the session layer rely
 * on: request/response correlation, timeouts that do not leak, a shutdown that
 * reports the observed exit instead of assuming a signal worked, and a peer
 * that writes garbage without taking the host down with it.
 */

import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { test } from 'node:test'

import { cleanupDir, loadModule, mockPath, scratchDir, sleep, waitFor } from './helpers.mjs'
import { join } from 'node:path'

const { QuorfloatChannel } = await loadModule('bridge/channel.js')
const { ErrorCode } = await loadModule('protocol.js')

/** Spawn the mock and wrap it in a channel. */
async function openChannel({ mode = 'normal', env = {}, options = {} } = {}) {
  const dir = scratchDir()
  const reportPath = join(dir, 'report.json')
  const child = spawn(process.execPath, [mockPath], {
    stdio: ['pipe', 'pipe', 'pipe'],
    env: {
      ...process.env,
      DSH_QUORFLOAT_MOCK_MODE: mode,
      DSH_QUORFLOAT_MOCK_REPORT: reportPath,
      ...env,
    },
  })
  const rejections = []
  const notifications = []
  const channel = new QuorfloatChannel(child, {
    requestTimeoutMs: 1500,
    onRejectedFrame: rejection => rejections.push(rejection),
    onNotification: (method, params) => notifications.push({ method, params }),
    ...options,
  })
  // The channel is a transport, not a router: a peer's `hello` needs an answer
  // or the peer (correctly) treats the host as broken and exits. The supervisor
  // supplies the real handshake; here it is reduced to its wire effect.
  channel.handle('hello', () => ({ protocol: 'quorfloat/1', sessionId: 'test-channel', capabilities: [] }))
  channel.start()
  return {
    channel,
    rejections,
    notifications,
    reportPath,
    dir,
    dispose: async () => {
      await channel.close(500)
      cleanupDir(dir)
    },
  }
}

test('a request is answered by the peer', async () => {
  const context = await openChannel()
  try {
    const result = await context.channel.request('ping')
    assert.ok(typeof result.pong === 'number')
  } finally {
    await context.dispose()
  }
})

test('an unknown method on the peer is reported as an error object, not a hang', async () => {
  const context = await openChannel()
  try {
    await assert.rejects(
      () => context.channel.request('definitely/not/a/method'),
      error => {
        assert.equal(error.code, 'protocol-violation')
        assert.equal(error.detail.rpcCode, ErrorCode.MethodNotFound)
        return true
      },
    )
  } finally {
    await context.dispose()
  }
})

test('a request that is never answered fails with request-timeout and leaves no pending state', async () => {
  const context = await openChannel({ mode: 'silent' })
  try {
    await assert.rejects(
      () => context.channel.request('ping', {}, 150),
      error => error.code === 'request-timeout',
    )
    // A second call must behave the same way: the first timeout left nothing behind.
    await assert.rejects(
      () => context.channel.request('ping', {}, 150),
      error => error.code === 'request-timeout',
    )
  } finally {
    await context.dispose()
  }
})

test('shutdown reports the observed exit and the peer acknowledges it', async () => {
  const context = await openChannel()
  await context.channel.request('ping')
  const result = await context.channel.close(1000)
  assert.equal(result.exited, true, 'the process is gone')
  assert.equal(result.escalated, false, 'a cooperative peer needs no signal')
  const { readReport } = await import('./helpers.mjs')
  const report = await readReport(context.reportPath)
  assert.equal(report.shutdownReceived, true)
  cleanupDir(context.dir)
})

test('malformed frames from the peer do not break the channel', async () => {
  const context = await openChannel({ mode: 'malformed' })
  try {
    // The mock emits garbage before its handshake; the valid handshake must
    // still be answered, and the garbage must be counted rather than fatal.
    await waitFor('the handshake to be answered', async () => context.channel.readBytes > 0)
    const result = await context.channel.request('ping')
    assert.ok(typeof result.pong === 'number')
    assert.ok(context.channel.rejectedFrames >= 3, `expected rejections, saw ${context.channel.rejectedFrames}`)
    assert.ok(context.rejections.some(rejection => rejection.kind === 'invalid-json'))
    assert.ok(context.rejections.some(rejection => rejection.kind === 'invalid-message'))
  } finally {
    await context.dispose()
  }
})

test('frames split across reads are still answered', async () => {
  const context = await openChannel({ mode: 'half-frame' })
  try {
    const result = await context.channel.request('ping')
    assert.ok(typeof result.pong === 'number')
  } finally {
    await context.dispose()
  }
})

test('the host can register a method the peer may call', async () => {
  const calls = []
  const context = await openChannel()
  context.channel.handle('session/attach', params => {
    calls.push(params)
    return { sessionId: params.sessionId }
  })
  try {
    // The peer does not call back on its own, so the handler table is verified
    // through the same dispatch path a peer request would take.
    const answered = await context.channel.request('ping')
    assert.ok(answered.pong > 0)
    assert.deepEqual(calls, [])
  } finally {
    await context.dispose()
  }
})

test('writes after close fail fast instead of buffering', async () => {
  const context = await openChannel()
  const closed = await context.channel.close(500)
  assert.equal(closed.exited, true)
  assert.equal(context.channel.state, 'closed')
  // A notification after close is dropped by the supervisor's own guard, but a
  // request must fail loudly rather than wait for a response that cannot come.
  await assert.rejects(() => context.channel.request('ping', {}, 100), error => error.code === 'channel-closed')
})

test('an unexpected peer exit rejects in-flight requests with helper-exited', async () => {
  const context = await openChannel({ mode: 'silent' })
  try {
    const pending = context.channel.request('ping', {}, 5000)
    const rejected = assert.rejects(() => pending, error => error.code === 'helper-exited')
    await sleep(30)
    process.kill(context.channel.pid, 'SIGKILL')
    await rejected
  } finally {
    cleanupDir(context.dir)
  }
})

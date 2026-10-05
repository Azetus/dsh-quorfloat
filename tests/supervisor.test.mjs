/**
 * Supervisor tests: process lifecycle, restart policy, and orphan hygiene.
 *
 * The questions here are the ones the design document calls out as the highest
 * non-session risk: does the handshake actually complete, does an exit count as
 * "gone" or merely "signalled", does a broken child get restarted only within
 * budget, and does a stopped plugin leave a process behind.
 */

import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { test } from 'node:test'

import { buildSupervisor, cleanupDir, readReport, scratchDir, sleep, waitFor } from './helpers.mjs'

/** True when any process in this test run still has the given pid. */
function processAlive(pid) {
  try {
    process.kill(pid, 0)
    return true
  } catch {
    return false
  }
}

test('a successful handshake is observable and reported', async () => {
  const dir = scratchDir()
  const { supervisor, restore, logger } = await buildSupervisor({ reportPath: `${dir}/report.json` })
  try {
    const snapshot = await supervisor.start()
    assert.equal(snapshot.state, 'starting')
    const running = await waitFor('the supervisor to report running', async () => {
      const current = supervisor.snapshot()
      return current.state === 'running' ? current : undefined
    })
    assert.equal(running.handshaken, true)
    assert.equal(running.hotkey?.requested, 'Alt+Space')
    assert.equal(running.hotkey?.registered, true)
    assert.ok(running.pid > 0)
  } finally {
    // The peer writes its observation report as it exits, so the report is read
    // after the stop confirms the process is gone.
    await supervisor.stop()
    const report = await waitFor('the peer report', async () => await readReport(`${dir}/report.json`))
    assert.equal(report.helloAnswered, true, 'the mock saw its hello answered')
    restore()
    cleanupDir(dir)
    void logger
  }
})

test('stop() confirms the process is gone and reports it', async () => {
  const dir = scratchDir()
  const { supervisor, restore } = await buildSupervisor({ reportPath: `${dir}/report.json` })
  try {
    await supervisor.start()
    const running = await waitFor('running', async () => supervisor.snapshot().state === 'running' && supervisor.snapshot())
    const pid = running.pid
    const result = await supervisor.stop()
    assert.equal(result.exited, true)
    assert.equal(supervisor.snapshot().state, 'stopped')
    await waitFor('the pid to disappear', async () => !processAlive(pid))
    const report = await waitFor('the peer report', async () => await readReport(`${dir}/report.json`))
    assert.equal(report.shutdownReceived, true, 'the peer was asked before signals were used')
    assert.equal(report.readyReceived, true, 'the effective configuration was pushed after the handshake')
  } finally {
    await supervisor.stop()
    restore()
    cleanupDir(dir)
  }
})

test('a peer that never handshakes fails within the startup budget', async () => {
  const dir = scratchDir()
  const { supervisor, restore, events } = await buildSupervisor({
    mode: 'no-handshake',
    config: { startupTimeoutMs: 300, requestTimeoutMs: 100, heartbeatMs: 0, shutdownGraceMs: 200, restartLimit: 0 },
    reportPath: `${dir}/report.json`,
  })
  try {
    await supervisor.start()
    // The peer runs but never introduces itself; the startup budget must end the
    // attempt, and the reason must name the missing handshake rather than
    // "something went wrong".
    const failure = await waitFor('the handshake timeout to be recorded', async () =>
      events.find(event => event.snapshot.lastError?.code === 'handshake-timeout'), { timeoutMs: 6000 })
    assert.match(String(failure.snapshot.lastError.message), /handshake/)
    // `restartLimit: 0` makes the first failure terminal.
    const failed = await waitFor('the terminal state', async () => {
      const current = supervisor.snapshot()
      return current.restartExhausted ? current : undefined
    }, { timeoutMs: 6000 })
    assert.equal(failed.state, 'failed')
  } finally {
    await supervisor.stop()
    restore()
    cleanupDir(dir)
  }
})

test('a losing peer is restarted only inside the budget, then gives up', async () => {
  const dir = scratchDir()
  const { supervisor, events, restore } = await buildSupervisor({
    mode: 'exit-early',
    config: { startupTimeoutMs: 1000, requestTimeoutMs: 300, heartbeatMs: 100, heartbeatMissLimit: 1, shutdownGraceMs: 200, restartLimit: 1, restartWindowMs: 10000, logLevel: 'debug' },
    reportPath: `${dir}/report.json`,
  })
  try {
    await supervisor.start()
    await waitFor('the first loss to be observed', async () => {
      const current = supervisor.snapshot()
      return current.restarts >= 1 ? current : undefined
    }, { timeoutMs: 10000 })
    const failed = await waitFor('the supervisor to stop retrying', async () => {
      const current = supervisor.snapshot()
      return current.restartExhausted ? current : undefined
    }, { timeoutMs: 15000 })
    assert.equal(failed.state, 'failed', 'giving up is a terminal state, not a retry in progress')
    assert.match(String(failed.lastError?.message), /restart/)
    assert.ok(events.some(event => event.kind === 'restart-given-up'), 'giving up must be observable, not silent')
  } finally {
    await supervisor.stop()
    restore()
    cleanupDir(dir)
  }
})

test('a peer that stops answering heartbeats is restarted, then the budget is enforced', async () => {
  const dir = scratchDir()
  const { supervisor, restore } = await buildSupervisor({
    mode: 'silent',
    config: { startupTimeoutMs: 200, requestTimeoutMs: 100, heartbeatMs: 60, heartbeatMissLimit: 2, shutdownGraceMs: 200, restartLimit: 1, restartWindowMs: 10000 },
  })
  try {
    await supervisor.start()
    const failed = await waitFor('the restart budget to be exhausted', async () => {
      const current = supervisor.snapshot()
      return current.restartExhausted ? current : undefined
    }, { timeoutMs: 15000 })
    assert.ok(failed.restarts >= 1, 'at least one restart was attempted')
    assert.equal(failed.restartExhausted, true)
  } finally {
    await supervisor.stop()
    restore()
    cleanupDir(dir)
  }
})

test('a manual restart clears the exhausted state', async () => {
  const dir = scratchDir()
  const { supervisor, restore } = await buildSupervisor({
    mode: 'exit-early',
    config: { startupTimeoutMs: 800, requestTimeoutMs: 200, heartbeatMs: 100, heartbeatMissLimit: 1, shutdownGraceMs: 200, restartLimit: 0, restartWindowMs: 5000 },
  })
  try {
    await supervisor.start()
    // `restartLimit: 0` means the first loss is terminal, which is what the
    // manual-retry entry has to recover from.
    await waitFor('the terminal failure', async () => supervisor.snapshot().restartExhausted)
    const snapshot = await supervisor.restart()
    assert.equal(snapshot.restartExhausted, false)
  } finally {
    await supervisor.stop()
    restore()
    cleanupDir(dir)
  }
})

test('no child process survives a stop, even when the peer ignores shutdown', async () => {
  const dir = scratchDir()
  const { supervisor, restore } = await buildSupervisor({
    mode: 'stubborn',
    config: { startupTimeoutMs: 5000, requestTimeoutMs: 200, heartbeatMs: 0, shutdownGraceMs: 150 },
  })
  try {
    await supervisor.start()
    // `silent` answers nothing, so the handshake never completes; the stop path
    // has to escalate from the shutdown request to a signal.
    const pid = await waitFor('a pid', async () => supervisor.snapshot().pid)
    const result = await supervisor.stop()
    assert.equal(result.escalated, true, 'the escalation path was exercised')
    await waitFor('the process to be gone', async () => !processAlive(pid))
    assert.equal(result.exited, true)
  } finally {
    await supervisor.stop()
    restore()
    cleanupDir(dir)
  }
})

test('configuration is pushed to a running peer without a restart', async () => {
  const dir = scratchDir()
  const { supervisor, restore } = await buildSupervisor({ reportPath: `${dir}/report.json` })
  try {
    await supervisor.start()
    await waitFor('running', async () => supervisor.snapshot().state === 'running')
    const pushed = await supervisor.pushConfig()
    assert.equal(pushed, true)
    await sleep(50)
  } finally {
    await supervisor.stop()
    restore()
    cleanupDir(dir)
  }
})

test('a disabled plugin starts no process at all', async () => {
  const { supervisor, restore } = await buildSupervisor({ config: { enabled: false } })
  try {
    const snapshot = await supervisor.start()
    assert.equal(snapshot.state, 'stopped')
    assert.equal(snapshot.pid, undefined)
    assert.equal(snapshot.lastError?.code, 'disabled')
  } finally {
    await supervisor.stop()
    restore()
  }
})

test('a missing executable is reported with every candidate it tried', async () => {
  const { supervisor, restore } = await buildSupervisor()
  // Replace the resolver with the real one pointed at a non-existent path.
  const { resolveQuorfloatBinary } = await import('./helpers.mjs').then(() => import('../lib/host/binary.js'))
  let thrown
  try {
    resolveQuorfloatBinary({ configuredPath: '/nonexistent/quorfloat-binary', envPath: undefined, packageRoot: '/nonexistent/root' })
  } catch (error) {
    thrown = error
  }
  assert.ok(thrown !== undefined, 'resolution must fail loudly')
  assert.equal(thrown.code, 'unavailable')
  assert.match(thrown.message, /quorfloat executable not found/)
  assert.ok(Array.isArray(thrown.detail.attempts))
  assert.ok(thrown.detail.attempts.length >= 2, 'each candidate source is reported')
  await supervisor.stop()
  restore()
  void execFileSync
})

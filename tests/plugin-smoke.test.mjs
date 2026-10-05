/**
 * Smoke test against the real Cordis framework.
 *
 * Everything else in `tests/` exercises one component with fakes. This file
 * answers the question the design document says must be answered before any UI
 * work: can this plugin be loaded by the actual plugin framework, reach its
 * services, spawn the subproject, complete a handshake, and be unloaded without
 * leaving a process behind?
 *
 * The services planted here are fakes, but the framework, the module loading,
 * the config validation, the spawn, the shutdown sequence, and the disposal path
 * are all real. Assertions read the plugin's own state object rather than its
 * log output, so a wording change cannot silently weaken a test.
 */

import assert from 'node:assert/strict'
import { existsSync, readFileSync } from 'node:fs'
import { test } from 'node:test'
import { join } from 'node:path'

import { cleanupDir, loadCordis, loadPlugin, mockPath, recordingLogger, scratchDir, waitFor } from './helpers.mjs'

const cordis = await loadCordis()
const pluginModule = await loadPlugin()

/** A fake `sessionController` covering the subset the plugin calls. */
function fakeSessionController() {
  const followRequests = []
  return {
    followRequests,
    async list() {
      return { items: [{ sessionId: 'session-a', updatedAt: 1, running: false, blank: true, cwd: '/tmp/ws' }] }
    },
    async create(request) {
      assert.equal(typeof request.workspaceId, 'string', 'create is always called with an explicit workspace')
      return { sessionId: 'session-created' }
    },
    async prompt() {
      return { accepted: true }
    },
    cancel() {
      return { accepted: true }
    },
    async page() {
      return { records: [], hasMore: false }
    },
    follow(request) {
      followRequests.push(request)
      let closed = false
      return {
        async *[Symbol.asyncIterator]() {
          yield { type: 'snapshot', cursor: 0, hasMore: false, records: [], projections: {}, assistantStream: null }
          while (!closed) await new Promise(resolve => setTimeout(resolve, 50))
        },
      }
    },
  }
}

/** A fake workspace registry. */
function fakeWorkspaceRegistry() {
  return {
    list: () => [{ id: 'w1', path: '/tmp/ws', title: 'Workspace' }],
    get: id => (id === 'w1' ? { id: 'w1', path: '/tmp/ws', title: 'Workspace' } : undefined),
  }
}

/**
 * Plant the Harness services on a fresh root context and load the plugin.
 *
 * @param options.withController - plant `sessionController`; `false` simulates a
 *   profile that does not compose the Web/API services.
 */
async function activate({ withController = true } = {}) {
  const dir = scratchDir()
  const reportPath = join(dir, 'report.json')
  const pidFile = join(dir, 'pid.json')
  const ctx = new cordis.Context()
  const logger = recordingLogger()
  const states = []
  ctx.provide('sessions', { list: () => [] })
  ctx.provide('workspaceRegistry', fakeWorkspaceRegistry())
  if (withController) ctx.provide('sessionController', fakeSessionController())
  ctx.provide('approval', { request: async () => 'unavailable' })
  ctx.provide('userQuestions', { ask: async () => ({ answers: [] }) })

  const saved = snapshotEnv()
  process.env['DSH_QUORFLOAT_PATH'] = mockPath
  process.env['DSH_QUORFLOAT_MOCK_MODE'] = 'normal'
  process.env['DSH_QUORFLOAT_MOCK_REPORT'] = reportPath
  process.env['DSH_QUORFLOAT_MOCK_PIDFILE'] = pidFile

  const plugin = pluginModule.createPlugin({
    logger,
    onStateChange: state => states.push(state),
    // The mock is a Node script rather than a native binary, so it is launched
    // through the interpreter; the spawn path itself is unchanged.
    resolveBinary: () => ({ path: process.execPath, args: [mockPath], source: 'config', attempts: [] }),
  })
  const fiber = ctx.plugin(plugin, {
    heartbeatMs: 150,
    startupTimeoutMs: 4000,
    requestTimeoutMs: 2000,
    shutdownGraceMs: 800,
    logLevel: 'debug',
  })
  await fiber

  const latest = () => states.at(-1) ?? null
  return {
    ctx,
    fiber,
    logger,
    states,
    latest,
    waitForRunning: () =>
      waitFor(
        'the supervisor to report a completed handshake',
        async () => (latest()?.supervisor?.state === 'running' ? latest() : undefined),
        { timeoutMs: 10000 },
      ),
    waitForPid: () =>
      waitFor(
        'the peer to record its pid',
        async () => {
          try {
            return JSON.parse(readFileSync(pidFile, 'utf8'))
          } catch {
            return undefined
          }
        },
        { timeoutMs: 10000 },
      ),
    reportPath,
    pidFile,
    async dispose() {
      await fiber.dispose()
      restoreEnv(saved)
      cleanupDir(dir)
    },
  }
}

/** Capture the environment keys this test mutates. */
function snapshotEnv() {
  const keys = ['DSH_QUORFLOAT_PATH', 'DSH_QUORFLOAT_MOCK_MODE', 'DSH_QUORFLOAT_MOCK_REPORT', 'DSH_QUORFLOAT_MOCK_PIDFILE']
  return Object.fromEntries(keys.map(key => [key, process.env[key]]))
}

/** Restore the captured environment. */
function restoreEnv(saved) {
  for (const [key, value] of Object.entries(saved)) {
    if (value === undefined) delete process.env[key]
    else process.env[key] = value
  }
}

test('the plugin declares exactly one hard dependency', () => {
  // Narrow on purpose. A real dsh proved `sessionController` does not exist when
  // `apply` runs (the profile composes in availability order), so without this
  // declaration the plugin would probe once, see nothing, and never start. Every
  // other service is optional: requiring them would leave the plugin in PENDING
  // on a smaller profile with no explanation of what was missing.
  assert.deepEqual(pluginModule.default.inject, ['sessionController'])
  assert.deepEqual(pluginModule.inject, ['sessionController'])
})

test('the plugin loads in the real framework and reports its services', async () => {
  const app = await activate()
  try {
    const running = await app.waitForRunning()
    assert.equal(running.services.sessionController, true)
    assert.deepEqual(running.missingServices, [])
    assert.equal(running.supervisor.handshaken, true)
    assert.equal(running.supervisor.hotkey.registered, true)
  } finally {
    await app.dispose()
  }
})

test('an invalid configuration is rejected before anything is spawned', async () => {
  const ctx = new cordis.Context()
  ctx.provide('sessionController', fakeSessionController())
  const plugin = pluginModule.createPlugin({ logger: recordingLogger() })
  const fiber = ctx.plugin(plugin, { hotkey: 'Alt+Space', hotket: 'typo' })
  await assert.rejects(async () => await fiber, error => {
    assert.match(error.message, /invalid config/i)
    assert.match(error.message, /hotket/)
    return true
  })
})

test('a missing session controller is named, and no process is started', async () => {
  const app = await activate({ withController: false })
  try {
    await waitFor('the plugin to probe its services', async () => app.latest() !== null)
    const state = app.latest()
    assert.deepEqual(state.missingServices, ['sessionController'], 'the missing service is named, not implied')
    assert.equal(state.supervisor, null, 'no process is started without a usable controller')
    assert.ok(!existsSync(app.pidFile), 'the peer was never launched')
  } finally {
    await app.dispose()
  }
})

test('unloading the plugin leaves no peer process behind', async () => {
  const app = await activate()
  await app.waitForRunning()
  const { pid } = await app.waitForPid()
  assert.ok(processAlive(pid), 'the peer is running before unload')
  const pidFile = app.pidFile
  await app.dispose()
  await waitFor('the peer process to be gone', async () => !processAlive(pid), { timeoutMs: 8000 })
  assert.ok(!existsSync(pidFile), 'the peer removed its own liveness marker as it exited')
})

test('the peer is asked to stop before a signal is used', async () => {
  const app = await activate()
  const running = await app.waitForRunning()
  const { pid } = await app.waitForPid()
  assert.equal(running.supervisor.pid, pid)
  const reportPath = app.reportPath
  await app.fiber.dispose()
  const report = JSON.parse(readFileSync(reportPath, 'utf8'))
  assert.equal(report.shutdownReceived, true, 'the shutdown request reached the peer')
  assert.equal(report.signalled, null, 'no signal was needed for a cooperative peer')
})

/** True while a process with that pid exists. */
function processAlive(pid) {
  try {
    process.kill(pid, 0)
    return true
  } catch {
    return false
  }
}

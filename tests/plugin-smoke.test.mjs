// Category 2 (upstream tripwire): the plugin loads in the real Cordis composition and spawns/unloads a real peer.
// Category 1 (cross-process protocol): the workspace push, observed as a frame on the mock sidecar.
/**
 * Smoke test against the real Cordis framework.
 *
 * Everything else in `tests/` exercises one component with fakes. This file
 * answers the question that must be answered before any UI work: can this plugin
 * be loaded by the actual plugin framework, reach its services, spawn the
 * subproject, complete a handshake, and be unloaded without leaving a process
 * behind?
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

import { cleanupDir, loadCordis, loadPlugin, mockPath, recordingLogger, scratchDir, sleep, waitFor } from './helpers.mjs'

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
 * A fake `workspaceController` whose `follow` stream yields prepared frames.
 *
 * The frames are yielded in order and then the stream stays open, like the Harness's own stream,
 * until the plugin's lifetime signal aborts. `forwarded` flips once the generator has been asked
 * for a frame past the last one — which happens only after the frame before it was awaited by the
 * plugin, so it is the point at which every prepared frame has been written to the peer.
 */
function fakeWorkspaceController(frames) {
  const controller = {
    forwarded: false,
    initializeDefault: async () => undefined,
    follow(signal) {
      return {
        async *[Symbol.asyncIterator]() {
          for (const frame of frames) {
            if (signal.aborted) return
            yield frame
          }
          controller.forwarded = true
          while (!signal.aborted) await new Promise(resolve => setTimeout(resolve, 20))
        },
      }
    },
  }
  return controller
}

/**
 * A `sessionProjections` service with just the change feed the session push subscribes to.
 *
 * The real service owns the title projection; here the feed is enough to prove the plugin
 * subscribed at the handshake, forwarded a title change, and released the listener on unload —
 * a subscription owned by the service's fiber would outlive the plugin that needs it.
 */
function fakeSessionProjections() {
  const listeners = new Set()
  return {
    count: () => listeners.size,
    onChanged(listener) {
      listeners.add(listener)
      return () => listeners.delete(listener)
    },
    change(session, key, value, seq = 1) {
      for (const listener of [...listeners]) listener(session, key, value, seq)
    },
  }
}

/**
 * Plant the Harness services on a fresh root context and load the plugin.
 *
 * @param options.withController - plant `sessionController`; `false` simulates a
 *   profile that does not compose the Web/API services.
 * @param options.workspaceController - plant a `workspaceController` (the workspace
 *   push stream); omitted simulates a profile without the workspace API.
 * @param options.sessionProjections - plant the session projection feed (the title
 *   push); omitted simulates a profile without it.
 */
async function activate({ withController = true, workspaceController, sessionProjections, ...overrides } = {}) {
  const dir = scratchDir()
  const reportPath = join(dir, 'report.json')
  const pidFile = join(dir, 'pid.json')
  const ctx = new cordis.Context()
  const logger = recordingLogger()
  const states = []
  ctx.provide('sessions', { list: () => [] })
  ctx.provide('workspaceRegistry', fakeWorkspaceRegistry())
  if (withController) ctx.provide('sessionController', fakeSessionController())
  if (workspaceController !== undefined) ctx.provide('workspaceController', workspaceController)
  if (sessionProjections !== undefined) ctx.provide('sessionProjections', sessionProjections)
  ctx.provide('approval', { request: async () => 'unavailable' })
  ctx.provide('userQuestions', { ask: async () => ({ answers: [] }) })

  const saved = snapshotEnv()
  process.env['DSH_QUORFLOAT_PATH'] = mockPath
  process.env['DSH_QUORFLOAT_MOCK_MODE'] = 'normal'
  process.env['DSH_QUORFLOAT_MOCK_REPORT'] = reportPath
  process.env['DSH_QUORFLOAT_MOCK_PIDFILE'] = pidFile

  const routers = []
  const plugin = pluginModule.createPlugin({
    logger,
    onStateChange: state => states.push(state),
    onRouter: router => routers.push(router),
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
    ...overrides,
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
    /** The routers created so far; the last one belongs to the live channel. */
    routers,
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
  // Narrow on purpose. `sessionController` does not exist when `apply` runs (the
  // profile composes in availability order), so without this declaration the plugin
  // would probe once, see nothing, and never start. Every other service is optional:
  // requiring them would leave the plugin in PENDING on a smaller profile with no
  // explanation of what was missing.
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

test('the host forwards workspace changes to the sidecar as they happen', async () => {
  // Category 1: the workspace push is a host→sidecar notification, so the shape asserted here is
  // the protocol itself. The frames are the Harness's own (an ISO `createdAt` on every workspace),
  // and what the mock records is what the real sidecar's `workspaces/changed` parser reads — the
  // projection to epoch milliseconds happens on the host side, once.
  //
  // The archive set travels on the same stream and is part of this contract: the panel's
  // conversation list is filtered by it, and only this projection stands between the Harness's
  // registry and the sidecar.
  const baselineAt = Date.parse('2026-01-02T03:04:05.000Z')
  const upsertAt = Date.parse('2026-01-03T04:05:06.000Z')
  const controller = fakeWorkspaceController([
    {
      type: 'baseline',
      value: {
        items: [
          { workspaceId: 'w1', path: '/tmp/ws', title: 'Workspace', sessionIds: ['session-a'], createdAt: '2026-01-02T03:04:05.000Z', updatedAt: '2026-01-02T03:04:05.000Z' },
        ],
        archivedSessionIds: ['session-a'],
        pinnedSessionIds: ['session-a'],
      },
    },
    {
      type: 'upsert',
      workspace: { workspaceId: 'w2', path: '/tmp/two', title: 'Two', sessionIds: [], createdAt: '2026-01-03T04:05:06.000Z' },
    },
    { type: 'remove', workspaceId: 'w1' },
    { type: 'order', workspaceIds: ['w2'] },
    { type: 'archived', archivedSessionIds: ['session-a', 'session-b'] },
  ])

  const app = await activate({ workspaceController: controller })
  try {
    await app.waitForRunning()
    // The subscription opens on the handshake, so this waits for the frames the fake prepared to
    // have been consumed — and each is awaited by the plugin before the next is taken.
    await waitFor('the workspace frames to be forwarded', async () => controller.forwarded === true, { timeoutMs: 8000 })
  } finally {
    // Disposing stops the peer, which writes its report as it exits — and it exercises the
    // lifetime abort: a stream that outlived the plugin would keep forwarding into nothing.
    await app.fiber.dispose()
  }

  const report = JSON.parse(readFileSync(app.reportPath, 'utf8'))
  assert.deepEqual(
    report.workspaceChanges,
    [
      {
        type: 'baseline',
        value: {
          items: [{ workspaceId: 'w1', path: '/tmp/ws', title: 'Workspace', sessionIds: ['session-a'], createdAt: baselineAt }],
          archivedSessionIds: ['session-a'],
          pinnedSessionIds: ['session-a'],
        },
      },
      {
        type: 'upsert',
        workspace: { workspaceId: 'w2', path: '/tmp/two', title: 'Two', sessionIds: [], createdAt: upsertAt },
      },
      { type: 'remove', workspaceId: 'w1' },
      { type: 'order', workspaceIds: ['w2'] },
      { type: 'archived', archivedSessionIds: ['session-a', 'session-b'] },
    ],
    'each frame arrives once, in order, in the shape the sidecar parses',
  )
})

test('the host forwards session changes to the sidecar, and releases the feed on unload', async () => {
  // The session push has no bootstrap frame of its own: it subscribes to the Harness's events
  // and to the projection feed, both of which only make sense once a channel exists to carry
  // what they produce. What is asserted here is the whole path — real Cordis event, the
  // projection feed, the supervisor's channel, and the child process's own record.
  const projections = fakeSessionProjections()
  const app = await activate({ sessionProjections: projections })
  const frames = () => {
    try {
      return JSON.parse(readFileSync(app.reportPath, 'utf8')).sessionChanges ?? []
    } catch {
      return []
    }
  }
  try {
    await app.waitForRunning()
    await waitFor('the session change stream to subscribe', async () => projections.count() === 1)

    app.ctx.emit('api-session/added', {
      sessionId: 'session-a',
      updatedAt: 10,
      running: false,
      blank: true,
      cwd: '/tmp/ws',
      projections: { kind: 'sequenced', asOfSeq: 1, values: { title: 'One' } },
    })
    projections.change({ id: 'session-a', header: { origin: 'primary' } }, 'title', 'Renamed')
    app.ctx.emit('api-session/status', 'session-a', true)
    app.ctx.emit('api-session/removed', 'session-a')

    await waitFor('the pushed frames to reach the peer', async () => frames().length === 4, { timeoutMs: 8000 })
  } finally {
    await app.fiber.dispose()
  }

  assert.deepEqual(
    frames(),
    [
      {
        type: 'upsert',
        session: { sessionId: 'session-a', title: 'One', cwd: '/tmp/ws', updatedAt: 10, running: false, blank: true },
      },
      { type: 'title', sessionId: 'session-a', title: 'Renamed' },
      { type: 'status', sessionId: 'session-a', running: true },
      { type: 'refresh', reason: 'removed' },
    ],
    'each frame arrives once, in order, in the shape the sidecar parses',
  )
  assert.equal(projections.count(), 0, 'the projection subscription is released, not leaked')
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

test('a presence report from the browser half reaches the plugin authority', async () => {
  // The browser half is the only source of "is the user looking at the Harness
  // window", so its entry point is exercised against the real framework. The
  // handshake is not required first: presence is reported as soon as the page
  // loads, which can precede a peer handshake.
  const app = await activate()
  try {
    await app.waitForRunning()
    const router = app.routers.at(-1)
    assert.ok(router !== undefined, 'a router was created for the live channel')

    const before = await router.handle('diag/snapshot', {})
    assert.equal(before.authority.authority, 'none', 'nothing reported yet, and the panel is not visible')

    const accepted = await router.handle('presence/report', { surface: 'web', visible: true, focused: true, seq: 1 })
    assert.deepEqual(accepted, { accepted: true })
    const after = await router.handle('diag/snapshot', {})
    assert.equal(after.authority.authority, 'harness', 'a focused visible page owns the next decision')

    const stale = await router.handle('presence/report', { surface: 'web', visible: false, focused: false, seq: 1 })
    assert.equal(stale.accepted, false, 'a replayed sequence number is refused')
    assert.equal(stale.reason, 'stale-sequence')
    const settled = await router.handle('diag/snapshot', {})
    assert.equal(settled.authority.authority, 'harness', 'the refused report changed nothing')
  } finally {
    await app.dispose()
  }
})

test('a malformed presence report is rejected as a bad request', async () => {
  const app = await activate()
  try {
    await app.waitForRunning()
    const router = app.routers.at(-1)
    // `focused` is required: visibility alone cannot tell "occluded on Windows"
    // from "in front of the user", so a half report must not be accepted.
    await assert.rejects(
      () => router.handle('presence/report', { surface: 'web', visible: true, seq: 1 }),
      error => {
        assert.equal(error.code, 'unavailable')
        assert.match(error.message, /focused must be a boolean/)
        return true
      },
    )
  } finally {
    await app.dispose()
  }
})

test('the plugin marks a language nobody chose as a fallback for the sidecar', async () => {
  // End to end through the real plugin, because the two halves are wired by hand: the
  // default configuration names no language and the fakes provide no Harness locale, so
  // this launch is exactly the "nobody chose anything" state. The sidecar must be told
  // that, or it would write `en` down as the panel's own choice and the panel's webview
  // would never get to answer with `navigator.languages`.
  const app = await activate()
  await app.waitForRunning()
  const reportPath = app.reportPath
  await app.fiber.dispose()
  const report = JSON.parse(readFileSync(reportPath, 'utf8'))
  assert.equal(report.windowLanguageEnv, 'en', 'the fallback is still what the first frame draws with')
  assert.equal(report.windowLanguageDecidedEnv, '0', 'and the plugin says it is not a decision')
  assert.equal(report.readyWindow?.languageDecided, false)
})

test('the interaction limits reach the objects that enforce them', async () => {
  // A config field nobody passes on is a silent bug: the schema accepts it, the
  // settings page offers it, and nothing changes. These are the ends the plugin
  // wires by hand, so they are asserted through the real composition rather than
  // by constructing the layers directly.
  const app = await activate({
    claimDeadlineMs: 12345,
    maxStderrLines: 7,
    maxWriteBufferBytes: 131072,
  })
  try {
    await app.waitForRunning()
    const router = app.routers.at(-1)
    assert.ok(router !== undefined, 'a router was created')
    const snapshot = await router.handle('diag/snapshot', {})
    assert.equal(snapshot.config.claimDeadlineMs, 12345, 'the plugin accepted the override')
    assert.equal(snapshot.config.maxStderrLines, 7)
    assert.equal(snapshot.config.maxWriteBufferBytes, 131072)
    assert.equal(
      snapshot.interactions.deadlineMs,
      12345,
      'and the interaction layer was constructed with it, not with the default',
    )
  } finally {
    await app.dispose()
  }
})

test('panel/lifecycle restart replaces the peer through the real supervisor', async () => {
  // The panel's tray menu asks the host to replace the process that hosts it. The
  // router, the supervisor, and the spawn are all real here; `onRouter` is what
  // makes the request reachable without a live peer.
  const app = await activate()
  try {
    await app.waitForRunning()
    const { pid } = await app.waitForPid()
    const router = app.routers.at(-1)
    assert.ok(router !== undefined, 'a router was created for the live channel')

    const result = await router.handle('panel/lifecycle', { action: 'restart' })
    assert.deepEqual(result, { action: 'restart', accepted: true })

    // A restart is stop-then-start: the old process is gone, a fresh one is
    // running, and it got a router of its own.
    const next = await app.waitForPid()
    assert.notEqual(next.pid, pid, 'the replacement process is a different one')
    await waitFor('the old process to be gone', async () => !processAlive(pid))
    const running = await app.waitForRunning()
    assert.equal(running.supervisor.pid, next.pid)
    assert.equal(app.routers.length, 2, 'the replacement channel got its own router')
    assert.notEqual(app.routers.at(-1), router)
  } finally {
    await app.dispose()
  }
})

test('panel/lifecycle stop ends the peer deliberately and no restart undoes it', async () => {
  const app = await activate()
  try {
    await app.waitForRunning()
    const { pid } = await app.waitForPid()
    const router = app.routers.at(-1)
    assert.ok(router !== undefined, 'a router was created for the live channel')

    const result = await router.handle('panel/lifecycle', { action: 'stop' })
    assert.deepEqual(result, { action: 'stop', accepted: true })
    assert.equal(app.latest().supervisor.state, 'stopped')
    assert.equal(app.latest().supervisor.pid, undefined)
    await waitFor('the stopped peer to be gone', async () => !processAlive(pid))

    // The whole reason `stop` must go through the supervisor's own stop entry:
    // the exit is then marked deliberate, so the restart policy does not read it
    // as a crash and bring the panel back. A silent automatic restart would also
    // create a router and emit state changes, so both counters are watched.
    const routers = app.routers.length
    const states = app.states.length
    await sleep(400)
    assert.equal(app.routers.length, routers, 'no replacement process was started')
    assert.equal(app.states.length, states, 'nothing changed after the stop settled')
    assert.equal(app.latest().supervisor.state, 'stopped')
  } finally {
    await app.dispose()
  }
})

test('the browser half reads supervision through the control endpoint', async () => {
  // The web half has no view of the sidecar process, so this endpoint is its
  // only read path. Everything here is the real composition: the framework, the
  // plugin, the supervisor, and the spawned mock. Resolving the service through
  // the context is what proves `src/index.ts` registered it and handed it the
  // supervisor that is actually running.
  const app = await activate()
  try {
    await app.waitForRunning()
    const control = app.ctx.get('quorfloatControl')
    assert.ok(control !== undefined, 'the control gateway is registered in the real composition')
    const { pid } = await app.waitForPid()
    const status = control.status()
    assert.equal(status.state, 'running')
    assert.equal(status.pid, pid, 'the reported pid is the process the mock recorded')
    assert.equal(status.seen, true, 'the handshake is what "seen" reports')
    assert.equal(status.visible, false, 'the panel has not reported a window')
    assert.equal(status.restartExhausted, false)
    assert.equal(status.lastError, null)
  } finally {
    await app.dispose()
  }
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

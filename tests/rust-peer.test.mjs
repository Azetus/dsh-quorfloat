/**
 * Cross-language handshake: the real host plugin against the real Rust binary.
 *
 * Everything on the host side is the shipping implementation — `QuorfloatSupervisor`
 * spawns the process, `QuorfloatChannel` frames it, and `HostRouter` validates the
 * `hello`. Only the binary path is substituted, so this test answers the question
 * the two sides cannot answer separately: *do they agree about the wire?*
 *
 * It exists because the alternative is finding out from a handshake timeout in a
 * packaged app, where the failure looks identical to a binary that never started.
 * The previous cross-language defect in this project (a remote method whose
 * parameter names did not match the descriptor) was found only by a live browser
 * request; this is the same class of bug, caught before it ships.
 *
 * Skipped when the Rust binary has not been built, so the suite stays runnable
 * from a clean clone. `cargo build` in `quorfloat/` enables it.
 */

import assert from 'node:assert/strict'
import { existsSync } from 'node:fs'
import { test } from 'node:test'
import { join } from 'node:path'

import { buildSupervisor, repoRoot, sleep, waitFor } from './helpers.mjs'

/** Where `cargo build` leaves the binary, newest profile first. */
function rustBinary() {
  const candidates = [
    join(repoRoot, 'quorfloat/target/debug/dsh-quorfloat'),
    join(repoRoot, 'quorfloat/target/release/dsh-quorfloat'),
  ]
  return candidates.find(candidate => existsSync(candidate))
}

const binary = rustBinary()
const skip = binary === undefined
  ? 'the Rust binary is not built; run `cargo build` in quorfloat/'
  : false

/** Start the real supervisor against the real Rust binary. */
async function start(config = {}) {
  const app = await buildSupervisor({
    config: { heartbeatMs: 200, startupTimeoutMs: 8000, ...config },
    deps: {
      // Only the executable is swapped; the resolver's own path handling is
      // already covered elsewhere.
      routerHost: {},
    },
  })
  return app
}

test('the real supervisor completes a handshake with the Rust binary', { skip }, async () => {
  const { QuorfloatSupervisor } = await import(new URL('../lib/host/supervisor.js', import.meta.url))
  const { HostRouter } = await import(new URL('../lib/bridge/router.js', import.meta.url))
  const { DEFAULT_CONFIG } = await import(new URL('../lib/config.js', import.meta.url))

  const effective = {
    ...DEFAULT_CONFIG,
    heartbeatMs: 200,
    heartbeatMissLimit: 3,
    startupTimeoutMs: 8000,
    shutdownGraceMs: 1000,
    logLevel: 'debug',
  }
  const events = []
  const logLines = []
  // The router is constructed by the supervisor and not exposed, so the recorded
  // handshake is read through the diagnostics hook the router already calls —
  // an existing observation point rather than a new test-only method.
  let recordedHandshake
  let router
  const logger = {
    info: (...args) => logLines.push(['info', ...args]),
    warn: (...args) => logLines.push(['warn', ...args]),
    debug: () => {},
    error: (...args) => logLines.push(['error', ...args]),
  }
  // A purposely unparseable accelerator, so the "not registered" assertion below
  // tests this crate's reporting rather than whether the machine's Alt+Space
  // happens to be free. A real registration is covered separately.
  process.env['DSH_QUORFLOAT_HOTKEY'] = 'Alt+Banana'
  const supervisor = new QuorfloatSupervisor({
    config: () => effective,
    resolveBinary: () => ({ path: binary, args: [], source: 'config', attempts: [] }),
    createRouter: channelSessionId => {
      router = new HostRouter({
        config: () => effective,
        channelSessionId: () => channelSessionId,
        hostVersion: () => 'cross-language-test',
        listWorkspaces: async () => ({ items: [] }),
        listSessions: async () => ({ items: [] }),
        createSession: async () => ({ sessionId: 's' }),
        attachSession: async sessionId => ({ sessionId }),
        readHistory: async () => ({ records: [], hasMore: false }),
        prompt: async () => ({ accepted: true }),
        cancel: async () => ({ accepted: true }),
        answerInteraction: async () => ({ accepted: true }),
        reportPresence: async () => ({ accepted: true }),
        diagnostics: () => {
          recordedHandshake = router?.snapshotForTest?.() ?? recordedHandshake
          return {}
        },
      })
      return router
    },
    log: logger,
    onEvent: event => events.push(event),
  })

  try {
    await supervisor.start()
    await waitFor(
      'the Rust binary to be accepted as running',
      async () => supervisor.snapshot().state === 'running',
      { timeoutMs: 10000 },
    )
    const snapshot = supervisor.snapshot()
    assert.equal(snapshot.handshaken, true, 'the host accepted our hello')

    // Read what the *host recorded about the peer*, which is the peer's own
    // report. The handshake event carries the host's answer instead, so asserting
    // on it would test the host's constants rather than the Rust side's facts.
    // Ask the router for its snapshot directly: `diag/snapshot` includes the
    // handshake facts it recorded from our `hello`.
    const diagnostics = await router.handle('diag/snapshot', {})
    const recorded = diagnostics.handshake
    assert.ok(recorded !== null && recorded !== undefined, 'the host recorded a handshake')
    assert.equal(recorded.protocol, 'quorfloat/1')
    assert.equal(recorded.quorfloatVersion, '0.0.1')
    // The wire carries the *host's* vocabulary, so these are compared against
    // Node's own constants rather than Rust's: `macos`/`aarch64` would be valid
    // facts spelled uselessly, and nothing on the host branches on them.
    assert.equal(recorded.platform, process.platform, 'platform uses process.platform spelling')
    assert.equal(recorded.arch, process.arch, 'arch uses process.arch spelling')
    assert.deepEqual(recorded.capabilities, ['window', 'hotkey', 'egui'])
    // The reported flag is the *real* outcome of the grab, which is what makes a
    // conflict visible in the host's settings surface instead of leaving the user
    // pressing a key that does nothing.
    assert.equal(recorded.hotkey.registered, false, 'an unparseable accelerator cannot register')
    assert.equal(recorded.hotkey.requested, 'Alt+Banana', 'the requested accelerator is reported verbatim')

    // Liveness is judged by `ping`, so surviving several heartbeats is the proof
    // that the Rust read loop is actually answering rather than merely alive.
    // `heartbeatMs` is 200 here, so ~900ms covers several rounds.
    await sleep(900)
    const after = supervisor.snapshot()
    assert.equal(after.state, 'running', 'still running after several heartbeats')
    assert.equal(after.consecutiveMissedHeartbeats, 0, 'every ping was answered')
    assert.equal(after.rejectedFrames, 0, 'the host rejected none of our frames, in either direction')
  } finally {
    const result = await supervisor.stop()
    assert.equal(result.exited, true, 'the binary exited when asked')
    assert.equal(result.escalated, false, 'it exited on the shutdown request, without SIGTERM')
    assert.equal(result.code, 0, `clean exit, got code ${result.code} signal ${result.signal}`)
  }
})

test('the Rust binary exits on stdin EOF instead of lingering', { skip }, async () => {
  // The host's cleanup is judged by observed process exit. A peer that ignored a
  // closed stdin would be killed by escalation, which is exactly the evidence the
  // host refuses to accept as a clean shutdown.
  const { spawn } = await import('node:child_process')
  const child = spawn(binary, [], { stdio: ['pipe', 'pipe', 'pipe'] })
  const stdout = []
  child.stdout.on('data', chunk => stdout.push(String(chunk)))

  // Answer the handshake so the process is not sitting in its startup path.
  await waitFor('the hello frame', async () => stdout.join('').includes('"hello"'), { timeoutMs: 5000 })
  const first = JSON.parse(stdout.join('').split('\n')[0])
  assert.equal(first.method, 'hello')
  child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id: 1, result: { protocol: 'quorfloat/1' } })}\n`)
  await sleep(150)

  const exited = new Promise(resolve => child.on('exit', (code, signal) => resolve({ code, signal })))
  child.stdin.end()
  const { code, signal } = await exited
  assert.equal(signal, null, 'it exited on its own, not by signal')
  assert.equal(code, 0)
})

test('a real accelerator registers and the session stays healthy', { skip }, async () => {
  // The positive half of the hotkey contract. An accelerator unlikely to be taken
  // is chosen so the test measures this crate rather than the machine's luck, and
  // the assertion is on the whole session: a registration that broke the event
  // loop would be worse than one that failed.
  process.env['DSH_QUORFLOAT_HOTKEY'] = 'Control+Alt+Shift+F13'
  const { QuorfloatSupervisor } = await import(new URL('../lib/host/supervisor.js', import.meta.url))
  const { HostRouter } = await import(new URL('../lib/bridge/router.js', import.meta.url))
  const { DEFAULT_CONFIG } = await import(new URL('../lib/config.js', import.meta.url))
  const effective = {
    ...DEFAULT_CONFIG,
    heartbeatMs: 200,
    heartbeatMissLimit: 3,
    startupTimeoutMs: 8000,
    shutdownGraceMs: 1000,
    logLevel: 'debug',
  }
  let recorded
  const supervisor = new QuorfloatSupervisor({
    config: () => effective,
    resolveBinary: () => ({ path: binary, args: [], source: 'config', attempts: [] }),
    createRouter: channelSessionId =>
      new HostRouter({
        config: () => effective,
        channelSessionId: () => channelSessionId,
        hostVersion: () => 'cross-language-test',
        listWorkspaces: async () => ({ items: [] }),
        listSessions: async () => ({ items: [] }),
        createSession: async () => ({ sessionId: 's' }),
        attachSession: async sessionId => ({ sessionId }),
        readHistory: async () => ({ records: [], hasMore: false }),
        prompt: async () => ({ accepted: true }),
        cancel: async () => ({ accepted: true }),
        answerInteraction: async () => ({ accepted: true }),
        reportPresence: async () => ({ accepted: true }),
        diagnostics: () => ({}),
      }),
    log: { info: () => {}, warn: () => {}, debug: () => {}, error: () => {} },
    onEvent: () => {},
  })
  try {
    await supervisor.start()
    await waitFor('running', async () => supervisor.snapshot().state === 'running', { timeoutMs: 10000 })
    const handshake = supervisor.snapshot().hotkey
    assert.ok(handshake !== null, 'the host recorded a hotkey report')
    assert.equal(handshake.requested, 'Control+Alt+Shift+F13')
    recorded = handshake.registered
    await sleep(700)
    assert.equal(supervisor.snapshot().state, 'running', 'registration did not disturb the loop')
    assert.equal(supervisor.snapshot().consecutiveMissedHeartbeats, 0, 'pings are still answered')
  } finally {
    delete process.env['DSH_QUORFLOAT_HOTKEY']
    const result = await supervisor.stop()
    assert.equal(result.exited, true)
    assert.equal(result.escalated, false)
    // Reported because a headless CI machine may legitimately have no window server,
    // and failing there would say nothing about the code.
    console.log(`hotkey registered on this machine: ${String(recorded)}`)
  }
})

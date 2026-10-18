// Category 1 (cross-process interface): the diagnostics endpoint the settings page calls, and the
// text/redaction contract it answers with.
/**
 * Diagnostics export tests.
 *
 * The file contains a current-state snapshot plus bounded recent supervisor and
 * sidecar-stderr history. These tests pin the interface the browser half depends
 * on: one zero-argument endpoint that writes a text file and answers its path,
 * plus the redaction and size rules that keep the export useful and bounded.
 */

import assert from 'node:assert/strict'
import { mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { test } from 'node:test'

import { loadModule } from './helpers.mjs'

const {
  formatDiagnostics,
  logDirectory,
  redactHome,
  saveDiagnostics,
  shortId,
} = await loadModule('host/diagnostics.js')
const {
  DiagnosticBuffers,
  EVENT_LIMIT,
  STDERR_LIMIT,
  STDERR_BYTE_LIMIT,
  STDERR_LINE_BYTE_LIMIT,
} = await loadModule('host/diagnostic-buffer.js')
const {
  registerDiagnosticsGateway,
  DIAGNOSTICS_ENDPOINT_METHOD,
  DIAGNOSTICS_NAMESPACE,
  DIAGNOSTICS_SERVICE_KEY,
} = await loadModule('host/diagnostics-gateway.js')
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

/** A complete enough snapshot for the formatter; the values are deliberately recognisable. */
function input() {
  return {
    generatedAt: '2026-10-18T06:30:00.000Z',
    protocolVersion: 'quorfloat/1',
    dshVersion: '0.2.0-rc.2',
    platform: 'darwin',
    arch: 'arm64',
    home: '/Users/alice',
    config: {
      enabled: true,
      hotkey: 'Alt+Space',
      quorfloatPath: '/Users/alice/bin/dsh-quorfloat',
      defaultWorkspaceId: 'workspace-1234567890abcdef',
    },
    state: {
      services: { sessionController: true, workspaceRegistry: true, approval: true, userQuestions: true },
      missingServices: [],
      supervisor: {
        state: 'running',
        handshaken: true,
        pid: 4242,
        binaryPath: '/Users/alice/.dsh/quorfloat/dsh-quorfloat',
        binarySource: 'platform-package',
        peerCapabilities: ['window', 'hotkey'],
        peerVersion: '0.1.6',
        peerPlatform: 'darwin',
        peerArch: 'arm64',
        restarts: 0,
        lastError: undefined,
      },
      sessions: {
        activeSessionId: 'session-1234567890abcdef',
        conversation: 'SECRET-CONVERSATION',
        follows: [{
          sessionId: 'session-1234567890abcdef',
          generation: 2,
          cursor: 120,
          snapshotDelivered: true,
        }],
        prompts: [{
          requestId: 'request-1234567890abcdef',
          sessionId: 'session-1234567890abcdef',
          state: 'accepted',
          text: 'SECRET-PROMPT',
        }],
      },
      interactions: {
        registered: true,
        deadlineMs: 600000,
        questionText: 'SECRET-QUESTION',
        pending: [{
          interactionId: 'interaction-1234567890abcdef',
          sessionId: 'session-1234567890abcdef',
          kind: 'approval',
        }],
        counters: { claimed: 1, answered: 0 },
      },
      stopped: false,
    },
    authority: { authority: 'panel', reason: 'harness-not-visible', fresh: ['web'] },
    logs: {
      supervisor: [],
      stderr: [],
      supervisorDropped: 0,
      stderrDropped: 0,
      supervisorTruncated: false,
      stderrTruncated: false,
    },
  }
}

test('the endpoint registers under its key and answers a path from the host callback', () => {
  const ctx = fakeContext()
  const expected = { path: '/Users/alice/.dsh-quorfloat/logs/x.log', filename: 'x.log' }
  const dispose = registerDiagnosticsGateway({
    ctx,
    run: () => expected,
    log: silentLog,
  })
  assert.equal(typeof dispose, 'function')
  assert.equal(ctx.provided[0].name, DIAGNOSTICS_SERVICE_KEY)
  const service = ctx.provided[0].value
  assert.equal(service.typertRemote.namespace, DIAGNOSTICS_NAMESPACE)
  const prototype = Object.getPrototypeOf(service)
  assert.deepEqual(prototype[REMOTE_METHOD_DESCRIPTOR].methods, [
    { method: DIAGNOSTICS_ENDPOINT_METHOD, invocation: { kind: 'direct' } },
  ])
  assert.deepEqual(prototype[DIAGNOSTICS_ENDPOINT_METHOD](), expected)
  assert.equal(
    prototype[DIAGNOSTICS_ENDPOINT_METHOD].toString().replace(/^[^(]*\(/, '').replace(/\).*$/s, ''),
    '',
    'the endpoint takes no arguments, so the page sends args: {}',
  )
})

test('the text has the current-state and bounded log sections', () => {
  const text = formatDiagnostics(input())
  for (const section of [
    '[supervisor]',
    '[config]',
    '[services]',
    '[sessions]',
    '[interactions]',
    '[authority]',
    '[logs]',
    '[logs.supervisor]',
    '[logs.stderr]',
  ]) {
    assert.ok(text.includes(section), `${section} is present`)
  }
  assert.ok(text.includes('supervisorEvents: 0'), 'the empty supervisor ring is reported')
  assert.ok(text.includes('stderrLines: 0'), 'the empty stderr ring is reported')
  assert.ok(!text.includes('marker'), 'the dev-only marker is absent')
  assert.ok(!text.includes('SECRET-CONVERSATION'), 'session text is not projected')
  assert.ok(!text.includes('SECRET-PROMPT'), 'prompt text is not projected')
  assert.ok(!text.includes('SECRET-QUESTION'), 'question text is not projected')
  assert.ok(text.endsWith('\n'), 'a text file ends with one newline')
})

test('long identifiers are shortened and home paths are redacted', () => {
  const text = formatDiagnostics(input())
  assert.ok(text.includes('session-…cdef'), 'the session id keeps both ends')
  assert.ok(text.includes('request-…cdef'), 'the request id keeps both ends')
  assert.ok(text.includes('interact…cdef'), 'the interaction id keeps both ends')
  assert.ok(text.includes('workspac…cdef'), 'the default workspace id keeps both ends')
  assert.ok(text.includes('~/.dsh/quorfloat/dsh-quorfloat'), 'the binary path is home-relative')
  assert.ok(text.includes('~/bin/dsh-quorfloat'), 'the override path is home-relative')
  assert.ok(!text.includes('/Users/alice'), 'the absolute home is not present')
})

test('the identifier/path helpers behave at their boundaries', () => {
  assert.equal(shortId('short-id'), 'short-id')
  assert.equal(shortId('session-1234567890abcdef'), 'session-…cdef')
  assert.equal(redactHome('/Users/alice/tmp/x', '/Users/alice'), '~/tmp/x')
  assert.equal(redactHome('prefix /Users/alice/tmp/x', '/Users/alice'), 'prefix ~/tmp/x')
})

test('the log directory follows the explicit override, then preferences, then home', () => {
  assert.equal(
    logDirectory({ DSH_QUORFLOAT_LOG_DIR: '/tmp/quorfloat-logs' }, '/home/alice'),
    resolve('/tmp/quorfloat-logs'),
  )
  assert.equal(
    logDirectory({ DSH_QUORFLOAT_PREFERENCES: '/tmp/state/preferences.json' }, '/home/alice'),
    join(resolve('/tmp/state'), 'logs'),
  )
  assert.equal(
    logDirectory({}, '/home/alice'),
    join('/home/alice', '.dsh-quorfloat', 'logs'),
  )
})

test('log lines are rendered and home-redacted', () => {
  const withLogs = input()
  withLogs.logs = {
    supervisor: [
      {
        at: Date.parse('2026-10-18T06:30:01.000Z'),
        kind: 'state',
        state: 'starting',
        pid: null,
        startedAt: null,
        restarts: 0,
        detail: null,
      },
      {
        at: Date.parse('2026-10-18T06:30:02.000Z'),
        kind: 'exit',
        state: 'running',
        pid: 4242,
        startedAt: 1,
        restarts: 1,
        detail: 'code=1 path=/Users/alice/tmp',
      },
    ],
    stderr: [{
      at: Date.parse('2026-10-18T06:30:03.000Z'),
      pid: 4242,
      startedAt: 1,
      state: 'running',
      line: 'first line\nsecond line /Users/alice/tmp',
    }],
    supervisorDropped: 2,
    stderrDropped: 3,
    supervisorTruncated: false,
    stderrTruncated: false,
  }
  const text = formatDiagnostics(withLogs)
  assert.ok(text.includes('[logs.supervisor]'), 'the supervisor section is present')
  assert.ok(text.includes('state=starting'), 'a state entry is rendered')
  assert.ok(text.includes('exit pid=4242'), 'an exit entry is rendered')
  assert.ok(text.includes('code=1 path=~/tmp'), 'event detail is home-redacted')
  assert.ok(text.includes('[logs.stderr]'), 'the stderr section is present')
  assert.ok(text.includes('first line\\nsecond line ~/tmp'), 'stderr is flattened and home-redacted')
  assert.ok(text.includes('dropped 2'), 'supervisor drops are reported')
  assert.ok(text.includes('dropped 3'), 'stderr drops are reported')
  assert.ok(!text.includes('/Users/alice'), 'no absolute home path survives in logs')
})

test('the diagnostic rings stay inside their count and byte caps', () => {
  const buffers = new DiagnosticBuffers()
  const longLine = 'x'.repeat(3000)
  for (let index = 0; index < 500; index += 1) {
    buffers.record({
      kind: 'stderr',
      snapshot: { state: 'running', pid: 42, startedAt: 1, restarts: 0 },
      detail: longLine,
    })
  }
  const logs = buffers.snapshot()
  assert.ok(logs.stderr.length <= STDERR_LIMIT, 'stderr stays under its line cap')
  assert.ok(
    logs.stderr.reduce((total, entry) => total + Buffer.byteLength(entry.line, 'utf8'), 0) <= STDERR_BYTE_LIMIT,
    'stderr text stays under its byte cap',
  )
  assert.ok(
    logs.stderr.every(entry => Buffer.byteLength(entry.line, 'utf8') <= STDERR_LINE_BYTE_LIMIT),
    'each stderr line is truncated to its own cap',
  )
  assert.ok(logs.stderrDropped > 0, 'old stderr lines are dropped')
  assert.equal(logs.stderrTruncated, true, 'truncation is recorded')

  for (let index = 0; index < 500; index += 1) {
    buffers.record({
      kind: 'state',
      snapshot: { state: 'running', pid: 42, startedAt: 1, restarts: 0 },
      detail: undefined,
    })
  }
  const events = buffers.snapshot()
  assert.ok(events.supervisor.length <= EVENT_LIMIT, 'events stay under their count cap')
  assert.ok(events.supervisorDropped > 0, 'old events are dropped')
})

test('saving writes one new file with the expected name and contents', () => {
  const directory = mkdtempSync(join(tmpdir(), 'dsh-quorfloat-diagnostics-'))
  try {
    const saved = saveDiagnostics('body', {
      directory,
      now: new Date('2026-10-18T06:30:00.000Z'),
    })
    assert.equal(saved.filename, 'dsh-quorfloat-diagnostics-20261018-063000-000.log')
    assert.equal(saved.path, join(directory, saved.filename))
    assert.equal(readFileSync(saved.path, 'utf8'), 'body\n')

    const again = saveDiagnostics('second', {
      directory,
      now: new Date('2026-10-18T06:30:00.000Z'),
    })
    assert.equal(again.filename, 'dsh-quorfloat-diagnostics-20261018-063000-000-1.log')
    assert.notEqual(again.path, saved.path)
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

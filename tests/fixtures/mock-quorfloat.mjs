#!/usr/bin/env node
/**
 * A fake quorfloat process for host-side testing.
 *
 * It speaks the real wire protocol over its own stdin/stdout, so the tests
 * exercise the framing, handshake, heartbeat, and shutdown paths exactly as the
 * Rust subproject will. Behaviour is selected with `DSH_QUORFLOAT_MOCK_MODE`:
 *
 * - `normal`     handshake, answer `ping`, honour `shutdown` (default)
 * - `malformed`  emit garbage and half frames before answering
 * - `half-frame` write every frame in two chunks with a delay between them
 * - `silent`     never answer `ping`, but keep consuming input
 * - `stubborn`   like `silent`, and also ignores `shutdown` (forces escalation)
 * - `no-handshake` never sends `hello` (exercises the startup budget)
 * - `slow`       delay every response by `DSH_QUORFLOAT_MOCK_DELAY_MS`
 * - `exit-early` exit immediately after the handshake
 * - `no-hotkey`  report that the hotkey could not be registered
 *
 * Scenario side effects are also observable: the mock writes a JSON summary to
 * the file named by `DSH_QUORFLOAT_MOCK_REPORT` when it exits, which lets tests
 * assert on shutdown order without parsing log output.
 */

import { rmSync, writeFileSync } from 'node:fs'

const MODE = process.env['DSH_QUORFLOAT_MOCK_MODE'] ?? 'normal'
const DELAY_MS = Number(process.env['DSH_QUORFLOAT_MOCK_DELAY_MS'] ?? '0')
const REPORT = process.env['DSH_QUORFLOAT_MOCK_REPORT']
/** Written at startup and removed at exit: a host-visible liveness marker. */
const PIDFILE = process.env['DSH_QUORFLOAT_MOCK_PIDFILE']
const VERSION = process.env['DSH_QUORFLOAT_MOCK_VERSION'] ?? '0.0.1-mock'
const PROTOCOL = process.env['DSH_QUORFLOAT_MOCK_PROTOCOL'] ?? 'quorfloat/1'

/** Records what the process observed, for assertions after it exits. */
const observed = {
  mode: MODE,
  pid: process.pid,
  helloAnswered: false,
  readyReceived: false,
  heartbeats: 0,
  pings: 0,
  requests: [],
  shutdownReceived: false,
  signalled: null,
  stdoutBytes: 0,
  stderr: [],
}

let buffer = ''
let nextId = 1
const pending = new Map()
/** Frames that arrived before the handshake finished, in arrival order. */
const queue = []
/** Set once the handshake completed; before that, frames are queued. */
let ready = false

/** Write one protocol frame to stdout; this is the only stdout writer. */
function send(message, { split = false } = {}) {
  const frame = `${JSON.stringify(message)}\n`
  observed.stdoutBytes += Buffer.byteLength(frame)
  if (split && frame.length > 4) {
    const cut = Math.floor(frame.length / 2)
    process.stdout.write(frame.slice(0, cut))
    setTimeout(() => process.stdout.write(frame.slice(cut)), 5)
    return
  }
  process.stdout.write(frame)
}

/** Record this process's existence so a test can observe its lifetime. */
function writePidFile() {
  if (!PIDFILE) return
  try {
    writeFileSync(PIDFILE, JSON.stringify({ pid: process.pid, startedAt: Date.now() }))
  } catch {
    // ignore
  }
}

/** Remove the liveness marker; the report file is the durable record. */
function removePidFile() {
  if (!PIDFILE) return
  try {
    rmSync(PIDFILE, { force: true })
  } catch {
    // ignore
  }
}

/** Log to stderr only: stdout must stay frame-pure. */
function note(...args) {
  const line = args.map(value => (typeof value === 'string' ? value : JSON.stringify(value))).join(' ')
  observed.stderr.push(line)
  process.stderr.write(`[mock-quorfloat] ${line}\n`)
}

/** Send a request to the host and resolve with its result. */
function call(method, params) {
  const id = nextId++
  const answer = new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject, method })
    setTimeout(() => {
      if (pending.delete(id)) reject(new Error(`${method} timed out in the mock`))
    }, 10000).unref?.()
  })
  send({ jsonrpc: '2.0', id, method, params })
  return answer
}

/** Answer a host request. */
function reply(id, result) {
  const act = () => send({ jsonrpc: '2.0', id, result })
  if (DELAY_MS > 0) setTimeout(act, DELAY_MS)
  else act()
}

/** Answer a host request with an error. */
function replyError(id, code, message) {
  send({ jsonrpc: '2.0', id, error: { code, message } })
}

/** Emit scenario-specific noise before normal operation starts. */
function emitNoise() {
  if (MODE !== 'malformed') return
  process.stdout.write('this is not json at all\n')
  process.stdout.write('{"jsonrpc":"2.0","method":"half-fra')
  process.stdout.write('me","params":{}}\n')
  process.stdout.write('{"jsonrpc":"2.0","id":"not-an-integer","method":"broken"}\n')
  process.stdout.write('\n')
}

/** Run the handshake, then keep serving frames until stdin ends. */
async function main() {
  // stdin is consumed from startup, not after the handshake: an unread pipe
  // does not hold the event loop open, and a real subproject reads its input
  // continuously too. Frames arriving before the handshake completes are
  // queued rather than dropped.
  process.stdin.setEncoding('utf8')
  process.stdin.on('data', chunk => {
    buffer += chunk
    for (;;) {
      const newline = buffer.indexOf('\n')
      if (newline === -1) break
      const line = buffer.slice(0, newline)
      buffer = buffer.slice(newline + 1)
      if (line.trim() === '') continue
      let message
      try {
        message = JSON.parse(line)
      } catch {
        note('host sent a frame the mock could not parse')
        continue
      }
      // A *response* settles a promise we are already waiting on, so it is
      // handled immediately even before the handshake completes; queueing it
      // would deadlock the handshake against itself. Frames the host sends
      // before readiness are queued instead, so nothing is lost either way.
      if (ready || isResponse(message)) handle(message)
      else queue.push(message)
    }
  })
  process.stdin.on('end', () => {
    note('stdin closed; exiting')
    finish(0)
  })

  emitNoise()
  note('mode', MODE, 'protocol', PROTOCOL)
  if (MODE === 'no-handshake') {
    note('deliberately not sending hello')
    return
  }
  const hello = await call('hello', {
    protocol: PROTOCOL,
    quorfloatVersion: VERSION,
    platform: process.platform,
    arch: process.arch,
    // The same build inventory the real binary declares. It matters now: the plugin
    // claims an interaction only when the peer says it can render that kind, so a
    // mock that under-declared would silently exercise the deferral path instead of
    // the claim path it is standing in for.
    capabilities: ['window', 'hotkey', 'tauri', 'approval'],
    hotkey: { requested: 'Alt+Space', registered: MODE !== 'no-hotkey' },
  })
  observed.helloAnswered = true
  note('handshake', hello)
  if (MODE === 'exit-early') {
    finish(0)
    return
  }
  ready = true
  for (const queued of queue.splice(0)) handle(queued)
}

/** True for a JSON-RPC response frame (an id and no method). */
function isResponse(message) {
  return typeof message.id === 'number' && message.method === undefined
}

/** Handle one inbound frame. */
function handle(message) {
  if (process.env['DSH_QUORFLOAT_MOCK_TRACE'] === '1') {
    note('inbound', JSON.stringify({ id: message.id, method: message.method, hasResult: 'result' in message }))
  }
  if (typeof message.id === 'number' && message.method === undefined) {
    const entry = pending.get(message.id)
    if (entry === undefined) return
    pending.delete(message.id)
    if (message.error) entry.reject(new Error(`${entry.method}: ${message.error.message}`))
    else entry.resolve(message.result)
    return
  }
  if (typeof message.method !== 'string') return
  observed.requests.push(message.method)
  if (typeof message.id !== 'number') {
    handleNotification(message)
    return
  }
  switch (message.method) {
    case 'ping':
      observed.pings += 1
      if (MODE === 'silent') return
      reply(message.id, { pong: Date.now() })
      return
    case 'shutdown':
      observed.shutdownReceived = true
      if (MODE === 'stubborn') return
      reply(message.id, { ok: true })
      // Exiting right after acknowledging is the contract the host relies on to
      // keep shutdown inside its grace budget.
      setTimeout(() => finish(0), 5)
      return
    default:
      replyError(message.id, -32601, `mock does not implement ${message.method}`)
  }
}

/** Handle one notification from the host. */
function handleNotification(message) {
  switch (message.method) {
    case 'host/heartbeat':
      observed.heartbeats += 1
      return
    case 'ready':
      observed.readyReceived = true
      note('ready received', message.params?.config?.hotkey ?? '(no hotkey)')
      return
    default:
      return
  }
}

/** Write the observation report. Diagnostics only; never changes the exit path. */
function writeReport() {
  removePidFile()
  if (!REPORT) return
  try {
    writeFileSync(REPORT, JSON.stringify(observed, null, 2))
  } catch {
    // ignore
  }
}

/** Write the observation report and exit. */
function finish(code) {
  writeReport()
  process.exit(code)
}

process.on('uncaughtException', error => {
  note('uncaught exception', error?.stack ?? String(error))
  finish(1)
})

process.on('SIGTERM', () => {
  // A signal is one of the documented ways this process can end; the report is
  // written either way so a test can tell "stopped cooperatively" from
  // "had to be signalled".
  observed.signalled = 'SIGTERM'
  writeReport()
  process.exit(143)
})

writePidFile()
main().catch(error => {
  note('fatal', error.message)
  finish(1)
})

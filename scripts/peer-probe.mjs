#!/usr/bin/env node
/**
 * A scripted quorfloat peer, for verifying the host plugin inside a real dsh.
 *
 * The Rust subproject does not exist yet, but the protocol does — and the host
 * plugin is the side that owns the Harness connection. This script speaks the
 * peer side of `protocol/README.md` over its own stdin/stdout, so the whole
 * session and interaction chain can be exercised against a real Harness
 * without waiting for any Rust code.
 *
 * It is a developer tool, not part of the shipped package.
 *
 * Usage (from the host plugin's point of view it is simply "the quorfloat
 * executable", so it is selected with the documented environment override):
 *
 *   DSH_QUORFLOAT_PATH=/abs/path/to/scripts/peer-probe.mjs \
 *   DSH_QUORFLOAT_PEER_MODE=auto \
 *   dsh --profile quorfloat-dev
 *
 * Modes (`DSH_QUORFLOAT_PEER_MODE`):
 * - `auto`    (default) run the handshake, then the scripted probe below, then
 *             keep serving frames so a human can watch real interactions;
 * - `handshake` only introduce itself and stay alive;
 * - `hold`    handshake, then start nothing: used to check that an idle peer
 *             stays alive and is cleaned up on exit.
 *
 * All narration goes to **stderr**; stdout carries protocol frames only, which
 * is the rule the host enforces by rejecting any other line.
 */

import { writeFileSync } from 'node:fs'

const MODE = process.env['DSH_QUORFLOAT_PEER_MODE'] ?? 'auto'
/** Always leave a report somewhere findable, even when nothing was configured. */
const REPORT = process.env['DSH_QUORFLOAT_PEER_REPORT'] ?? '/tmp/dsh-quorfloat-probe.json'
const PROMPT = process.env['DSH_QUORFLOAT_PEER_PROMPT'] ??
  '请用一句话说明你收到了这条消息，然后运行 `echo dsh-quorfloat-probe` 并给出它的输出。'

/** What this run observed, written to the report file at exit. */
const observed = {
  mode: MODE,
  pid: process.pid,
  handshake: null,
  ready: false,
  workspaces: null,
  sessionId: null,
  promptAccepted: null,
  events: [],
  streams: 0,
  interactions: [],
  answered: [],
  stderr: [],
}

let nextId = 1
let buffer = ''
const pending = new Map()
let ready = false
const queued = []
const sessionEvents = []

/** Write one protocol frame. This is the only stdout writer in this file. */
function send(message) {
  process.stdout.write(`${JSON.stringify(message)}\n`)
}

/** Narrate to stderr, which the host forwards into the dsh log. */
function note(...args) {
  const line = args.map(value => (typeof value === 'string' ? value : JSON.stringify(value))).join(' ')
  observed.stderr.push(line)
  process.stderr.write(`[peer-probe] ${line}\n`)
}

/** Call one host method and await its result. */
function call(method, params, timeoutMs = 15000) {
  const id = nextId++
  const answer = new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      if (pending.delete(id)) reject(new Error(`${method} timed out after ${timeoutMs}ms`))
    }, timeoutMs)
    timer.unref?.()
    pending.set(id, { resolve, reject, method, timer })
  })
  send(params === undefined ? { jsonrpc: '2.0', id, method } : { jsonrpc: '2.0', id, method, params })
  return answer
}

/** Answer one host request. */
function reply(id, result) {
  send({ jsonrpc: '2.0', id, result })
}

/** Answer one host request with an error. */
function replyError(id, code, message) {
  send({ jsonrpc: '2.0', id, error: { code, message } })
}

/** True for a JSON-RPC response frame. */
function isResponse(message) {
  return typeof message.id === 'number' && message.method === undefined
}

/** Route one inbound frame. */
function handle(message) {
  if (isResponse(message)) {
    const entry = pending.get(message.id)
    if (entry === undefined) return
    pending.delete(message.id)
    clearTimeout(entry.timer)
    if (message.error) entry.reject(new Error(`${entry.method}: ${message.error.message}`))
    else entry.resolve(message.result)
    return
  }
  if (typeof message.method !== 'string') return
  if (typeof message.id === 'number') {
    switch (message.method) {
      case 'ping':
        reply(message.id, { pong: Date.now() })
        return
      case 'shutdown':
        note('host asked us to shut down')
        reply(message.id, { ok: true })
        setTimeout(() => finish(0), 20)
        return
      default:
        replyError(message.id, -32601, `peer-probe does not implement ${message.method}`)
    }
    return
  }
  handleNotification(message)
}

/** React to one host notification. */
function handleNotification(message) {
  const params = message.params ?? {}
  switch (message.method) {
    case 'ready':
      observed.ready = true
      note('handshake complete; host config:', JSON.stringify(params.config ?? {}))
      return
    case 'host/heartbeat':
      return
    case 'host/config':
      note('host pushed a configuration change:', JSON.stringify(params))
      return
    case 'session/snapshot':
      observed.events.push({ kind: 'snapshot', sessionId: params.sessionId, cursor: params.cursor, records: params.records?.length ?? 0 })
      note(`snapshot for ${params.sessionId}: cursor=${params.cursor} records=${params.records?.length ?? 0} hasMore=${params.hasMore}`)
      return
    case 'session/event': {
      observed.events.push({ kind: 'event', seq: params.seq, type: params.type })
      sessionEvents.push(params)
      note(`event #${params.seq} ${params.type}`)
      return
    }
    case 'session/stream':
      observed.streams += 1
      return
    case 'session/resync':
      note('*** host asked for a resync:', JSON.stringify(params))
      return
    case 'interaction/open':
      void onInteraction(params)
      return
    default:
      note('unhandled notification', message.method)
  }
}

/**
 * Answer one pending approval or question.
 *
 * Both are answered automatically here so the probe can drive a full turn
 * unattended: an approval would otherwise block on a human and the scripted
 * sequence would sit forever. Set `DSH_QUORFLOAT_PEER_ANSWER=manual` to leave
 * approvals open and answer them yourself in the floating panel instead.
 */
async function onInteraction(params) {
  const manual = process.env['DSH_QUORFLOAT_PEER_ANSWER'] === 'manual'
  observed.interactions.push({ interactionId: params.interactionId, kind: params.kind, payload: params.payload })
  if (params.kind === 'approval') {
    note(`*** approval requested for tool "${params.payload?.toolName}": ${params.payload?.reason ?? '(no reason)'}`)
    if (manual) {
      note('leaving it for the human (DSH_QUORFLOAT_PEER_ANSWER=manual)')
      return
    }
    const answer = { kind: 'approval', outcome: 'allowed-once' }
    const result = await call('interaction/answer', { interactionId: params.interactionId, answer })
    observed.answered.push({ interactionId: params.interactionId, answer, result })
    note('answered the approval with allowed-once ->', JSON.stringify(result))
    return
  }
  const questions = params.payload?.questions ?? []
  note(`*** ${questions.length} question(s) to answer`)
  const answers = questions.map(question => ({
    id: question.id,
    selected: question.options?.length ? [question.options[0].label] : [],
    ...(question.options?.length ? {} : { custom: 'peer-probe auto answer' }),
  }))
  const result = await call('interaction/answer', { interactionId: params.interactionId, answer: { kind: 'question', answers } })
  observed.answered.push({ interactionId: params.interactionId, answer: { kind: 'question', answers }, result })
  note('answered the questions ->', JSON.stringify(result))
}

/** Sleep for a number of milliseconds. */
function sleep(ms) {
  return new Promise(resolve => setTimeout(resolve, ms))
}

/**
 * Wait for a durable event matching a predicate, or give up.
 *
 * @returns the matching event, or `undefined` on timeout.
 */
async function waitForEvent(predicate, timeoutMs, description) {
  const deadline = Date.now() + timeoutMs
  for (;;) {
    const found = sessionEvents.find(predicate)
    if (found !== undefined) return found
    if (Date.now() > deadline) {
      note(`timed out waiting for ${description}`)
      return undefined
    }
    await sleep(100)
  }
}

/** The scripted probe: everything the host plugin must be able to do. */
async function probe() {
  // 1. Workspaces.
  const workspaces = await call('workspaces/list', {})
  observed.workspaces = workspaces
  note(`workspaces: ${workspaces.items?.length ?? 0}`)
  for (const item of workspaces.items ?? []) note(`  - ${item.workspaceId}  ${item.title}  ${item.path}`)

  // 2. Sessions already known to Harness.
  const sessions = await call('sessions/list', {})
  note(`sessions: ${sessions.items?.length ?? 0}`)

  const workspaceId = workspaces.items?.[0]?.workspaceId
  if (workspaceId === undefined) {
    note('no workspace is registered; create one in dsh first, then re-run')
    return
  }

  // 3. Create a session in that workspace.
  const created = await call('session/create', { workspaceId })
  observed.sessionId = created.sessionId
  note(`created session ${created.sessionId} in ${workspaceId}`)

  // 4. Follow it: the snapshot must arrive before any event is accepted.
  await call('session/attach', { sessionId: created.sessionId })
  await sleep(300)

  // 5. History reads must work even on an empty session.
  const page = await call('session/history', { sessionId: created.sessionId })
  note(`history page: ${page.records?.length ?? 0} record(s), hasMore=${page.hasMore}`)

  // 6. Send a prompt. This is where a real model call starts, and where an
  //    approval can appear — which is exactly what the panel has to survive.
  const requestId = `probe-${Date.now().toString(36)}`
  const accepted = await call('session/prompt', { sessionId: created.sessionId, requestId, text: PROMPT })
  observed.promptAccepted = accepted
  note(`prompt accepted: ${JSON.stringify(accepted)} (requestId=${requestId})`)

  // A repeat of the same request id must be an idempotent replay, not a second
  // prompt: this is the property that keeps a reconnect from resending input.
  const replayed = await call('session/prompt', { sessionId: created.sessionId, requestId, text: PROMPT })
  note(`replayed the same requestId -> ${JSON.stringify(replayed)} (must not enqueue twice)`)

  const finished = await waitForEvent(event => event.type === 'turn/end', 120000, 'the turn to end')
  if (finished !== undefined) {
    note(`turn ended: ${JSON.stringify(finished.data)}`)
  }

  // 7. A second prompt, cancelled immediately — verifies the cancel path and
  //    that the host reports a cancelling state rather than a terminal one.
  const cancelRequestId = `probe-cancel-${Date.now().toString(36)}`
  await call('session/prompt', { sessionId: created.sessionId, requestId: cancelRequestId, text: '请从 1 数到 200，每个数字单独一行。' })
  await sleep(400)
  const cancelled = await call('session/cancel', { sessionId: created.sessionId })
  note(`cancel requested: ${JSON.stringify(cancelled)}`)
  const abortEvent = await waitForEvent(
    event => event.type === 'turn/end' && event.data?.reason?.kind === 'aborted',
    30000,
    'the aborted turn/end',
  )
  note(abortEvent === undefined ? 'no aborted turn/end observed' : 'observed aborted turn/end')

  note('scripted probe finished; staying alive to serve interactions')
}

/** Write the report and exit. */
function finish(code) {
  try {
    writeFileSync(REPORT, JSON.stringify({ ...observed, exitCode: code, endedAt: Date.now() }, null, 2))
    note(`report written to ${REPORT}`)
  } catch (error) {
    note('could not write the report:', error?.message ?? String(error))
  }
  process.exit(code)
}

/** Start reading stdin before anything else: an unread pipe does not hold the event loop. */
/**
 * Give up on an unterminated frame once it is implausibly long.
 *
 * Without this, a host that writes a partial frame and never terminates it would
 * leave every following frame stuck behind it in this buffer: the peer would look
 * alive while answering nothing. The host uses the same 1 MiB ceiling, so the two
 * sides agree on what "too long" means.
 */
const MAX_FRAME_BYTES = 1024 * 1024

process.stdin.setEncoding('utf8')
process.stdin.on('data', chunk => {
  buffer += chunk
  for (;;) {
    const newline = buffer.indexOf('\n')
    if (newline === -1) {
      if (buffer.length > MAX_FRAME_BYTES) {
        note(`dropping ${buffer.length} bytes of unterminated frame data`)
        buffer = ''
      }
      break
    }
    const line = buffer.slice(0, newline)
    buffer = buffer.slice(newline + 1)
    if (line.trim() === '') continue
    let message
    try {
      message = JSON.parse(line)
    } catch {
      note('host sent a frame this peer could not parse')
      continue
    }
    // Responses are settled immediately; anything else waits for the handshake.
    if (ready || isResponse(message)) handle(message)
    else queued.push(message)
  }
})
process.stdin.on('end', () => {
  note('stdin closed (host is gone); exiting')
  finish(0)
})

process.on('uncaughtException', error => {
  note('uncaught exception', error?.stack ?? String(error))
  finish(1)
})
process.on('unhandledRejection', error => {
  note('unhandled rejection', error?.stack ?? String(error))
})

/** Run the handshake, then the selected mode. */
async function main() {
  note('starting', `mode=${MODE}`)
  const handshake = await call('hello', {
    protocol: 'quorfloat/1',
    quorfloatVersion: '0.0.1-peer-probe',
    platform: process.platform,
    arch: process.arch,
    capabilities: ['window', 'hotkey', 'probe'],
    hotkey: { requested: 'Alt+Space', registered: false },
  })
  observed.handshake = handshake
  note('handshake answered:', JSON.stringify(handshake))
  ready = true
  for (const message of queued.splice(0)) handle(message)
  if (MODE === 'handshake' || MODE === 'hold') {
    note(`mode ${MODE}: staying alive without probing`)
    return
  }
  try {
    await probe()
  } catch (error) {
    note('the probe failed:', error?.message ?? String(error))
  }
}

main().catch(error => {
  note('fatal', error?.message ?? String(error))
  finish(1)
})

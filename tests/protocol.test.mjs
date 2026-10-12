// Category 1 (cross-process protocol): NDJSON framing and JSON-RPC message classification.
/**
 * Framing and message-classification tests.
 *
 * These are the cases that break a line-delimited protocol in practice: a frame
 * split across reads, several frames in one read, a peer that writes something
 * that is not JSON, and a peer that never sends a newline. None of them may
 * crash the host, and each must be labelled so the log says which happened.
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'

import { loadModule, waitFor } from './helpers.mjs'

const { FrameDecoder, classify, encodeFrame, request, notification, success, failure, ErrorCode, PROTOCOL_VERSION, MAX_FRAME_BYTES } =
  await loadModule('protocol.js')

/** Collect frames from a sequence of chunks. */
function decode(chunks, maxBytes) {
  const decoder = new FrameDecoder(maxBytes)
  return chunks.flatMap(chunk => decoder.push(chunk))
}

test('a complete frame decodes into a classified request', () => {
  const frames = decode([encodeFrame(request(1, 'ping'))])
  assert.equal(frames.length, 1)
  assert.equal(frames[0].ok, true)
  assert.equal(frames[0].kind, 'request')
  assert.equal(frames[0].message.method, 'ping')
  assert.equal(frames[0].message.id, 1)
})

test('one frame split across three reads is reassembled exactly once', () => {
  const frame = encodeFrame(request(7, 'session/prompt', { text: '你好 world' }))
  const cut1 = 5
  const cut2 = 17
  const frames = decode([frame.slice(0, cut1), frame.slice(cut1, cut2), frame.slice(cut2)])
  assert.equal(frames.length, 1)
  assert.equal(frames[0].message.params.text, '你好 world', 'multi-byte characters survive chunk boundaries')
})

test('several frames in a single read are emitted separately, in order', () => {
  const payload = encodeFrame(notification('session/event', { seq: 1 })) +
    encodeFrame(notification('session/event', { seq: 2 })) +
    encodeFrame(success(3, { ok: true }))
  const frames = decode([payload])
  assert.equal(frames.length, 3)
  assert.deepEqual(frames.map(frame => frame.kind), ['notification', 'notification', 'success'])
  assert.deepEqual(frames.slice(0, 2).map(frame => frame.message.params.seq), [1, 2])
})

test('a trailing carriage return is tolerated as a line terminator', () => {
  const frames = decode(['{"jsonrpc":"2.0","method":"ping"}\r\n'])
  assert.equal(frames.length, 1)
  assert.equal(frames[0].ok, true)
  assert.equal(frames[0].message.method, 'ping')
})

test('a byte-split inside a multi-byte character still decodes', () => {
  const frame = Buffer.from(encodeFrame(notification('log', { text: '模型' })), 'utf8')
  const frames = []
  const decoder = new FrameDecoder()
  for (const byte of frame) frames.push(...decoder.push(Buffer.from([byte])))
  assert.equal(frames.length, 1)
  assert.equal(frames[0].message.params.text, '模型')
})

test('invalid JSON is rejected as invalid-json and does not throw', () => {
  const frames = decode(['{not json}\n'])
  assert.equal(frames.length, 1)
  assert.equal(frames[0].ok, false)
  assert.equal(frames[0].rejection.kind, 'invalid-json')
})

test('valid JSON that is not a JSON-RPC message is rejected as invalid-message', () => {
  const cases = [
    '{"jsonrpc":"1.0","method":"ping"}\n',
    '{"jsonrpc":"2.0"}\n',
    '{"jsonrpc":"2.0","id":"text","method":"ping"}\n',
    '{"jsonrpc":"2.0","id":1,"result":{},"error":{"code":1}}\n',
    '{"jsonrpc":"2.0","id":1,"error":{"message":"no code"}}\n',
    '[1,2,3]\n',
    '"a string"\n',
  ]
  for (const payload of cases) {
    const frames = decode([payload])
    assert.equal(frames.length, 1, payload)
    assert.equal(frames[0].ok, false, payload)
    assert.equal(frames[0].rejection.kind, 'invalid-message', payload)
  }
})

test('an empty line is reported instead of being silently skipped', () => {
  const frames = decode(['\n'])
  assert.equal(frames.length, 1)
  assert.equal(frames[0].ok, false)
  assert.equal(frames[0].rejection.kind, 'invalid-message')
})

test('invalid UTF-8 is rejected as invalid-json', () => {
  const frames = decode([Buffer.concat([Buffer.from('{"a":"'), Buffer.from([0xff, 0xfe]), Buffer.from('"}\n')])])
  assert.equal(frames.length, 1)
  assert.equal(frames[0].ok, false)
  assert.equal(frames[0].rejection.kind, 'invalid-json')
  assert.match(frames[0].rejection.detail, /UTF-8/)
})

test('a frame longer than the ceiling is dropped without unbounded buffering', () => {
  const decoder = new FrameDecoder(64)
  const frames = decoder.push('x'.repeat(200))
  assert.equal(frames.length, 1)
  assert.equal(frames[0].rejection.kind, 'oversized')
  assert.equal(decoder.bufferedBytes, 0, 'the oversized partial frame is discarded, not retained')
})

test('a complete oversized frame is dropped but later frames still decode', () => {
  const big = `${JSON.stringify({ jsonrpc: '2.0', method: 'x', params: { pad: 'y'.repeat(200) } })}\n`
  const frames = decode([big + encodeFrame(request(2, 'ping'))], 64)
  assert.equal(frames.length, 2)
  assert.equal(frames[0].rejection.kind, 'oversized')
  assert.equal(frames[1].ok, true)
  assert.equal(frames[1].message.id, 2)
})

test('an unterminated partial frame is retained until its newline arrives', () => {
  const decoder = new FrameDecoder()
  const frame = encodeFrame(request(9, 'ping'))
  assert.deepEqual(decoder.push(frame.slice(0, -1)), [])
  assert.ok(decoder.bufferedBytes > 0)
  const frames = decoder.push('\n')
  assert.equal(frames.length, 1)
  assert.equal(frames[0].message.id, 9)
  assert.equal(decoder.bufferedBytes, 0)
})

test('encodeFrame escapes embedded newlines so one message stays one frame', () => {
  const encoded = encodeFrame(request(1, 'session/prompt', { text: 'line1\nline2' }))
  assert.equal(encoded.split('\n').length, 2, 'exactly one terminator')
  const frames = decode([encoded])
  assert.equal(frames[0].message.params.text, 'line1\nline2')
})

test('message constructors produce the documented shapes', () => {
  assert.deepEqual(request(1, 'ping'), { jsonrpc: '2.0', id: 1, method: 'ping' })
  assert.deepEqual(request(1, 'ping', { a: 1 }), { jsonrpc: '2.0', id: 1, method: 'ping', params: { a: 1 } })
  assert.deepEqual(notification('ready'), { jsonrpc: '2.0', method: 'ready' })
  assert.deepEqual(success(2, { ok: true }), { jsonrpc: '2.0', id: 2, result: { ok: true } })
  assert.deepEqual(failure(3, ErrorCode.Unavailable, 'nope'), {
    jsonrpc: '2.0',
    id: 3,
    error: { code: ErrorCode.Unavailable, message: 'nope' },
  })
  assert.equal(PROTOCOL_VERSION, 'quorfloat/1')
  assert.ok(MAX_FRAME_BYTES >= 1024)
})

test('classify accepts a valid response with a null id', () => {
  const classified = classify({ jsonrpc: '2.0', id: null, error: { code: -32700, message: 'parse error' } })
  assert.ok(!('detail' in classified))
  assert.equal(classified.kind, 'failure')
})

// Protocol contract: the chunked snapshot. One frame used to carry a whole
// conversation, and a conversation past the 1 MiB ceiling was dropped by the reader
// before any of it could be folded — which left the panel blank for the rest of the
// run. The sequence below is the framing that replaces it.
test('a snapshot larger than one frame is emitted as begin, bounded parts, and end', async () => {
  const { SessionLayer, SNAPSHOT_PART_BUDGET_BYTES } = await loadModule('harness/session-layer.js')
  // The measured shape of the failure: a 50-message window around 1.3 MB. The text is
  // CJK on purpose — three bytes per character — so a budget enforced on
  // `String.length` would pass a size assertion while shipping a frame three times the
  // ceiling, which is exactly the mistake this protocol rule exists to prevent.
  const records = Array.from({ length: 50 }, (_, index) => ({
    type: 'event',
    event: {
      type: 'assistant/message',
      seq: index,
      time: index,
      data: { message: { role: 'assistant', content: [{ type: 'text', text: '中'.repeat(8_000) }] } },
    },
  }))
  const snapshot = {
    type: 'snapshot',
    cursor: 49,
    hasMore: false,
    records,
    projections: null,
    assistantStream: null,
  }
  const notifications = []
  const layer = new SessionLayer({
    harness: {
      follow(sessionId, onFrame) {
        queueMicrotask(() => onFrame(snapshot))
        return { close() {}, done: new Promise(() => {}) }
      },
      stats: async () => undefined,
    },
    notify: async (method, params) => {
      notifications.push({ method, params })
    },
    defaultWorkspaceId: () => '',
    log: { warn() {}, debug() {} },
  })
  await layer.attach('session-1')
  await waitFor('the snapshot sequence to finish', async () =>
    notifications.some(entry => entry.method === 'session/snapshot/end'))

  assert.equal(notifications[0].method, 'session/snapshot/begin', 'the identity and meta come first')
  assert.equal(notifications.at(-1).method, 'session/snapshot/end', 'and the sequence is closed')

  const parts = notifications.filter(entry => entry.method === 'session/snapshot/part')
  assert.ok(parts.length > 1, `the snapshot was chunked, not sent as one frame (${parts.length} part(s))`)
  assert.equal(notifications[0].params.parts, parts.length, 'begin declares how many parts follow')
  assert.equal(notifications[0].params.sessionId, 'session-1')

  const whole = Buffer.byteLength(JSON.stringify(records))
  assert.ok(whole > MAX_FRAME_BYTES, `the conversation really is past the ceiling: ${whole} bytes`)
  for (const [index, part] of parts.entries()) {
    assert.equal(part.params.index, index, 'parts are numbered in order')
    assert.equal(part.params.sessionId, 'session-1', 'every part names the conversation it belongs to')
    const bytes = Buffer.byteLength(JSON.stringify(part.params))
    assert.ok(
      bytes <= SNAPSHOT_PART_BUDGET_BYTES,
      `part ${index} is ${bytes} bytes, over the ${SNAPSHOT_PART_BUDGET_BYTES}-byte budget`,
    )
    assert.ok(bytes < MAX_FRAME_BYTES, `part ${index} must stay under the frame ceiling`)
  }

  // Nothing may be lost or reordered by the split: the peer reassembles these in order.
  assert.deepEqual(parts.flatMap(part => part.params.records), records)
})

// Protocol contract: the older page. It travels as one JSON-RPC response, so it is bounded by
// the same 1 MiB frame ceiling — and the peer's reader drops an oversized frame before anything
// can fold it, which for a page means the reader's request silently never lands.
test('an older page is bounded to one frame, keeping the records nearest the window', async () => {
  const { boundedHistoryPage, HISTORY_PAGE_BUDGET_BYTES } = await loadModule('harness/session-layer.js')
  // CJK text on purpose, as in the snapshot test: a budget enforced on `String.length` would
  // pass here while shipping frames three times the intended size.
  const record = seq => ({
    type: 'event',
    event: {
      type: 'assistant/message',
      seq,
      time: seq,
      data: { message: { role: 'assistant', content: [{ type: 'text', text: '中'.repeat(4_000) }] } },
    },
  })
  const records = Array.from({ length: 100 }, (_, index) => record(index))

  const bounded = boundedHistoryPage({ records, hasMore: false, cursor: 0 })
  assert.ok(bounded.records.length > 0, 'a page always carries something')
  assert.ok(
    Buffer.byteLength(JSON.stringify(bounded.records)) <= HISTORY_PAGE_BUDGET_BYTES,
    `the page fits the budget: ${Buffer.byteLength(JSON.stringify(bounded.records))} bytes`,
  )
  assert.ok(bounded.records.length < records.length, 'the page really was trimmed')
  // The newest end survives: the panel extends its window upwards, so what arrives has to be
  // contiguous with what is already on screen, or the window would have a hole in it.
  assert.deepEqual(bounded.records.at(-1), records.at(-1))
  assert.equal(bounded.hasMore, true, 'the records left behind are still reachable')
  assert.equal(
    bounded.cursor,
    bounded.records[0].event.seq,
    'and paging continues below the oldest record that travelled',
  )

  // A page that fits is handed back untouched, cursor and all — the trim is not a rewrite.
  const small = { records: records.slice(0, 2), hasMore: false, cursor: 1 }
  assert.equal(boundedHistoryPage(small), small)
  assert.equal(boundedHistoryPage({ records: [], hasMore: false, cursor: null }).cursor, null)
})

// Protocol contract: the per-record clamp. The page budget above is not, on its own, a bound on
// a *record*: one record larger than the page budget would travel as an oversized frame, and the
// peer's reader drops that frame before anything can fold it — for a page, the reader's request
// then simply times out. This is the same failure that made long snapshots blank, one level down.
test('a record larger than the per-record budget is clamped, not sent oversized', async () => {
  const { boundedHistoryPage, HISTORY_PAGE_BUDGET_BYTES, HISTORY_RECORD_BUDGET_BYTES } =
    await loadModule('harness/session-layer.js')
  const huge = {
    type: 'event',
    event: {
      type: 'assistant/message',
      seq: 7,
      time: 7,
      data: { message: { role: 'assistant', content: [{ type: 'text', text: '中'.repeat(400_000) }] } },
    },
  }
  assert.ok(
    Buffer.byteLength(JSON.stringify(huge)) > HISTORY_PAGE_BUDGET_BYTES,
    'the fixture really is one record past the whole page budget',
  )

  const bounded = boundedHistoryPage({ records: [huge], hasMore: false, cursor: 7 })
  const record = bounded.records[0]
  const bytes = Buffer.byteLength(JSON.stringify(record))
  assert.ok(
    bytes <= HISTORY_RECORD_BUDGET_BYTES,
    `the record must fit its own budget: ${bytes} > ${HISTORY_RECORD_BUDGET_BYTES}`,
  )
  assert.ok(
    Buffer.byteLength(JSON.stringify(bounded.records)) <= HISTORY_PAGE_BUDGET_BYTES,
    'and so the page fits one frame',
  )
  // The envelope is what every consumer of a page places a record by: the backwards cursor
  // (`event.seq`), the fold (`event.type`), and the forward watermark.
  assert.equal(record.type, 'event', 'the record envelope survives')
  assert.equal(record.event.type, 'assistant/message', 'the event kind survives, so the transcript can fold it')
  assert.equal(record.event.seq, 7, 'the sequence survives, so paging continues below it')
  assert.equal(bounded.cursor, 7)
  // The cut is stated in the text, exactly as the transcript's own per-entry clip states it: a
  // clipped answer that looks complete is worse than a long one.
  assert.match(record.event.data.message.content[0].text, /已截断 \d+ 字节/)
})

test('a record that cannot be clipped into the budget travels as its envelope, never oversized', async () => {
  const { boundedHistoryPage, HISTORY_RECORD_BUDGET_BYTES } = await loadModule('harness/session-layer.js')
  // The structural case: thousands of short fields whose *shape* is over budget and whose every
  // string is too short to be worth clipping (they are identity-sized, and clipping one would
  // turn a record this build folds into one it counts as unrecognised).
  const data = {}
  for (let index = 0; index < 4_000; index += 1) data[`field-${index}`] = 'x'.repeat(8)
  const record = { type: 'event', event: { type: 'assistant/message', seq: 9, time: 9, data } }

  const bounded = boundedHistoryPage({ records: [record], hasMore: false, cursor: 9 })
  const sent = bounded.records[0]
  assert.ok(
    Buffer.byteLength(JSON.stringify(sent)) <= HISTORY_RECORD_BUDGET_BYTES,
    'the record still fits its budget, however it had to get there',
  )
  assert.equal(sent.event.type, 'assistant/message')
  assert.equal(sent.event.seq, 9, 'the envelope is kept: it is the half every consumer needs')
  assert.equal(sent.event.data.truncated, true, 'and the payload says what happened to it')
  assert.equal(bounded.cursor, 9)
})

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

import { loadModule } from './helpers.mjs'

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

// Category 1 (cross-process protocol): large JSON-RPC messages survive the 4 MiB frame
// ceiling by fragmenting their serialized form and reassembling it before classification.

import assert from 'node:assert/strict'
import { test } from 'node:test'

import { loadModule } from './helpers.mjs'

const { FragmentAssembler, fragmentMessage } = await loadModule('bridge/fragments.js')
const { FRAGMENT_RAW_BYTES, MAX_LOGICAL_MESSAGE_BYTES } = await loadModule('protocol.js')

test('a message over one frame round-trips through fragments', () => {
  const original = {
    jsonrpc: '2.0',
    method: 'session/event',
    params: { text: 'x'.repeat(FRAGMENT_RAW_BYTES + 1024) },
  }
  const fragments = fragmentMessage(original, 'f-1')
  assert.equal(fragments.length, 2)
  assert.ok(fragments.every(fragment => fragment.method === 'quorfloat/fragment'))
  const assembler = new FragmentAssembler()
  assert.equal(assembler.accept(fragments[0].params).kind, 'pending')
  const done = assembler.accept(fragments[1].params)
  assert.equal(done.kind, 'complete')
  assert.deepEqual(done.frame.message, original)
})

test('an out-of-order or duplicate fragment is refused', () => {
  const original = { jsonrpc: '2.0', method: 'x', params: { text: 'x'.repeat(FRAGMENT_RAW_BYTES + 1) } }
  const fragments = fragmentMessage(original, 'f-2')
  const assembler = new FragmentAssembler()
  assert.equal(assembler.accept(fragments[1].params).kind, 'rejected')
})

test('a message over the logical ceiling is refused before any fragment is sent', () => {
  const original = { jsonrpc: '2.0', method: 'x', params: { text: 'x'.repeat(MAX_LOGICAL_MESSAGE_BYTES + 1) } }
  assert.throws(() => fragmentMessage(original, 'f-3'), /exceeds/)
})

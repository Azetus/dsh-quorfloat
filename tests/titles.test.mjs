// The fold that names a conversation, copied from the Harness's own
// (`foldSessionTitle`, packages/session/session-title/src/index.ts:282).
//
// It is worth a test of its own because it decides what the panel's conversation list says: the
// wrong field would put a paragraph where a title belongs, and no field at all would leave the
// list as session ids.
import assert from 'node:assert/strict'
import test from 'node:test'

const { foldSessionTitle } = await import(new URL('../lib/harness/adapter.js', import.meta.url))

test('the latest title event wins, as the harness folds it', () => {
  assert.equal(
    foldSessionTitle([
      { type: 'user/message', data: { content: [] } },
      { type: 'session/title', data: { title: '第一次的名字', source: 'auto' } },
      { type: 'assistant/message', data: {} },
      { type: 'session/title', data: { title: '后来的名字', source: 'user' } },
    ]),
    '后来的名字',
  )
})

test('a conversation that was never named has no title', () => {
  assert.equal(foldSessionTitle([]), undefined)
  assert.equal(foldSessionTitle([{ type: 'user/message', data: {} }]), undefined)
})

test('a shape this build does not recognise yields no title rather than a wrong one', () => {
  // The field the Harness uses is `data.title`; anything else is a shape change, and a paragraph
  // under a conversation's name is worse than its id.
  assert.equal(foldSessionTitle([{ type: 'session/title', data: { text: '不是 title 字段' } }]), undefined)
  assert.equal(foldSessionTitle([{ type: 'session/title', data: { name: '也不是' } }]), undefined)
  assert.equal(foldSessionTitle([{ type: 'session/title', data: { title: 42 } }]), undefined)
  assert.equal(foldSessionTitle([{ type: 'session/title', data: { title: '' } }]), undefined)
  assert.equal(foldSessionTitle([{ type: 'session/title' }]), undefined)
})

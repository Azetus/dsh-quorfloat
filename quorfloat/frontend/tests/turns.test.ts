import { describe, expect, test } from 'vitest'
import { foldTurns } from '../src/lib/turns'
import type { Block, Entry } from '../src/lib/state'

const user = (text: string): Entry => ({ kind: 'user', text })
const text = (text: string, streaming = false): Entry => ({
  kind: 'assistant',
  streaming,
  blocks: [{ kind: 'text', text }],
})
const reasoning = (text: string): Entry => ({
  kind: 'assistant',
  streaming: false,
  blocks: [{ kind: 'reasoning', text }],
})
const call = (name: string, args = '{}'): Entry => ({
  kind: 'assistant',
  streaming: false,
  blocks: [{ kind: 'call', name, arguments: args }],
})
const mixed = (blocks: Block[]): Entry => ({ kind: 'assistant', streaming: false, blocks })
const tool = (name: string, output: string, isError = false): Entry => ({
  kind: 'tool',
  name,
  text: output,
  isError,
})

describe('foldTurns', () => {
  test('a turn is question, one fold, and the final answer', () => {
    const view = foldTurns(
      [user('帮我看看'), mixed([
        { kind: 'reasoning', text: '想一下' },
        { kind: 'text', text: '先看目录' },
      ]), tool('bash', 'exit=0'), text('看到了')],
      null,
      false,
    )
    expect(view.turns).toHaveLength(1)
    const turn = view.turns[0]!
    expect(turn.question).toBe('帮我看看')
    expect(turn.answer?.blocks).toEqual([{ kind: 'text', text: '看到了' }])
    expect(turn.working).toEqual([
      { kind: 'reasoning', name: null, text: '想一下', arguments: null, isError: false },
      { kind: 'text', name: null, text: '先看目录', arguments: null, isError: false },
      { kind: 'tool', name: 'bash', text: 'exit=0', arguments: null, isError: false },
    ])
    expect(turn.inProgress).toBe(false)
  })

  test("the answer message's own reasoning goes into the fold, not beside it", () => {
    const view = foldTurns(
      [user('问题'), mixed([
        { kind: 'reasoning', text: '回答前的推理' },
        { kind: 'text', text: '正式回答' },
      ])],
      null,
      false,
    )
    const turn = view.turns[0]!
    expect(turn.answer?.blocks).toEqual([{ kind: 'text', text: '正式回答' }])
    expect(turn.working).toEqual([{ kind: 'reasoning', name: null, text: '回答前的推理', arguments: null, isError: false }])
  })

  test('a message ending in a tool call is still working, never the answer', () => {
    const view = foldTurns(
      [user('跑一下'), mixed([
        { kind: 'text', text: '我来执行' },
        { kind: 'call', name: 'bash', arguments: '{"command":"ls"}' },
      ]), tool('bash', 'total 0'), text('执行完了')],
      null,
      false,
    )
    const turn = view.turns[0]!
    expect(turn.answer?.blocks).toEqual([{ kind: 'text', text: '执行完了' }])
    expect(turn.working.map(line => line.kind)).toEqual(['text', 'call', 'tool'])
    expect(turn.working[1]).toMatchObject({ kind: 'call', name: 'bash', arguments: '{"command":"ls"}' })
  })

  test('the last text message is the answer; an earlier one is narration', () => {
    const view = foldTurns([user('问'), text('第一段'), text('第二段')], null, false)
    const turn = view.turns[0]!
    expect(turn.answer?.blocks).toEqual([{ kind: 'text', text: '第二段' }])
    expect(turn.working).toEqual([{ kind: 'text', name: null, text: '第一段', arguments: null, isError: false }])
  })

  test('a live stream is the answer under construction; its reasoning folds', () => {
    const view = foldTurns(
      [user('在吗')],
      mixed([
        { kind: 'reasoning', text: '边想边写' },
        { kind: 'text', text: '正在回答' },
      ]),
      true,
    )
    const turn = view.turns[0]!
    expect(turn.answer).toEqual({ blocks: [{ kind: 'text', text: '正在回答' }], streaming: true })
    expect(turn.working).toEqual([{ kind: 'reasoning', name: null, text: '边想边写', arguments: null, isError: false }])
    expect(turn.inProgress).toBe(true)
  })

  test('asking the same question twice makes two turns with different identities', () => {
    const view = foldTurns([user('重试一下'), text('第一次'), user('重试一下'), text('第二次')], null, false)
    expect(view.turns).toHaveLength(2)
    expect(view.turns[0]!.id).not.toBe(view.turns[1]!.id)
    expect(view.turns[0]!.question).toBe('重试一下')
    expect(view.turns[1]!.question).toBe('重试一下')
  })

  test('the fold identity carries the position, so trimming shifts it', () => {
    const entries = [user('一'), text('答一'), user('二'), text('答二')]
    const whole = foldTurns(entries, null, false)
    const trimmed = foldTurns(entries.slice(2), null, false)
    expect(whole.turns[1]!.id).not.toBe(trimmed.turns[0]!.id)
    // Both are "question 二", but at different positions — exactly the rule
    // that makes the fold memory be *forgotten* rather than reassigned.
    expect(trimmed.turns[0]!.question).toBe('二')
  })

  test('a lifecycle line outside any turn is loose, never folded', () => {
    const view = foldTurns(
      [{ kind: 'system', text: '会话已同步' }, user('好'), text('行')],
      null,
      false,
    )
    expect(view.loose).toEqual([{ text: '会话已同步' }])
    expect(view.turns[0]!.working).toEqual([])
  })

  test('a turn is in progress while the host says so, even without a live stream', () => {
    const view = foldTurns([user('慢的'), text('等一下')], null, true)
    expect(view.turns[0]!.inProgress).toBe(true)
  })
})

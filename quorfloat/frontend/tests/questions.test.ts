// The question card's decisions: what counts as answered, what a click does, and what
// goes on the wire. The widgets themselves are verified by eye; these by value.

import { describe, expect, it } from 'vitest'
import {
  answerPayload, emptyDrafts, isComplete, setCustom, toggleChoice, type Question,
} from '../src/lib/questions'

const single: Question = {
  id: 'q1',
  header: '部署目标',
  question: '部署到哪个环境？',
  detail: null,
  options: [
    { label: '预发', description: 'pre' },
    { label: '生产', description: null },
  ],
  multiSelect: false,
}

const multi: Question = { ...single, id: 'q2', multiSelect: true }

const free: Question = { ...single, id: 'q3', options: [] }

describe('emptyDrafts', () => {
  it('gives every question an empty draft', () => {
    const drafts = emptyDrafts([single, multi])
    expect(Object.keys(drafts)).toEqual(['q1', 'q2'])
    expect(drafts['q1']).toEqual({ selected: [], custom: '' })
  })
})

describe('toggleChoice', () => {
  it('replaces the pick for a single-select question', () => {
    const first = toggleChoice(single, emptyDrafts([single]), '预发')
    const second = toggleChoice(single, first, '生产')
    expect(second['q1']?.selected).toEqual(['生产'])
  })

  it('adds and removes picks for a multi-select question', () => {
    const one = toggleChoice(multi, emptyDrafts([multi]), '生产')
    const two = toggleChoice(multi, one, '预发')
    // Offered order, not click order: the payload must not depend on how it was clicked.
    expect(two['q2']?.selected).toEqual(['预发', '生产'])
    const three = toggleChoice(multi, two, '生产')
    expect(three['q2']?.selected).toEqual(['预发'])
  })

  it('keeps the free text already typed', () => {
    const typed = setCustom(single, emptyDrafts([single]), '别的')
    const picked = toggleChoice(single, typed, '预发')
    expect(picked['q1']).toEqual({ selected: ['预发'], custom: '别的' })
  })
})

describe('isComplete', () => {
  it('needs an answer for every question, not just one', () => {
    const questions = [single, multi]
    const half = toggleChoice(single, emptyDrafts(questions), '预发')
    expect(isComplete(questions, half)).toBe(false)
    const whole = toggleChoice(multi, half, '生产')
    expect(isComplete(questions, whole)).toBe(true)
  })

  it('accepts custom text as an answer on its own', () => {
    const drafts = setCustom(free, emptyDrafts([free]), '  灰度  ')
    expect(isComplete([free], drafts)).toBe(true)
  })

  it('does not accept whitespace as an answer', () => {
    const drafts = setCustom(free, emptyDrafts([free]), '   ')
    expect(isComplete([free], drafts)).toBe(false)
  })

  it('is false for a card with no questions', () => {
    expect(isComplete([], {})).toBe(false)
  })
})

describe('answerPayload', () => {
  it('sends one answer per question with trimmed custom text', () => {
    let drafts = emptyDrafts([single, multi])
    drafts = toggleChoice(single, drafts, '预发')
    drafts = toggleChoice(multi, drafts, '生产')
    drafts = setCustom(multi, drafts, '  顺便通知 @ops  ')
    expect(answerPayload([single, multi], drafts)).toEqual([
      { id: 'q1', selected: ['预发'] },
      { id: 'q2', selected: ['生产'], custom: '顺便通知 @ops' },
    ])
  })

  it('omits custom entirely when nothing was typed', () => {
    const drafts = setCustom(single, emptyDrafts([single]), '   ')
    expect(answerPayload([single], drafts)).toEqual([{ id: 'q1', selected: [] }])
  })
})

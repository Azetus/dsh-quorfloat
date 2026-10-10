// The tokens the panel names a workspace or conversation with, and a conversation's age:
// the pure half of the old `render.ts`, now returning dictionary keys and host data rather
// than finished Chinese sentences. The field itself is verified by eye (docs/ui).

import { afterEach, beforeEach, describe, expect, it, test, vi } from 'vitest'
import { msg, raw, resolveText } from '../src/lib/i18n'
import { relativeTime, sessionTitle, targetWorkspace, workspaceTitle } from '../src/lib/view-text'
import type { Snapshot } from '../src/lib/state'

/** A snapshot whose only interesting field is `session`. */
const snap = (session: Record<string, unknown>): Snapshot => ({ session }) as unknown as Snapshot

describe('targetWorkspace', () => {
  it('uses the shell’s answer when the user has picked nothing', () => {
    expect(targetWorkspace(snap({ createWorkspace: 'ws-project' }), null)).toBe('ws-project')
  })

  it('prefers the workspace the user picked for this visit', () => {
    expect(targetWorkspace(snap({ createWorkspace: 'ws-project' }), 'ws-notes')).toBe('ws-notes')
  })

  it('is null when neither knows, which is what asks the user', () => {
    expect(targetWorkspace(snap({ createWorkspace: null }), null)).toBeNull()
  })
})

describe('sessionTitle', () => {
  test('a new-conversation state is a key, not a sentence', () => {
    const title = sessionTitle(snap({ following: null }))
    expect(title).toEqual(msg('session.new'))
    expect(resolveText('zh', title)).toBe('新会话')
    expect(resolveText('en', title)).toBe('New conversation')
  })

  test('the host’s own title is shown verbatim', () => {
    const title = sessionTitle(snap({
      following: { sessionId: 's-1' },
      conversations: [{ sessionId: 's-1', title: '设计评审' }],
    }))
    expect(title).toEqual(raw('设计评审'))
    expect(resolveText('en', title)).toBe('设计评审')
  })

  test('without a title the id speaks', () => {
    const title = sessionTitle(snap({
      following: { sessionId: 'abcdefghijklmnop' },
      conversations: [],
    }))
    expect(title).toEqual(raw('abcdefghijklm'))
  })
})

describe('workspaceTitle', () => {
  test('with nothing known it asks, by key', () => {
    const title = workspaceTitle(snap({ following: null, workspaces: [] }), null)
    expect(title).toEqual(msg('workspace.choose'))
    expect(resolveText('zh', title)).toBe('选择工作区')
    expect(resolveText('en', title)).toBe('Choose a workspace')
  })

  test('the user’s pick resolves through the workspace list', () => {
    const title = workspaceTitle(snap({
      following: null,
      workspaces: [{ workspaceId: 'w-1', title: 'Quorvox', path: '/wg/quorvox' }],
    }), 'w-1')
    expect(title).toEqual(raw('Quorvox'))
  })

  test('a following session’s label is host data', () => {
    const title = workspaceTitle(snap({
      following: { sessionId: 's-1', label: 'notes' },
      conversations: [],
      workspaces: [],
    }), null)
    expect(title).toEqual(raw('notes'))
  })

  test('a following session with no label falls back to the heading key', () => {
    const title = workspaceTitle(snap({
      following: { sessionId: 's-1', label: null },
      conversations: [],
      workspaces: [],
    }), null)
    expect(title).toEqual(msg('workspace.heading'))
    expect(resolveText('en', title)).toBe('Workspace')
  })

  test('the cwd projection wins over the label', () => {
    const title = workspaceTitle(snap({
      following: { sessionId: 's-1', label: 'wrong' },
      conversations: [{ sessionId: 's-1', cwd: '/wg/quorvox' }],
      workspaces: [{ workspaceId: 'w-1', title: 'Quorvox', path: '/wg/quorvox' }],
    }), null)
    expect(title).toEqual(raw('Quorvox'))
  })
})

describe('relativeTime', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date('2026-10-10T14:05:00'))
  })
  afterEach(() => {
    vi.useRealTimers()
  })

  test('under a minute is the now key', () => {
    expect(relativeTime(Date.now() - 30_000)).toEqual(msg('time.now'))
    expect(resolveText('zh', relativeTime(Date.now() - 30_000))).toBe('刚刚')
    expect(resolveText('en', relativeTime(Date.now() - 30_000))).toBe('Just now')
  })

  test('under an hour carries the minutes', () => {
    expect(relativeTime(Date.now() - 5 * 60_000)).toEqual(msg('time.minutesAgo', { minutes: 5 }))
    expect(resolveText('zh', relativeTime(Date.now() - 5 * 60_000))).toBe('5 分钟前')
    expect(resolveText('en', relativeTime(Date.now() - 5 * 60_000))).toBe('5 min ago')
  })

  test('earlier today is the host clock reading, raw', () => {
    expect(relativeTime(new Date('2026-10-10T11:05:00').getTime())).toEqual(raw('11:05'))
  })

  test('an earlier day is a date key with its numbers', () => {
    const time = relativeTime(new Date('2026-10-09T14:05:00').getTime())
    expect(time).toEqual(msg('time.date', { month: 10, day: 9 }))
    expect(resolveText('zh', time)).toBe('10月9日')
    expect(resolveText('en', time)).toBe('10/9')
  })
})

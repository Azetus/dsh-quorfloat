// Ground truth for the panel's DOM: the real page skeleton, the real render
// module, and a recorded IPC stub — no hand-built fixtures where the page's
// own markup is the thing under test.
//
// This is the test the switch inversion asks for: it proves, against the real
// markup and the real binding, that aria-checked follows hideOnBlur and that a
// click asks the shell for the inverted value.

import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { beforeEach, describe, expect, test } from 'vitest'
import type { Snapshot } from '../src/lib/state'

// The real page skeleton, before render.ts's module top-level queries it.
// vitest's module URLs are http, so the file is read by path from the vite root.
const html = readFileSync(join(process.cwd(), 'index.html'), 'utf8')
const body = /<body>([\s\S]*)<\/body>/.exec(html)?.[1] ?? ''
document.body.innerHTML = body

// The shell's IPC, stubbed: every invoke is recorded.
const invocations: { cmd: string; args: Record<string, unknown> }[] = []
;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {
  invoke: (cmd: string, args: Record<string, unknown>) => {
    invocations.push({ cmd, args })
    return Promise.resolve({})
  },
}

const render = await import('../src/render')
render.bindStatic()

function snapshot(overrides: Partial<Snapshot> = {}): Snapshot {
  const base: Snapshot = {
    settings: {
      width: 640,
      maxHeight: 560,
      alwaysOnTop: true,
      reduceMotion: false,
      hideOnBlur: false,
      theme: 'dark',
    },
    hotkey: { requested: 'Alt+Space', held: 'Alt+Space', registered: true, reason: null },
    height: { target: 0, capped: false },
    window: { visible: true },
    session: {
      ready: true,
      following: null,
      pinned: null,
      chosen: null,
      createFailure: null,
      conversations: [],
      workspaces: [],
      workspacesAsked: false,
      options: null,
      stats: null,
      settingFailure: null,
    },
    transcript: {
      sessionId: null,
      cursor: 0,
      title: null,
      streaming: false,
      turnActive: false,
      entries: [],
      live: null,
    },
    interactions: [],
    handoff: null,
    delivery: {
      prompt: { state: 'idle', status: null },
      cancel: { state: 'idle', status: null },
    },
  }
  return { ...base, ...overrides }
}

beforeEach(() => {
  invocations.length = 0
})

describe('the keep-open switch', () => {
  test('hideOnBlur=false renders the switch on', () => {
    render.renderState(snapshot())
    expect(document.getElementById('q-keep-open')?.getAttribute('aria-checked')).toBe('true')
  })

  test('hideOnBlur=true renders the switch off', () => {
    render.renderState(snapshot({ settings: { ...snapshot().settings, hideOnBlur: true } }))
    expect(document.getElementById('q-keep-open')?.getAttribute('aria-checked')).toBe('false')
  })

  test('a click asks the shell for the value opposite to what is shown', () => {
    render.renderState(snapshot())
    expect(document.getElementById('q-keep-open')?.getAttribute('aria-checked')).toBe('true')
    invocations.length = 0
    document.getElementById('q-keep-open')?.click()
    expect(invocations).toEqual([{ cmd: 'set_preferences', args: { keepOpen: false } }])
  })
})

describe('the window drag region', () => {
  test('the top bar is a deep drag region', () => {
    expect(document.querySelector('.q-top')?.getAttribute('data-tauri-drag-region')).toBe('deep')
  })
})

describe('reduceMotion', () => {
  test('the host preference toggles the motion class on the root', () => {
    render.renderState(snapshot())
    expect(document.documentElement.classList.contains('q-reduce-motion')).toBe(false)

    render.renderState(snapshot({ settings: { ...snapshot().settings, reduceMotion: true } }))
    expect(document.documentElement.classList.contains('q-reduce-motion')).toBe(true)

    render.renderState(snapshot())
    expect(document.documentElement.classList.contains('q-reduce-motion')).toBe(false)
  })
})

describe('the footer key chips', () => {
  test('the design markup: drawn enter arrow, shift spelling, and esc as text', () => {
    render.renderState(snapshot())
    const status = document.getElementById('q-status')!
    const chips = status.querySelectorAll('kbd')
    expect(chips).toHaveLength(3)
    // The enter chip is the drawn ↵ icon (the bundled font lacks the glyph).
    expect(chips[0]?.querySelector('svg')).not.toBeNull()
    // Shift+Enter: the ⇧ is text (the font has it), the enter is drawn.
    expect(chips[1]?.textContent?.trim().startsWith('⇧')).toBe(true)
    expect(chips[1]?.querySelector('svg')).not.toBeNull()
    // esc stays text, as the design writes it.
    expect(chips[2]?.textContent).toBe('esc')
    expect(chips[2]?.querySelector('svg')).toBeNull()
    expect(status.textContent).toContain('发送')
    expect(status.textContent).toContain('换行')
    expect(status.textContent).toContain('关闭')
  })
})

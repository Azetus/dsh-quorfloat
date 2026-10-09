// Ground truth for the panel's DOM: the real page skeleton, the real render
// module, and a recorded IPC stub — no hand-built fixtures where the page's
// own markup is the thing under test.
//
// This is the test the switch inversion asks for: it proves, against the real
// markup and the real binding, that aria-checked follows hideOnBlur and that a
// click asks the shell for the inverted value.

import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { beforeEach, describe, expect, test, vi } from 'vitest'
import { ICONS } from '../src/lib/icons'
import type { PermissionOption, Snapshot } from '../src/lib/state'

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
      pinnedWorkspace: null,
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

// Permission fixtures shared by the menu and the footer entry: values and names as
// `session/options` sends them. The host translates the built-in label
// (src/harness/adapter.ts) but the *value* is the stable key — the same keys Harness's
// own picker maps glyphs for
// (packages/client/ui-primitives/src/PermissionIcon.tsx).
const trio: PermissionOption[] = [
  { value: 'read-only', name: '仅可查看', description: null },
  { value: 'workspace-write', name: '工作区内修改', description: null },
  { value: 'danger-full-access', name: '完全权限', description: null },
]

/**
 * The inner markup of one embedded asset — what the rendered `<svg>` must carry.
 * Spelled out here instead of asking the module under test which asset it meant:
 * a fixture that shares the parser's assumption cannot catch the parser. The
 * asset goes through the same DOM parser as the real glyph so that serialisation
 * (`<path/>` vs `<path></path>`) cannot be mistaken for a wrong icon.
 * @param name - asset key from the embedded map.
 * @returns the asset's markup without its `<svg>` wrapper.
 */
function assetBody(name: string): string {
  const source = ICONS[name]
  if (source === undefined) throw new Error(`the icon map carries no ${name}`)
  const host = document.createElement('div')
  host.innerHTML = source.trim()
  return host.firstElementChild?.innerHTML ?? ''
}

/**
 * One snapshot whose `session/options` carries the given catalog and current value.
 * @param permissions - the catalog rows.
 * @param permission - the current value, or null for "the host has not said".
 * @returns the snapshot to render.
 */
function withPermissions(permissions: PermissionOption[], permission: string | null): Snapshot {
  const base = snapshot()
  return snapshot({
    session: { ...base.session, options: { models: [], current: null, permissions, permission } },
  })
}

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

describe('the workspace pin (direction A)', () => {
  const workspaces = [{ workspaceId: 'ws-1', title: '项目', path: '/work/project' }]
  const following = {
    sessionId: 's-1',
    label: '项目',
    generation: 1,
    events: 0,
    streams: 0,
    resyncs: 0,
    stale: 0,
  }
  const withWorkspaces = (following: Snapshot['session']['following']) =>
    snapshot({ session: { ...snapshot().session, workspaces, following } })

  test('the pin button exists only in new-conversation state', () => {
    render.renderState(withWorkspaces(following))
    render.toggleMenu(withWorkspaces(following), 'q-workspace')
    expect(document.querySelectorAll('#q-workspace-menu .q-pin')).toHaveLength(0)

    render.renderState(withWorkspaces(null))
    render.toggleMenu(withWorkspaces(null), 'q-workspace')
    expect(document.querySelectorAll('#q-workspace-menu .q-pin')).toHaveLength(1)
  })

  test('choosing a workspace while following leaves the conversation first', () => {
    const state = withWorkspaces(following)
    render.renderState(state)
    render.toggleMenu(state, 'q-workspace')
    invocations.length = 0
    document.querySelector<HTMLButtonElement>('#q-workspace-menu .q-option')?.click()
    expect(invocations.some(invocation => invocation.cmd === 'start_new')).toBe(true)
  })

  test('pinning a workspace asks the shell for pin_workspace', () => {
    const state = withWorkspaces(null)
    render.renderState(state)
    render.toggleMenu(state, 'q-workspace')
    invocations.length = 0
    document.querySelector<HTMLButtonElement>('#q-workspace-menu .q-pin')?.click()
    expect(
      invocations.some(
        invocation => invocation.cmd === 'pin_workspace' && invocation.args.workspaceId === 'ws-1',
      ),
    ).toBe(true)
  })
})

describe('the permission menu glyphs', () => {
  function openPermissionMenu(state: Snapshot): string[] {
    render.renderState(state)
    // The module remembers which popover is open; a previous test may have left
    // this one open, and toggling it again would close it and assert on nothing.
    render.closeMenu()
    render.toggleMenu(state, 'q-permission')
    // `:not(.q-check)` — the selected row carries the check mark as well, and it is
    // the same kind of direct child.
    return [...document.querySelectorAll('#q-permission-menu .q-option > svg:not(.q-check)')]
      .map(svg => svg.innerHTML)
  }

  test('the three built-in presets carry three different glyphs, chosen by value', () => {
    expect(openPermissionMenu(withPermissions(trio, 'read-only'))).toEqual([
      assetBody('eye'), assetBody('folder-check'), assetBody('shield'),
    ])
  })

  test('a preset outside the trio keeps the uniform shield, never another option glyph', () => {
    const extra: PermissionOption = { value: 'auto', name: '自动审查', description: null }
    expect(openPermissionMenu(withPermissions([...trio, extra], 'auto'))).toEqual([
      assetBody('eye'), assetBody('folder-check'), assetBody('shield'), assetBody('shield-check'),
    ])
  })

  test('the glyph follows the value, not the host label', () => {
    const renamed = trio.map(option => ({ ...option, name: `自定义 ${option.value}` }))
    expect(openPermissionMenu(withPermissions(renamed, 'read-only'))).toEqual([
      assetBody('eye'), assetBody('folder-check'), assetBody('shield'),
    ])
  })
})

describe('the footer permission entry', () => {
  /** The glyph on the entry's own left edge — not the chevron's, which is the
   *  next `<svg>` inside the same button. */
  function entryGlyph(): string {
    return document.querySelector('#q-permission-icon > svg')?.innerHTML ?? ''
  }

  test('the entry shows the icon of the permission currently in force', () => {
    for (const [value, asset] of [
      ['read-only', 'eye'],
      ['workspace-write', 'folder-check'],
      ['danger-full-access', 'shield'],
    ] as const) {
      render.renderState(withPermissions(trio, value))
      expect(entryGlyph()).toBe(assetBody(asset))
    }
  })

  test('the entry and the selected menu row carry the same glyph', () => {
    const state = withPermissions(trio, 'danger-full-access')
    render.renderState(state)
    render.closeMenu()
    render.toggleMenu(state, 'q-permission')
    const selected = document.querySelectorAll('#q-permission-menu .q-option[aria-pressed="true"] > svg:not(.q-check)')
    expect(selected).toHaveLength(1)
    expect(entryGlyph()).toBe(selected[0]?.innerHTML)
  })

  test('a permission the catalog cannot name keeps the uniform shield', () => {
    // The host reports a value it never listed (or none at all): the entry must fall
    // back to the uniform glyph rather than borrow a name it cannot check.
    render.renderState(withPermissions(trio, 'auto'))
    expect(entryGlyph()).toBe(assetBody('shield-check'))

    const unknown = snapshot()
    render.renderState(unknown)
    expect(entryGlyph()).toBe(assetBody('shield-check'))
  })
})

describe('the model and reasoning lists', () => {
  // A catalog with descriptions on both levels: the host sends them, and the design's
  // own model/effort sub-list drops them (`option(label, selected, onPick)`), so the
  // panel must not grow a second text line the design never had.
  const models = [{
    provider: 'deepseek',
    providerName: 'DeepSeek',
    id: 'v41-flash',
    name: 'DeepSeek-V41-Flash',
    description: '面向日常对话的快速模型',
    efforts: [
      { id: 'high', name: '高', description: '更慢，也更仔细' },
      { id: 'low', name: '低', description: null },
    ],
    defaultEffort: 'high',
  }]

  function configState(): Snapshot {
    const base = snapshot()
    return snapshot({
      session: {
        ...base.session,
        options: {
          models,
          current: { provider: 'deepseek', model: 'v41-flash', reasoningEffort: 'high' },
          permissions: [],
          permission: null,
        },
      },
    })
  }

  function openConfigMenu(): HTMLElement[] {
    const state = configState()
    render.renderState(state)
    render.closeMenu()
    render.toggleMenu(state, 'q-config')
    return [...document.querySelectorAll<HTMLElement>('#q-config-menu .q-option')]
  }

  test('every model and reasoning row is one line of text', () => {
    const rows = openConfigMenu()
    // One model plus its two efforts.
    expect(rows).toHaveLength(3)
    expect(rows.every(row => row.querySelector('small') === null)).toBe(true)
    expect(rows.map(row => row.querySelector('.q-option-main')?.textContent))
      .toEqual(['DeepSeek-V41-Flash', '高', '低'])
  })

  test('the row still marks the model and the effort in force', () => {
    const rows = openConfigMenu()
    expect(rows[0]?.getAttribute('aria-pressed')).toBe('true')
    expect(rows[1]?.getAttribute('aria-pressed')).toBe('true')
    expect(rows[2]?.getAttribute('aria-pressed')).toBe('false')
  })
})

describe('swapFor', () => {
  const painted = { page: 'main' as const, session: 'session-1' }

  test('the first paint is the panel appearing, not a swap', () => {
    expect(render.swapFor(null, painted)).toBeNull()
  })

  test('a repaint of the same content is not a swap', () => {
    // The snapshots arrive on every transcript event while a turn streams, so a fade
    // tied to "a render happened" would strobe.
    expect(render.swapFor(painted, { page: 'main', session: 'session-1' })).toBeNull()
  })

  test('a conversation change is a session swap, including back to new-conversation', () => {
    expect(render.swapFor(painted, { page: 'main', session: 'session-2' })).toBe('session')
    expect(render.swapFor(painted, { page: 'main', session: null })).toBe('session')
  })

  test('a page change outranks a conversation change', () => {
    // One fade can only say one thing; the settings page is the bigger move.
    expect(render.swapFor(painted, { page: 'settings', session: 'session-2' })).toBe('page')
  })
})

describe('the content swap on screen', () => {
  const SWAP = 'q-swap'
  const hasSwap = (id: string) => document.getElementById(id)?.classList.contains(SWAP) ?? false

  test('the first paint animates nothing', () => {
    render.renderState(snapshot())
    expect(document.querySelectorAll(`.${SWAP}`)).toHaveLength(0)
  })

  test('the arriving page carries the swap, and the one it replaced does not', () => {
    const state = snapshot()
    render.renderState(state)

    render.openSettings(state)
    expect(hasSwap('q-settings')).toBe(true)
    expect(hasSwap('q-main')).toBe(false)

    render.backFromSettings(state)
    expect(hasSwap('q-main')).toBe(true)
    expect(hasSwap('q-settings')).toBe(false)
  })

  test('a conversation switch animates the thread, not the page around it', () => {
    const base = snapshot()
    render.renderState(base)
    render.renderState(snapshot({ transcript: { ...base.transcript, sessionId: 'session-2' } }))
    expect(hasSwap('q-thread-scroll')).toBe(true)
    expect(hasSwap('q-main')).toBe(false)
    expect(hasSwap('q-settings')).toBe(false)
  })
})

describe('heightPlan', () => {
  test('no swap: report what was measured, pin nothing', () => {
    expect(render.heightPlan({ from: null, desired: 400, animate: true }))
      .toEqual({ pin: false, report: 400 })
  })

  test('a height that did not move is not an animation', () => {
    expect(render.heightPlan({ from: 400, desired: 400.4, animate: true }))
      .toEqual({ pin: false, report: 400.4 })
  })

  test('growing reports the new height at once, so the shell can make room first', () => {
    expect(render.heightPlan({ from: 200, desired: 400, animate: true }))
      .toEqual({ pin: true, report: 400 })
  })

  test('shrinking keeps the room until the panel has finished collapsing', () => {
    // Reporting the smaller height immediately would shrink the native window at once and
    // the window would cut the collapse off mid-flight: the panel would be trimmed, not
    // seen to close. This is the "grow reserves room, shrink releases at the end" half of
    // the contract in `quorfloat/src/app/height.rs`.
    expect(render.heightPlan({ from: 400, desired: 200, animate: true }))
      .toEqual({ pin: true, report: 400 })
  })

  test('reduceMotion jumps to the new height, and says so at once', () => {
    expect(render.heightPlan({ from: 400, desired: 200, animate: false }))
      .toEqual({ pin: false, report: 200 })
  })
})

describe('the panel height a swap animates', () => {
  const panel = () => document.getElementById('q-window') as HTMLElement

  // The markup starts the panel hidden (`.q-away`); the shell takes that class off when it
  // shows the window, and every case here is about a panel somebody can see.
  beforeEach(() => { panel().classList.remove('q-away') })

  /**
   * Settle a running height pin the way the browser does it — jsdom has no transitions,
   * so the event the panel listens for has to be delivered by hand.
   */
  function settle(): void {
    const event = new Event('transitionend') as Event & { propertyName: string }
    Object.defineProperty(event, 'propertyName', { value: 'height' })
    panel().dispatchEvent(event)
  }

  /**
   * Put the panel back on the conversation page, settled, so that the swap a case is
   * about is the next thing that happens.
   * @param state - the snapshot to render with.
   */
  function onTheConversation(state: Snapshot): void {
    render.renderState(state)
    // Whatever an earlier case left on screen may itself be a swap; settle it, then make
    // sure the page is the conversation so `openSettings` below is a page change.
    render.backFromSettings(state)
    settle()
  }

  test('a swap pins both ends, and the transition ending releases the pin', () => {
    // `height: auto` is not interpolable, so both ends have to be told. (jsdom has no
    // layout, so the numbers here are degenerate; `heightPlan` pins the policy.)
    const state = snapshot()
    onTheConversation(state)
    expect(panel().style.height).toBe('')

    render.openSettings(state)
    expect(panel().style.height).not.toBe('')

    settle()
    expect(panel().style.height).toBe('')
  })

  test('a transition whose end never arrives is released by its deadline', () => {
    // The failure this guards (2026-10-09): the panel was pinned, the window was hidden
    // 90ms later, and a webview that stops rendering frames never delivers
    // `transitionend` — the panel stayed at the old height and the shell was never told
    // the new one, so nothing could correct it.
    vi.useFakeTimers()
    try {
      const state = snapshot()
      onTheConversation(state)
      render.openSettings(state)
      expect(panel().style.height).not.toBe('')

      vi.advanceTimersByTime(2000)
      expect(panel().style.height).toBe('')
    } finally {
      vi.useRealTimers()
    }
  })

  test('a panel on its way out is not pinned at all', () => {
    const state = snapshot()
    onTheConversation(state)
    panel().classList.add('q-away')

    render.openSettings(state)
    expect(panel().style.height).toBe('')
  })

  test('reduceMotion leaves the panel content-sized', () => {
    const state = snapshot()
    state.settings.reduceMotion = true
    onTheConversation(state)
    render.openSettings(state)
    expect(panel().style.height).toBe('')
  })
})

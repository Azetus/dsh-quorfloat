// The settings row's language dropdown, verified through the DOM it draws.
//
// The panel has no component-test dependency and no JSX test files (`vite.config.ts`
// includes `tests/**/*.test.ts`), so this renders the real component with `react-dom/client`
// and drives it with React's own `act` — the same thing a testing library would do under
// the hood, with nothing new to install. `../src/api` is mocked so a pick records the
// command instead of reaching for Tauri.

import { act, createElement, useState, type ReactElement } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import { api } from '../src/api'
import { I18nProvider } from '../src/components/common/I18nProvider'
import { LanguagePicker } from '../src/components/settings/LanguagePicker'
import type { Language } from '../src/lib/i18n'

vi.mock('../src/api', () => ({ api: { setPreferences: vi.fn() } }))

/** The app's own wiring: the app owns which popover is open. */
function Harness({ language }: { readonly language: Language }): ReactElement {
  const [open, setOpen] = useState(false)
  return createElement(LanguagePicker, {
    language,
    open,
    onToggle: () => { setOpen(current => !current) },
    onClose: () => { setOpen(false) },
  })
}

let container: HTMLDivElement
let root: Root

beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
  container = document.createElement('div')
  document.body.appendChild(container)
  root = createRoot(container)
})

afterEach(() => {
  act(() => { root.unmount() })
  container.remove()
  vi.clearAllMocks()
})

/** Render the picker with `language` in force, framed in Chinese like the panel. */
function render(language: Language): void {
  act(() => {
    // `children` travels as a prop rather than a third argument: `I18nProviderProps`
    // declares it required, and React's variadic-children overload cannot see that.
    root.render(createElement(I18nProvider, {
      language: 'zh',
      children: createElement(Harness, { language }),
    }))
  })
}

function trigger(): HTMLButtonElement {
  const element = container.querySelector<HTMLButtonElement>('#q-language')
  if (element === null) throw new Error('#q-language is not rendered')
  return element
}

function popover(): HTMLDivElement {
  const element = container.querySelector<HTMLDivElement>('#q-language-menu')
  if (element === null) throw new Error('#q-language-menu is not rendered')
  return element
}

function options(): HTMLButtonElement[] {
  return [...container.querySelectorAll<HTMLButtonElement>('.q-option')]
}

/** Click the trigger, as a user would. */
function toggle(): void {
  act(() => { trigger().click() })
}

describe('language picker trigger', () => {
  test('shows the language in force with a caret, collapsed', () => {
    render('zh')
    expect(trigger().textContent).toBe('中文')
    expect(trigger().querySelector('.q-chevron')).not.toBeNull()
    expect(trigger().getAttribute('aria-expanded')).toBe('false')
    expect(trigger().getAttribute('aria-haspopup')).toBe('menu')
    expect(trigger().getAttribute('aria-controls')).toBe('q-language-menu')
  })

  test('shows English when English is in force', () => {
    render('en')
    expect(trigger().textContent).toBe('English')
  })
})

describe('language picker menu', () => {
  test('opening lists both endonyms and marks the current one', () => {
    render('zh')
    toggle()
    expect(trigger().getAttribute('aria-expanded')).toBe('true')
    expect(popover().hasAttribute('hidden')).toBe(false)
    expect(options().map(option => option.textContent)).toEqual(['中文', 'English'])
    expect(options()[0]!.getAttribute('aria-pressed')).toBe('true')
    expect(options()[1]!.getAttribute('aria-pressed')).toBe('false')
    // The design's visible "this is the current one" mark.
    expect(options()[0]!.querySelector('.q-check')).not.toBeNull()
    expect(options()[1]!.querySelector('.q-check')).toBeNull()
  })

  test('choosing the other language writes it through the shell and closes', () => {
    render('zh')
    toggle()
    act(() => { options()[1]!.click() })
    expect(api.setPreferences).toHaveBeenCalledTimes(1)
    expect(api.setPreferences).toHaveBeenCalledWith({ language: 'en' })
    expect(popover().hasAttribute('hidden')).toBe(true)
  })

  test('choosing Chinese from English writes zh', () => {
    render('en')
    toggle()
    act(() => { options()[0]!.click() })
    expect(api.setPreferences).toHaveBeenCalledWith({ language: 'zh' })
  })
})

describe('language picker dismissal', () => {
  test('Escape closes it and returns focus to the trigger', () => {
    render('zh')
    toggle()
    expect(popover().hasAttribute('hidden')).toBe(false)
    act(() => { document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })) })
    expect(popover().hasAttribute('hidden')).toBe(true)
    expect(document.activeElement).toBe(trigger())
  })

  test('a click outside closes it', () => {
    render('zh')
    toggle()
    expect(popover().hasAttribute('hidden')).toBe(false)
    act(() => { document.body.dispatchEvent(new MouseEvent('click', { bubbles: true })) })
    expect(popover().hasAttribute('hidden')).toBe(true)
  })

  test('a second click on the trigger closes it', () => {
    render('zh')
    toggle()
    expect(popover().hasAttribute('hidden')).toBe(false)
    toggle()
    expect(popover().hasAttribute('hidden')).toBe(true)
  })
})

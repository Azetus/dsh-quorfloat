// Category 1 (cross-process protocol): the interface between this webview and the Tauri
// shell — the command names and argument shapes `src/api.ts` sends (`quorfloat/src/main.rs`).

import { beforeEach, describe, expect, test, vi } from 'vitest'
import { api } from '../src/api'

// `invoke` is the whole wire. Mocking it records what would have crossed to Rust without
// dragging in Tauri, a webview, or a DOM: the only thing under test is the call itself.
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

import { invoke } from '@tauri-apps/api/core'

const wire = vi.mocked(invoke)

beforeEach(() => {
  wire.mockClear()
})

describe('commands that carry no arguments', () => {
  test.each([
    { name: 'cancel', send: () => api.cancel() },
    { name: 'start_new', send: () => api.startNew() },
    { name: 'request_workspaces', send: () => api.requestWorkspaces() },
    { name: 'dismiss_handoff', send: () => api.dismissHandoff() },
  ])('$name is invoked by name, with no payload', async ({ name, send }) => {
    await send()
    // `invoke(cmd)` — a Rust command whose only parameter is `tauri::State` takes no payload.
    expect(wire).toHaveBeenCalledWith(name)
  })
})

describe('commands whose payload is one field', () => {
  test('submit carries the text and the chosen workspace', async () => {
    await api.submit('hello', 'ws-1')
    expect(wire).toHaveBeenCalledWith('submit', { text: 'hello', workspaceId: 'ws-1' })
  })

  test('submit sends a null workspace for "no workspace chosen"', async () => {
    await api.submit('hello', null)
    expect(wire).toHaveBeenCalledWith('submit', { text: 'hello', workspaceId: null })
  })

  test('pin and pin_workspace send their target, null meaning "stop pinning"', async () => {
    await api.pin('session-1')
    expect(wire).toHaveBeenCalledWith('pin', { sessionId: 'session-1' })
    await api.pinWorkspace(null)
    expect(wire).toHaveBeenCalledWith('pin_workspace', { workspaceId: null })
  })

  test('select_session, dismiss_interaction and log carry one string', async () => {
    await api.selectSession('session-1')
    expect(wire).toHaveBeenCalledWith('select_session', { sessionId: 'session-1' })
    await api.dismissInteraction('interaction-1')
    expect(wire).toHaveBeenCalledWith('dismiss_interaction', { interactionId: 'interaction-1' })
    await api.log('a breadcrumb')
    expect(wire).toHaveBeenCalledWith('log', { line: 'a breadcrumb' })
  })

  test('set_permission and set_visible carry one scalar', async () => {
    await api.setPermission('workspace-write')
    expect(wire).toHaveBeenCalledWith('set_permission', { value: 'workspace-write' })
    await api.setVisible(false)
    expect(wire).toHaveBeenCalledWith('set_visible', { visible: false })
  })

  test('report_content_height carries a number', async () => {
    await api.reportContentHeight(420)
    expect(wire).toHaveBeenCalledWith('report_content_height', { height: 420 })
  })

  test('report_languages carries the tags verbatim, in order', async () => {
    await api.reportLanguages(['zh-Hans-CN', 'en-US'])
    expect(wire).toHaveBeenCalledWith('report_languages', { languages: ['zh-Hans-CN', 'en-US'] })
  })
})

describe('commands whose payload is two or more fields', () => {
  test('answer_approval carries the interaction and the decision', async () => {
    await api.answerApproval('interaction-1', true)
    expect(wire).toHaveBeenCalledWith('answer_approval', { interactionId: 'interaction-1', allow: true })
  })

  test('answer_question carries one answer per question, as { id, selected, custom? }', async () => {
    // The Rust side parses exactly this shape (`parse_answers`), and `custom` travels only
    // when the user typed something — so both forms are asserted.
    await api.answerQuestion('interaction-1', [
      { id: 'q1', selected: ['Yes'] },
      { id: 'q2', selected: ['Other'], custom: 'my own answer' },
    ])
    expect(wire).toHaveBeenCalledWith('answer_question', {
      interactionId: 'interaction-1',
      answers: [
        { id: 'q1', selected: ['Yes'] },
        { id: 'q2', selected: ['Other'], custom: 'my own answer' },
      ],
    })
  })

  test('select_model carries the provider, the model and the effort', async () => {
    await api.selectModel('anthropic', 'claude', 'high')
    expect(wire).toHaveBeenCalledWith('select_model', { provider: 'anthropic', model: 'claude', effort: 'high' })
    await api.selectModel('anthropic', 'claude', null)
    expect(wire).toHaveBeenCalledWith('select_model', { provider: 'anthropic', model: 'claude', effort: null })
  })

  test('set_preferences sends exactly the changed fields, camelCased', async () => {
    // Each of these is a separate settings action, and the Rust parameters are
    // `theme`/`keep_open`/`hotkey`/`language`: a field that is absent must stay absent.
    await api.setPreferences({ language: 'en' })
    expect(wire).toHaveBeenCalledWith('set_preferences', { language: 'en' })
    await api.setPreferences({ keepOpen: true })
    expect(wire).toHaveBeenCalledWith('set_preferences', { keepOpen: true })
    await api.setPreferences({ hotkey: 'Command+Shift+Q' })
    expect(wire).toHaveBeenCalledWith('set_preferences', { hotkey: 'Command+Shift+Q' })
    await api.setPreferences({ theme: 'dark' })
    expect(wire).toHaveBeenCalledWith('set_preferences', { theme: 'dark' })
  })
})

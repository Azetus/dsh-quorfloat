// Typed wrappers over the shell's Tauri commands (`quorfloat/src/main.rs`).
// The command arguments are the bridge's parameters, camelCased — Tauri converts
// them back to the Rust snake_case names.

import { invoke } from '@tauri-apps/api/core'
import type { QuestionAnswer } from './logic/questions'

export const api = {
  openExternalUrl: (url: string) => invoke<void>('open_external_url', { url }),
  submit: (text: string, workspaceId: string | null) => invoke('submit', { text, workspaceId }),
  cancel: () => invoke('cancel'),
  answerApproval: (interactionId: string, allow: boolean) => invoke('answer_approval', { interactionId, allow }),
  /** Answer a model question: one entry per question, `selected` holding option labels. */
  answerQuestion: (interactionId: string, answers: readonly QuestionAnswer[]) =>
    invoke('answer_question', { interactionId, answers }),
  selectSession: (sessionId: string) => invoke('select_session', { sessionId }),
  /**
   * Ask the shell for one older page of the conversation on screen.
   *
   * No arguments: the cursor is the shell's (it is the oldest line it holds, in the
   * Harness's `beforeSeq` vocabulary) and the page does not come back here — it arrives as
   * the next `quorfloat/state` snapshot, so the page keeps rendering what the host said.
   */
  loadEarlier: () => invoke('load_earlier'),
  startNew: () => invoke('start_new'),
  pin: (sessionId: string | null) => invoke('pin', { sessionId }),
  pinWorkspace: (workspaceId: string | null) => invoke('pin_workspace', { workspaceId }),
  requestWorkspaces: () => invoke('request_workspaces'),
  selectModel: (provider: string, model: string, effort: string | null) => invoke('select_model', { provider, model, effort }),
  setPermission: (value: string) => invoke('set_permission', { value }),
  dismissInteraction: (interactionId: string) => invoke('dismiss_interaction', { interactionId }),
  dismissHandoff: () => invoke('dismiss_handoff'),
  log: (line: string) => invoke('log', { line }),
  setPreferences: (preferences: {
    theme?: string
    keepOpen?: boolean
    hotkey?: string
    /** The panel's own language, the same two ids the snapshot carries. */
    language?: 'zh' | 'en'
  }) => invoke('set_preferences', preferences),
  reportContentHeight: (height: number) => invoke('report_content_height', { height }),
  setVisible: (visible: boolean) => invoke('set_visible', { visible }),
  /**
   * Report the language tags this webview can see, once, on load.
   *
   * Sent verbatim and in the browser's own order: the shell keeps the raw list as the
   * record of what this machine says, and resolves it only when nobody has chosen a
   * language (see `logic/languages.ts`).
   */
  reportLanguages: (languages: readonly string[]) => invoke('report_languages', { languages }),
}

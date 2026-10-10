// Typed wrappers over the shell's Tauri commands (`quorfloat/src/main.rs`).
// The command arguments are the bridge's parameters, camelCased — Tauri converts
// them back to the Rust snake_case names.

import { invoke } from '@tauri-apps/api/core'
import type { QuestionAnswer } from './lib/questions'

export const api = {
  submit: (text: string, workspaceId: string | null) => invoke('submit', { text, workspaceId }),
  cancel: () => invoke('cancel'),
  answerApproval: (interactionId: string, allow: boolean) => invoke('answer_approval', { interactionId, allow }),
  /** Answer a model question: one entry per question, `selected` holding option labels. */
  answerQuestion: (interactionId: string, answers: readonly QuestionAnswer[]) =>
    invoke('answer_question', { interactionId, answers }),
  selectSession: (sessionId: string) => invoke('select_session', { sessionId }),
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
}

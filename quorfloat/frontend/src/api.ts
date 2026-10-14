// Typed wrappers over the shell's Tauri commands (`quorfloat/src/main.rs`).
// The command arguments are the bridge's parameters, camelCased — Tauri converts
// them back to the Rust snake_case names.

import { invoke } from '@tauri-apps/api/core'
import type { QuestionAnswer } from './logic/questions'

interface ImageChunk {
  token: string; offset: number; nextOffset: number; totalBytes: number
  mimeType: string; data: string; done: boolean
}

/** Assemble a bounded transfer independently of conversation snapshots. */
async function readLocalImage(sessionId: string, path: string, signal: AbortSignal): Promise<string> {
  let token: string | undefined
  let offset = 0
  let total = 0
  let mimeType = ''
  let completed = false
  const parts: Uint8Array<ArrayBuffer>[] = []
  try {
    while (true) {
      signal.throwIfAborted()
      const part = await invoke<ImageChunk>('read_local_image', { sessionId, path, token, offset })
      const previousToken = token
      token = part.token
      signal.throwIfAborted()
      if (typeof token !== 'string' || !token || (previousToken !== undefined && token !== previousToken)
        || part.offset !== offset || !Number.isSafeInteger(part.totalBytes) || part.totalBytes <= 0 || part.totalBytes > 8 * 1024 * 1024
        || !Number.isSafeInteger(part.nextOffset) || part.nextOffset <= offset || part.nextOffset > part.totalBytes
        || typeof part.data !== 'string' || part.data.length > 350_000
        || !['image/png', 'image/jpeg', 'image/webp', 'image/gif'].includes(part.mimeType)
        || typeof part.done !== 'boolean' || part.done !== (part.nextOffset === part.totalBytes)
        || (offset > 0 && (total !== part.totalBytes || mimeType !== part.mimeType))) {
        throw { reason: 'readFailed' }
      }
      const bytes = Uint8Array.from(atob(part.data), char => char.charCodeAt(0))
      if (bytes.length !== part.nextOffset - offset || bytes.length > 256 * 1024) throw { reason: 'readFailed' }
      parts.push(bytes)
      offset = part.nextOffset
      total = part.totalBytes
      mimeType = part.mimeType
      if (part.done) {
        completed = true
        return URL.createObjectURL(new Blob(parts, { type: mimeType }))
      }
    }
  } finally {
    if (!completed && typeof token === 'string') {
      // The host also expires abandoned snapshots. Release explicitly on errors
      // or cancellation instead of waiting for that fallback.
      await invoke('read_local_image', { sessionId, path, token, release: true }).catch(() => {})
    }
  }
}

type ImageJob = { start: () => void; signal: AbortSignal; abort: () => void }
const imageQueue: ImageJob[] = []
let activeImages = 0

/** Match the host's two-transfer budget when many local images mount together. */
function drainImages(): void {
  while (activeImages < 2 && imageQueue.length > 0) {
    const job = imageQueue.shift()!
    job.signal.removeEventListener('abort', job.abort)
    job.start()
  }
}

function loadLocalImage(sessionId: string, path: string, signal: AbortSignal): Promise<string> {
  return new Promise((resolve, reject) => {
    if (signal.aborted) { reject(signal.reason); return }
    const job: ImageJob = {
      signal,
      abort: () => {
        const index = imageQueue.indexOf(job)
        if (index >= 0) imageQueue.splice(index, 1)
        reject(signal.reason)
      },
      start: () => {
        activeImages += 1
        void readLocalImage(sessionId, path, signal).then(resolve, reject).finally(() => {
          activeImages -= 1
          drainImages()
        })
      },
    }
    signal.addEventListener('abort', job.abort, { once: true })
    imageQueue.push(job)
    drainImages()
  })
}

export const api = {
  loadLocalImage,
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

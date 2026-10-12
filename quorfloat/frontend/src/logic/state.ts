// The shell's `quorfloat/state` snapshot, as TypeScript types.
//
// The shape is projected by `quorfloat/src/app/bridge.rs` — this file is its
// mirror, and the two must stay in step. Every field here exists because the
// bridge writes it; nothing here is guessed.

export interface Snapshot {
  settings: {
    width: number
    maxHeight: number
    alwaysOnTop: boolean
    reduceMotion: boolean
    hideOnBlur: boolean
    theme: 'system' | 'light' | 'dark'
    /**
     * The language the panel draws in. Optional on purpose: the field is newer than some
     * shells, and `logic/i18n`'s `languageOf` reads an absent value as `'zh'` — the panel's
     * default — rather than crashing on it.
     */
    language?: 'zh' | 'en'
  }
  hotkey: {
    requested: string | null
    held: string | null
    registered: boolean
    reason: string | null
  }
  height: {
    target: number
    capped: boolean
  }
  window: {
    visible: boolean
    /**
     * The summon counter, and the process lifetime it belongs to.
     *
     * A count alone cannot tell a summon from a reload: a webview reload or a sidecar restart
     * starts the count over, and reading that reset as a summon would throw away the workspace
     * the user chose for this visit. Only "same token, higher count" is a summon — see
     * `logic/summon.ts`, which owns that rule.
     */
    summon: {
      token: string
      count: number
    }
  }
  session: {
    ready: boolean
    following: {
      sessionId: string
      label: string | null
      generation: number | null
      events: number
      streams: number
      resyncs: number
      stale: number
    } | null
    pinned: string | null
    /** The pinned **workspace**: where a new conversation starts. Independent of `pinned`,
     *  which decides which conversation opens — neither is derived from the other. */
    pinnedWorkspace: string | null
    /**
     * Why a half of the pin was forgotten, while the explanation is still current.
     *
     * A reason, never a sentence: the panel's copy lives in the dictionaries, and this is what
     * `view-text.ts` turns into the status line. Valid for one visit — a summon clears it.
     */
    pinNotice: { kind: 'sessionGone' | 'workspaceGone' } | null
    chosen: string | null
    /** Where a conversation submitted now would be created (the ladder's answer, or null
     *  when only the user can decide). */
    createWorkspace: string | null
    createFailure: string | null
    conversations: Conversation[]
    workspaces: Workspace[]
    workspacesAsked: boolean
    /**
     * Whether the workspace list has been *answered*.
     *
     * "Not arrived yet" and "there are none" are different facts, and only the second one is a
     * reason to ask the host for a default workspace or to tell the user to make one.
     */
    workspacesLoaded: boolean
    /** What came of asking the host for its first-use default workspace. */
    defaultWorkspace: { state: 'idle' | 'done' | 'failed' }
    options: Options | null
    stats: Stats | null
    settingFailure: string | null
  }
  transcript: {
    sessionId: string | null
    cursor: number
    title: string | null
    streaming: boolean
    turnActive: boolean
    /**
     * Whether the host says history precedes the oldest line on screen.
     *
     * The thread offers "load earlier" only at the top and when history exists behind it.
     * Absent on a shell older than this build, which reads as `false` — no control, rather
     * than a request nobody can serve.
     */
    hasMore?: boolean
    /** Whether an older page is in flight, which is the control's busy state. */
    loadingEarlier?: boolean
    entries: Entry[]
    live: Entry | null
  }
  interactions: Interaction[]
  handoff: {
    kind: 'approval' | 'question'
    reason: string
    surfaces: string[]
  } | null
  delivery: {
    prompt: Delivery
    cancel: Delivery
  }
}

export interface Conversation {
  sessionId: string
  updatedAt: number
  title: string | null
  label: string | null
  cwd: string | null
  running: boolean
  blank: boolean
}

export interface Workspace {
  workspaceId: string
  title: string
  path: string
  /**
   * The conversations the registry accounts to this workspace, in its own order.
   *
   * **Membership, not the directory.** It can name a conversation the session list no longer
   * shows, so a consumer intersects it with the conversations it can see — see `logic/scope.ts`,
   * which is the one place that derivation lives.
   */
  sessionIds: string[]
  /** Registration time in epoch milliseconds: one unit end to end, and never negative. */
  createdAt: number
}

export interface Options {
  models: ModelOption[]
  current: { provider: string; model: string; reasoningEffort: string | null } | null
  permissions: PermissionOption[]
  permission: string | null
}

export interface ModelOption {
  provider: string
  providerName: string
  id: string
  name: string
  description: string | null
  efforts: EffortOption[]
  defaultEffort: string | null
}

export interface EffortOption {
  id: string
  name: string
  description: string | null
}

export interface PermissionOption {
  value: string
  name: string
  description: string | null
}

export interface Stats {
  turns: number
  steps: number
  tokensPerSecond: number | null
  totalTokens: number | null
  cacheHitPercent: number | null
  contextTokens: number | null
  contextLimit: number | null
}

export type Entry =
  | { kind: 'user'; text: string }
  | { kind: 'assistant'; streaming: boolean; blocks: Block[] }
  | { kind: 'tool'; name: string | null; text: string; isError: boolean }
  | { kind: 'system'; text: string }

export type Block =
  | { kind: 'text'; text: string }
  | { kind: 'reasoning'; text: string }
  | { kind: 'call'; name: string; arguments: string }

export type InteractionState = 'pending' | 'submitting' | 'applied' | 'refused'

/** One selectable answer, as the asker offered it. */
export interface QuestionOption {
  /** The user-facing label, and what an answer sends back. */
  label: string
  /** Optional extra context. */
  description: string | null
}

/** One question of a card. */
export interface InteractionQuestion {
  /** The asker's stable id, echoed in the answer. */
  id: string
  /** Optional short heading. */
  header: string | null
  /** The question itself. */
  question: string
  /** Optional supporting detail. */
  detail: string | null
  /** The choices offered, possibly none (a free-text question). */
  options: QuestionOption[]
  /** Whether more than one option may be picked. */
  multiSelect: boolean
}

/** One answer this panel sent, as the shell echoes it back. */
export interface InteractionAnswer {
  /** The question answered. */
  id: string
  /** Option labels that were picked. */
  selected: string[]
  /** Free text, when some was typed. */
  custom: string | null
}

export interface Interaction {
  id: string
  sessionId: string
  kind: 'approval' | 'question'
  state: InteractionState
  verdict: 'allowed-once' | 'rejected' | null
  refusalReason: string | null
  actionable: boolean
  toolName: string | null
  detail: string | null
  questions: InteractionQuestion[]
  answers: InteractionAnswer[]
  questionCount: number
}

export interface Delivery {
  state: 'idle' | 'sending' | 'accepted' | 'failed'
  status: string | null
}

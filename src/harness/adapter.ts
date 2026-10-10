/**
 * The single boundary between this plugin and DeepSeek Harness internals.
 *
 * Everything this project knows about `sessionController`, its request types,
 * its outcomes, and its error vocabulary is declared here. Two consequences
 * follow, and both are deliberate:
 *
 * - **Upstream drift is local.** A nightly Harness build that renames a field
 *   breaks this file and nothing else; `src/harness/session-layer.ts` keeps
 *   working against the project's own view model.
 * - **The plugin stays testable.** `QuorfloatHarness` is a plain interface, so
 *   tests drive the whole host plugin with a fake Harness instead of a running
 *   desktop app.
 *
 * Only the subset this plugin actually calls is declared: create, list,
 * page, prompt, cancel, follow, plus the workspace registry lookup and the
 * version probes. Adding a method here is a deliberate act, not an accident of
 * what happens to be reachable on `ctx`.
 */

import type { ChannelErrorCode } from '../bridge/errors.js'

/**
 * Failure raised by a Harness-facing operation.
 *
 * The `code` set is this project's own, so callers never branch on upstream
 * error strings; {@link QuorfloatHarness} implementations translate.
 */
export class HarnessError extends Error {
  readonly code: ChannelErrorCode
  readonly detail: Record<string, unknown>

  /**
   * @param code - project-level failure kind.
   * @param message - human-readable explanation.
   * @param detail - structured context for logs and diagnostics.
   */
  constructor(code: ChannelErrorCode, message: string, detail: Record<string, unknown> = {}) {
    super(message)
    this.name = 'HarnessError'
    this.code = code
    this.detail = detail
  }
}

/** One selectable Harness workspace. */
export interface WorkspaceView {
  readonly workspaceId: string
  readonly path: string
  readonly title: string
}

/** One row of the Harness session list. */
/**
 * How many conversations one `sessions/list` call may read a title for.
 *
 * The panel lists every few seconds; a title read is an `inspect`, and unbounded work per poll is
 * how a list of twenty conversations becomes twenty reads every three seconds. Whatever is left
 * over keeps its id for one more poll rather than forever.
 */
const MAX_TITLE_READS = 2

/**
 * Fold the latest title out of a conversation's events.
 *
 * The same fold the Harness does — `foldSessionTitle`,
 * `packages/session/session-title/src/index.ts:282` — copied rather than guessed: the last
 * `session/title` event wins, and the title is its `data.title`.
 *
 * Only the shape the Harness source documents is encoded; anything else yields no title — a list
 * of ids, which is where the panel already was — rather than a name invented out of the wrong
 * field.
 *
 * @param events - a conversation's events, in order.
 * @returns the title, or `undefined` when the conversation has never been named.
 */
export function foldSessionTitle(events: readonly unknown[]): string | undefined {
  const event = events.findLast(item => (item as { type?: unknown } | undefined)?.type === 'session/title') as
    | { data?: { title?: unknown } }
    | undefined
  if (event === undefined) return undefined
  const title = event.data?.title
  return typeof title === 'string' && title !== '' ? title : undefined
}

export interface SessionSummaryView {
  readonly sessionId: string
  /**
   * What the conversation is called, when the Harness has a name for it.
   *
   * The panel draws the conversation list from this record, and a list of session ids is a list
   * nobody can read: the title is the only thing that tells two conversations apart.
   */
  readonly title?: string
  readonly cwd?: string
  readonly updatedAt: number
  readonly running: boolean
  readonly blank: boolean
}

/** One durable history page: raw wire records, in the shape the protocol carries. */
export interface HistoryPageView {
  readonly records: readonly unknown[]
  readonly hasMore: boolean
}

/** One frame from the follow subscription, already reduced to a wire-safe shape. */
export type FollowFrame =
  | { readonly type: 'snapshot'; readonly cursor: number; readonly hasMore: boolean; readonly records: readonly unknown[]; readonly projections: unknown; readonly assistantStream: unknown }
  | { readonly type: 'event'; readonly seq: number; readonly eventType: string; readonly time: number; readonly data: unknown }
  | { readonly type: 'assistant-stream'; readonly frame: unknown }

/** Result of starting a follow subscription. */
export interface FollowHandle {
  /** Stop following. Safe to call more than once. */
  close(): void
  /** Resolves when the subscription ends for any reason. */
  readonly done: Promise<void>
}

/**
 * The Harness capabilities this plugin requires.
 *
 * Implemented once for the real runtime (`createHarnessFromContext`) and once
 * per test. Methods take project-level inputs and return project-level values.
 */
export interface QuorfloatHarness {
  /** Human-readable description of what was found, for diagnostics. */
  describe(): Record<string, unknown>
  /** List workspaces the user may choose from, in Harness display order. */
  listWorkspaces(): readonly WorkspaceView[]
  /** Look up one workspace; `undefined` when it no longer exists. */
  workspace(workspaceId: string): WorkspaceView | undefined
  /**
   * Create (or idempotently adopt) one session inside a workspace.
   *
   * @param workspaceId - the workspace the user selected; never omitted.
   */
  createSession(workspaceId: string): Promise<{ sessionId: string }>
  /** List sessions visible in the Harness session list. */
  listSessions(): Promise<readonly SessionSummaryView[]>
  /**
   * Read the session's current durable cursor, without activating its Agent.
   *
   * History paging is *cursor-relative*: the upstream call requires an inclusive
   * upper bound that is a real sequence and never past the session's cursor. A
   * client therefore cannot ask for "the latest events" by inventing a large
   * number — it has to know the cursor. That knowledge has to come from
   * somewhere, and this is the cold-safe source for it.
   *
   * @param sessionId - durable session identity.
   * @returns the highest committed sequence, or `undefined` when unknown.
   */
  cursor(sessionId: string): Promise<number | undefined>
  /**
   * Read one message-aligned history page.
   *
   * @param sessionId - durable session identity.
   * @param throughSeq - inclusive upper bound; must be a real sequence at or
   *   below the session cursor (pass {@link QuorfloatHarness.cursor}'s value).
   * @param beforeSeq - exclusive backwards cursor within that bound.
   */
  pageHistory(sessionId: string, throughSeq: number, beforeSeq?: number): Promise<HistoryPageView>
  /**
   * Admit one prompt.
   *
   * @param sessionId - target session.
   * @param requestId - caller-owned idempotency key; a repeat of the same key
   *   must return the original acceptance instead of enqueueing twice.
   * @param text - prompt text.
   */
  prompt(sessionId: string, requestId: string, text: string): Promise<{ accepted: true }>
  /** Request cancellation of the session's active turn. */
  cancel(sessionId: string): Promise<{ accepted: true }>
  /**
   * Follow one session from its opening snapshot.
   *
   * @param sessionId - durable session identity.
   * @param onFrame - called for every frame; must not throw.
   * @returns a handle that stops the subscription.
   */
  follow(sessionId: string, onFrame: (frame: FollowFrame) => void): FollowHandle
  /**
   * Read what may be chosen for one session, and what is chosen now.
   *
   * Two upstream sources behind one call, because a picker needs both halves at the
   * same moment: the model catalog (`sessionController.modelCatalog`, process-wide) and the
   * permission presets (`permissionPresets.catalog` / `current`, per session). A picker
   * that fetched them separately could render a list beside a stale value.
   *
   * @param sessionId - durable session identity, or `undefined` before one is attached —
   *   the catalogs are still readable then, which is what lets the picker be ready.
   * @returns the selectable options and the current values.
   */
  options(sessionId?: string): Promise<SessionOptionsView>
  /**
   * Switch one session's model and reasoning effort.
   *
   * **Both in one call**, because upstream holds them in one `ModelSelection`: choosing a
   * model resets the effort to that model's default, so sending them separately would
   * briefly register an effort the new model may not have.
   *
   * @param sessionId - target session.
   * @param selection - provider, model, and the effort to pair with them.
   */
  selectModel(sessionId: string, selection: ModelChoiceView): Promise<void>
  /**
   * Apply one permission preset to a session.
   *
   * A preset is *not* a sandbox mode: it decides the sandbox mode **and** the approval
   * policy together, and it is recorded as its own durable event. Setting a bare mode
   * would leave the two halves disagreeing.
   *
   * @param sessionId - target session.
   * @param value - the preset's stable value, as the catalog spelled it.
   */
  setPermission(sessionId: string, value: string): Promise<void>
  /**
   * Read a session's statistics without activating its Agent.
   *
   * Cold-safe on the same terms as {@link QuorfloatHarness.cursor}: it reads registered
   * projections out of the session log rather than waking anything.
   *
   * @param sessionId - durable session identity.
   * @returns the statistics, or `undefined` when the session is gone.
   */
  stats(sessionId: string): Promise<SessionStatsView | undefined>
}

/** One reasoning effort a model offers. */
export interface EffortView {
  /** Stable value sent back on selection. */
  readonly id: string
  /** The label a person reads. */
  readonly name: string
  /** One sentence on what it does, when the provider supplies one. */
  readonly description?: string
}

/** One selectable model. */
export interface ModelView {
  /** Provider-owned model id. */
  readonly id: string
  /** The label a person reads. */
  readonly name: string
  /** One sentence about the model, when the provider supplies one. */
  readonly description?: string
  /** The efforts this model accepts, empty when it has no reasoning control. */
  readonly efforts: readonly EffortView[]
  /** The effort the provider defaults to, when it names one. */
  readonly defaultEffort?: string
}

/** One provider's models. */
export interface ModelGroupView {
  /** Provider route, sent back with the chosen model. */
  readonly provider: string
  /** The label a person reads. */
  readonly name: string
  /** The models under this provider, in the catalog's order. */
  readonly models: readonly ModelView[]
}

/** One selectable permission preset. */
export interface PermissionView {
  /** Stable value sent back when chosen. */
  readonly value: string
  /** The label a person reads. */
  readonly name: string
  /** One sentence on what the preset allows, when it has one. */
  readonly description?: string
}

/** The model half of a selection: what is chosen now, and what may be. */
export interface ModelChoiceView {
  /** Provider route of the current model. */
  readonly provider: string
  /** The current model's id. */
  readonly model: string
  /** The current reasoning effort, when one is set. */
  readonly reasoningEffort?: string
}

/** Everything a session's pickers need, in one answer. */
export interface SessionOptionsView {
  /** The models available, grouped by provider. */
  readonly groups: readonly ModelGroupView[]
  /** What is selected now, or `undefined` when nothing is attached yet. */
  readonly current?: ModelChoiceView
  /** The permission presets available. */
  readonly permissions: readonly PermissionView[]
  /** The permission preset in effect, or `undefined` when unknown. */
  readonly permission?: string
}

/** A session's statistics, reduced to what the panel shows. */
export interface SessionStatsView {
  /** Distinct turns with at least one closed step. */
  readonly turns: number
  /** Closed steps. */
  readonly steps: number
  /** Output tokens per second over the steps that reported usage, or `undefined`. */
  readonly tokensPerSecond?: number
  /**
   * Every token the session has been billed for, or `undefined` when none was reported.
   *
   * The four usage buckets are disjoint, so the total is their sum: the three prompt-side
   * buckets plus output. Reasoning tokens are already inside `outputTokens` and are *not*
   * added again — counting them twice would inflate the one figure a person uses to judge cost.
   */
  readonly totalTokens?: number
  /** Cache-read share of the prompt, as a percentage, or `undefined`. */
  readonly cacheHitPercent?: number
  /** Prompt size of the most recent request, when the provider reported one. */
  readonly contextTokens?: number
  /** The model's context capacity, when known. */
  readonly contextLimit?: number
}

/** Structural view of the one upstream service this plugin depends on. */
interface SessionControllerLike {
  inspect(sessionId: string, signal: AbortSignal): Promise<{ events?: readonly Record<string, unknown>[] }>
  /** The process-wide model catalog; `undefined` on a build without one. */
  modelCatalog?(): Promise<unknown>
  /** Switch one session's model and reasoning effort together. */
  selectModel?(request: unknown): Promise<unknown>
  /** Read every registered projection for one session, cold. */
  projections?(request: unknown, signal: AbortSignal): Promise<unknown>
  list(request: unknown, signal: AbortSignal): Promise<{ items?: readonly Record<string, unknown>[] }>
  create(request: unknown): Promise<{ sessionId?: unknown }>
  prompt(request: unknown, signal: AbortSignal): Promise<unknown>
  cancel(request: unknown): unknown
  page(request: unknown, signal: AbortSignal): Promise<{ records?: readonly unknown[]; hasMore?: unknown }>
  follow(request: unknown, signal: AbortSignal): AsyncIterable<Record<string, unknown>>
}

/** Structural view of the live-session store: `get` by id, no Agent activation. */
interface SessionsLike {
  /** The live Session for an id, or `undefined` when it is not loaded. */
  get(id: string): unknown
}

/** Structural view of the permission-preset service. */
interface PermissionPresetsLike {
  /** Every currently selectable preset, process-wide. */
  catalog(): { options?: readonly Record<string, unknown>[] }
  /** The preset value in effect for one session. */
  current(session: unknown): string
  /** Switch one session's preset; the only write path. */
  set(session: unknown, name: string): void
}

/** Structural view of the workspace registry. */
interface WorkspaceRegistryLike {
  list(): readonly WorkspaceEntityLike[]
  get(id: string): WorkspaceEntityLike | undefined
}

interface WorkspaceEntityLike {
  readonly id: string
  readonly path: string
  readonly title: string
}

/** Minimal logger shape the adapter needs. */
export interface AdapterLogger {
  warn(message: string, detail?: unknown): void
  debug(message: string, detail?: unknown): void
}

/** Services the adapter was asked for, and whether each was present. */
export interface ServicePresence {
  readonly sessionController: boolean
  readonly workspaceRegistry: boolean
  readonly approval: boolean
  readonly userQuestions: boolean
}

/** Outcome of probing a context for the services this plugin needs. */
export interface ProbeResult {
  readonly presence: ServicePresence
  /** Names of required services that are missing; empty means usable. */
  readonly missing: readonly string[]
}

/**
 * Probe a Cordis context for the services the plugin needs.
 *
 * @param get - the context's optional-service reader (`ctx.get`).
 * @returns which services are present and which required ones are missing.
 */
export function probeServices(get: (name: string) => unknown): ProbeResult {
  const presence: ServicePresence = {
    sessionController: get('sessionController') !== undefined,
    workspaceRegistry: get('workspaceRegistry') !== undefined,
    approval: get('approval') !== undefined,
    userQuestions: get('userQuestions') !== undefined,
  }
  const missing: string[] = []
  if (!presence.sessionController) missing.push('sessionController')
  // The registry is read through the controller when it is reachable, so it is
  // required only as a fallback; report it without failing the probe.
  return { presence, missing }
}

/**
 * Build the real adapter over a Cordis context.
 *
 * @param get - the context's optional-service reader.
 * @param log - logger for translation warnings.
 * @returns the adapter, or `undefined` when a required service is missing.
 */
export function createHarnessFromContext(
  get: (name: string) => unknown,
  log: AdapterLogger,
): { harness?: QuorfloatHarness; probe: ProbeResult } {
  const probe = probeServices(get)
  if (probe.missing.length > 0) return { probe }
  const controller = get('sessionController') as SessionControllerLike
  const registry = get('workspaceRegistry') as WorkspaceRegistryLike | undefined
  // Permission presets are their own service, and **optional**: a build without the
  // interaction package has no presets to offer, and the panel then hides the picker
  // rather than showing one with nothing in it.
  const presets = get('permissionPresets') as PermissionPresetsLike | undefined
  // The session store is how a `Session` object is obtained from an id. It is only
  // consulted for the two writes that need one (a permission switch, a model switch);
  // the reads that answer pickers never need it.
  const sessions = get('sessions') as SessionsLike | undefined
  return { harness: new CordisHarness(controller, registry, presets, sessions, log), probe }
}

/**
 * Adapter over the live `sessionController` (and, when reachable, the workspace
 * registry it is built on).
 */
class CordisHarness implements QuorfloatHarness {
  readonly #controller: SessionControllerLike
  readonly #registry: WorkspaceRegistryLike | undefined
  readonly #presets: PermissionPresetsLike | undefined
  readonly #sessions: SessionsLike | undefined
  readonly #log: AdapterLogger
  /** Counters that make upstream translation problems visible in diagnostics. */
  readonly #counters = {
    listCalls: 0,
    createCalls: 0,
    promptCalls: 0,
    cancelled: 0,
    followOpened: 0,
    followClosed: 0,
    cursorCalls: 0,
    selectModelCalls: 0,
    permissionCalls: 0,
    statsCalls: 0,
  }
  /** Titles by session, with the `updatedAt` they were read at. */
  readonly #titles = new Map<string, { updatedAt: number; title: string | undefined }>()

  /**
   * @param controller - the live session controller service.
   * @param registry - the workspace registry, when composed.
   * @param presets - the permission-preset service, when composed.
   * @param sessions - the live-session store, when composed.
   * @param log - logger for translation warnings.
   */
  constructor(
    controller: SessionControllerLike,
    registry: WorkspaceRegistryLike | undefined,
    presets: PermissionPresetsLike | undefined,
    sessions: SessionsLike | undefined,
    log: AdapterLogger,
  ) {
    this.#controller = controller
    this.#registry = registry
    this.#presets = presets
    this.#sessions = sessions
    this.#log = log
  }

  /** {@inheritDoc QuorfloatHarness.describe} */
  describe(): Record<string, unknown> {
    return { adapter: 'cordis', workspaceSource: this.#registry === undefined ? 'controller' : 'registry', counters: { ...this.#counters } }
  }

  /** {@inheritDoc QuorfloatHarness.listWorkspaces} */
  listWorkspaces(): readonly WorkspaceView[] {
    if (this.#registry === undefined) return []
    try {
      return this.#registry.list().map(entity => ({
        workspaceId: entity.id,
        path: entity.path,
        title: entity.title,
      }))
    } catch (error) {
      this.#log.warn('workspace listing failed', error)
      return []
    }
  }

  /** {@inheritDoc QuorfloatHarness.workspace} */
  workspace(workspaceId: string): WorkspaceView | undefined {
    if (this.#registry === undefined) return undefined
    const entity = this.#registry.get(workspaceId)
    return entity === undefined ? undefined : { workspaceId: entity.id, path: entity.path, title: entity.title }
  }

  /** {@inheritDoc QuorfloatHarness.createSession} */
  async createSession(workspaceId: string): Promise<{ sessionId: string }> {
    // `workspaceId` and `cwd` are mutually exclusive upstream; passing only the
    // id is what keeps the child's own directory from ever becoming a workspace.
    this.#counters.createCalls += 1
    const result = await this.#controller.create({ workspaceId })
    const sessionId = result?.sessionId
    if (typeof sessionId !== 'string' || sessionId === '') {
      throw new HarnessError('protocol-violation', 'session creation returned no identity', { workspaceId, result })
    }
    return { sessionId }
  }

  /** {@inheritDoc QuorfloatHarness.listSessions} */
  async listSessions(): Promise<readonly SessionSummaryView[]> {
    this.#counters.listCalls += 1
    const signal = new AbortController().signal
    const value = await this.#controller.list({}, signal)
    const items = Array.isArray(value?.items) ? value.items : []
    const summaries = items.map(item => {
      const sessionId = typeof item['sessionId'] === 'string' ? item['sessionId'] : ''
      const cwd = typeof item['cwd'] === 'string' ? item['cwd'] : undefined
      const title = typeof item['title'] === 'string' && item['title'] !== '' ? item['title'] : undefined
      return {
        sessionId,
        ...(title === undefined ? {} : { title }),
        ...(cwd === undefined ? {} : { cwd }),
        updatedAt: typeof item['updatedAt'] === 'number' ? item['updatedAt'] : 0,
        running: item['running'] === true,
        blank: item['blank'] === true,
      }
    }).filter(item => item.sessionId !== '')
    return await this.#withTitles(summaries, signal)
  }

  /**
   * Name the conversations, reading the ones whose name is not known yet.
   *
   * The Harness's own list carries no titles. The names exist in each conversation's history as a
   * `session/title` event, and `inspect` is documented as cold-safe — the persisted header and
   * prefix, without resuming an agent — so reading them wakes nothing.
   *
   * Cached by session and invalidated by `updatedAt`: a conversation that has moved on may have
   * been renamed, and one that has not is asked about once.
   *
   * @param summaries - what the list gave, without names.
   * @param signal - abort signal, shared with the list call.
   * @returns the same conversations, as many named as the budget allowed.
   */
  async #withTitles(
    summaries: readonly SessionSummaryView[],
    signal: AbortSignal,
  ): Promise<readonly SessionSummaryView[]> {
    let budget = MAX_TITLE_READS
    const named: SessionSummaryView[] = []
    for (const summary of summaries) {
      if (summary.title !== undefined) {
        named.push(summary)
        continue
      }
      const cached = this.#titles.get(summary.sessionId)
      if (cached !== undefined && cached.updatedAt === summary.updatedAt) {
        named.push(cached.title === undefined ? summary : { ...summary, title: cached.title })
        continue
      }
      if (budget <= 0) {
        named.push(summary)
        continue
      }
      budget -= 1
      try {
        const title = await this.#readTitle(summary.sessionId, signal)
        this.#titles.set(summary.sessionId, { updatedAt: summary.updatedAt, title })
        named.push(title === undefined ? summary : { ...summary, title })
      } catch (error) {
        // A conversation that cannot be inspected keeps its id in the list rather than vanishing
        // from it: the list is about what exists, not about what can be named.
        this.#log.debug('could not read a conversation title', {
          sessionId: summary.sessionId,
          error: String(error),
        })
        named.push(summary)
      }
    }
    return named
  }

  /**
   * Read one conversation's title from its history.
   *
   * The last `session/title` event wins: a conversation can be renamed as it develops.
   *
   * @param sessionId - the conversation to read.
   * @param signal - abort signal.
   * @returns the title, or `undefined` when it has never been named.
   */
  async #readTitle(sessionId: string, signal: AbortSignal): Promise<string | undefined> {
    const inspection = await this.#controller.inspect(sessionId, signal)
    const events = Array.isArray(inspection?.events) ? inspection.events : []
    return foldSessionTitle(events)
  }

  /** {@inheritDoc QuorfloatHarness.cursor} */
  async cursor(sessionId: string): Promise<number | undefined> {
    this.#counters.cursorCalls += 1
    const signal = new AbortController().signal
    try {
      // `inspect` is documented as cold-safe: it returns the attached state or the
      // persisted header and event prefix without resuming an Agent.
      const inspection = await this.#controller.inspect(sessionId, signal)
      const events = Array.isArray(inspection?.events) ? inspection.events : []
      const last = events.at(-1)
      const seq = last !== undefined && typeof (last as { seq?: unknown }).seq === 'number'
        ? (last as { seq: number }).seq
        : undefined
      return seq
    } catch (error) {
      // History is optional; the caller can still follow. Report and continue.
      this.#log.warn('could not read the session cursor', { sessionId, error: String(error) })
      return undefined
    }
  }

  /** {@inheritDoc QuorfloatHarness.pageHistory} */
  async pageHistory(sessionId: string, throughSeq: number, beforeSeq?: number): Promise<HistoryPageView> {
    const signal = new AbortController().signal
    const request: Record<string, unknown> = {
      address: { kind: 'session', sessionId },
      throughSeq,
      maxMessages: 50,
    }
    if (beforeSeq !== undefined) request['beforeSeq'] = beforeSeq
    const page = await this.#controller.page(request, signal)
    return {
      records: Array.isArray(page?.records) ? page.records : [],
      hasMore: page?.hasMore === true,
    }
  }

  /** {@inheritDoc QuorfloatHarness.prompt} */
  async prompt(sessionId: string, requestId: string, text: string): Promise<{ accepted: true }> {
    this.#counters.promptCalls += 1
    const signal = new AbortController().signal
    await this.#controller.prompt(
      {
        requestId,
        sessionId,
        mode: 'queue',
        content: [{ type: 'text', text }],
      },
      signal,
    )
    // Acceptance only means the Host admitted the prompt. Generation state comes
    // from follow frames; nothing here may be reported as "the answer started".
    return { accepted: true }
  }

  /** {@inheritDoc QuorfloatHarness.cancel} */
  async cancel(sessionId: string): Promise<{ accepted: true }> {
    this.#counters.cancelled += 1
    this.#controller.cancel({ sessionId })
    return { accepted: true }
  }

  /** {@inheritDoc QuorfloatHarness.options} */
  async options(sessionId?: string): Promise<SessionOptionsView> {
    const groups = await this.#modelGroups()
    // The permission half is absent on a build without the interaction package, and the
    // picker is then hidden rather than shown with nothing in it — an empty menu is worse
    // than no menu, because it looks like a failure.
    let permissions: PermissionView[] = []
    const values = sessionId === undefined ? undefined : await this.#selectionProjections(sessionId)
    const permission = readCurrentPermission(values)
    if (this.#presets !== undefined) {
      const catalog = this.#presets.catalog()
      permissions = (catalog.options ?? []).flatMap(option => {
        const value = typeof option['value'] === 'string' ? option['value'] : undefined
        const name = typeof option['name'] === 'string' ? option['name'] : undefined
        if (value === undefined || name === undefined) return []
        const description = typeof option['description'] === 'string' ? option['description'] : undefined
        return [{ value, name: permissionLabel(value, name), ...(description === undefined ? {} : { description }) }]
      })
    }
    const current = readCurrentSelection(values)
    return {
      groups,
      ...(current === undefined ? {} : { current }),
      permissions,
      ...(permission === undefined ? {} : { permission }),
    }
  }

  /** {@inheritDoc QuorfloatHarness.selectModel} */
  async selectModel(sessionId: string, selection: ModelChoiceView): Promise<void> {
    if (this.#controller.selectModel === undefined) {
      throw new HarnessError('unavailable', 'this Harness build cannot select a model', { sessionId })
    }
    this.#counters.selectModelCalls += 1
    await this.#controller.selectModel({
      sessionId,
      provider: selection.provider,
      model: selection.model,
      ...(selection.reasoningEffort === undefined ? {} : { reasoningEffort: selection.reasoningEffort }),
    })
  }

  /** {@inheritDoc QuorfloatHarness.setPermission} */
  async setPermission(sessionId: string, value: string): Promise<void> {
    const presets = this.#presets
    const session = this.#sessions?.get(sessionId)
    if (presets === undefined) {
      throw new HarnessError('unavailable', 'this Harness build has no permission presets', { sessionId })
    }
    if (session === undefined) {
      // The Session store has not loaded this session. The writes are `Session`-shaped
      // upstream (a preset appends an event to the log), so there is nothing to write to
      // rather than something to work around.
      throw new HarnessError('unavailable', 'this session is not loaded, so its permissions cannot change', { sessionId })
    }
    this.#counters.permissionCalls += 1
    // `set` rather than a sandbox-mode write: a preset decides the sandbox mode *and* the
    // approval policy, records its own durable event, and is the upstream's only switch.
    presets.set(session, value)
  }

  /** {@inheritDoc QuorfloatHarness.stats} */
  async stats(sessionId: string): Promise<SessionStatsView | undefined> {
    if (this.#controller.projections === undefined) return undefined
    this.#counters.statsCalls += 1
    const abort = new AbortController()
    let value: unknown
    try {
      value = await this.#controller.projections({ sessionId }, abort.signal)
    } catch (error) {
      // Statistics are an adornment: a session that cannot report them still works, and a
      // panel that failed to draw its input because a counter was unavailable would be a
      // worse panel. The failure is logged rather than raised.
      this.#log.debug('session statistics are unavailable', { sessionId, error: String(error) })
      return undefined
    }
    return readStats(value)
  }

  /**
   * The model catalog as picker groups.
   *
   * @returns the groups, or an empty list when this build has no catalog.
   */
  async #modelGroups(): Promise<ModelGroupView[]> {
    if (this.#controller.modelCatalog === undefined) {
      // Said out loud: "the model menu is empty" has two very different causes — a build without the
      // method, and a build whose providers all failed — and they are indistinguishable from the
      // panel alone.
      this.#log.warn('this Harness build has no model catalog method')
      return []
    }
    try {
      const catalog = await this.#controller.modelCatalog()
      const groups = readModelGroups(catalog)
      if (groups.length === 0) {
        // The upstream answer also carries *why* each provider could not be listed (a missing key, an
        // unreachable route). Dropping that is how "the model menu is empty" becomes unanswerable, so
        // it goes to the log where the reason is readable.
        this.#log.warn('the model catalog listed no models', { failures: (catalog as Record<string, unknown>)['failures'] ?? [] })
      }
      return groups
    } catch (error) {
      this.#log.warn('the model catalog could not be read', error)
      return []
    }
  }

  /**
   * What is selected for one session now.
   *
   * Read from the `modelSelection` projection rather than from the agent: the projection is
   * what the *next* step will use, which is the value a picker must show — reading the
   * last-used model from the log would show what the previous step ran with.
   *
   * @param sessionId - the session to read.
   * @returns the selection, or `undefined` when the projection is not available.
   */
  async #selectionProjections(sessionId: string): Promise<unknown> {
    if (this.#controller.projections === undefined) return undefined
    try {
      const value = await this.#controller.projections({ sessionId }, new AbortController().signal)
      return value
    } catch (error) {
      this.#log.debug('session selection projections are unavailable', { sessionId, error: String(error) })
      return undefined
    }
  }

  /** {@inheritDoc QuorfloatHarness.follow} */
  follow(sessionId: string, onFrame: (frame: FollowFrame) => void): FollowHandle {
    this.#counters.followOpened += 1
    const abort = new AbortController()
    const request = {
      address: { kind: 'session', sessionId },
      assistantStream: true,
      maxMessages: 50,
    }
    const iterator = this.#controller.follow(request, abort.signal)
    let closed = false
    // The iteration is driven here rather than handed to the caller; its only
    // failure mode is reported through the log, never as an unhandled rejection.
    const done = (async () => {
      try {
        for await (const frame of iterator) {
          if (closed) break
          const projected = projectFrame(frame)
          if (projected === undefined) continue
          try {
            onFrame(projected)
          } catch (error) {
            // A display-side failure must not tear down the durable subscription.
            this.#log.warn('follow frame consumer threw', error)
          }
        }
      } catch (error) {
        if (!closed) this.#log.warn('follow subscription ended with an error', error)
      } finally {
        this.#counters.followClosed += 1
      }
    })()
    return {
      done,
      close: () => {
        if (closed) return
        closed = true
        // Aborting resolves the iteration; the rejection cannot escape as an
        // unhandled rejection from a fire-and-forget consumer.
        abort.abort(new Error('dsh-quorfloat: follow closed by host'))
      },
    }
  }
}

/**
 * Read the projection baseline's `values` map.
 *
 * Upstream answers `{asOfSeq, values}`; anything else is a shape this build does not
 * recognise, and the rule for those is to report nothing rather than to guess — a wrong
 * token count is worse than no token count.
 *
 * @param value - a projections response.
 * @returns the values map, or `undefined`.
 */
function projectionValues(value: unknown): Record<string, unknown> | undefined {
  if (typeof value !== 'object' || value === null) return undefined
  const values = (value as Record<string, unknown>)['values']
  if (typeof values !== 'object' || values === null) return undefined
  return values as Record<string, unknown>
}

/** A finite, non-negative number, or `undefined`. */
function count(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) && value >= 0 ? value : undefined
}

/**
 * Reduce the statistics projections to what the panel shows.
 *
 * Two projections, because they answer different questions: `sessionStats` counts turns,
 * steps and time; `tokenUsage` counts tokens, and the cache-hit share and the output speed
 * are ratios of those. Either can be absent — a session with no completed step has no
 * speed — so every field is optional and the panel decides what it can draw.
 *
 * @param value - a projections response.
 * @returns the statistics, or `undefined` when this build has none for the session.
 */
export function readStats(value: unknown): SessionStatsView | undefined {
  const values = projectionValues(value)
  if (values === undefined) return undefined
  const stats = values['sessionStats']
  const usage = values['tokenUsage']
  const pressure = values['contextPressure']
  const statsRecord = typeof stats === 'object' && stats !== null ? stats as Record<string, unknown> : {}
  const usageRecord = typeof usage === 'object' && usage !== null ? usage as Record<string, unknown> : {}
  const pressureRecord = typeof pressure === 'object' && pressure !== null ? pressure as Record<string, unknown> : {}

  const turns = count(statsRecord['turns'])
  const steps = count(statsRecord['steps'])
  // The numerator prefers the projection's own view of the *next* request, which is what the meter
  // is for; the raw sample is the fallback, as upstream has it.
  const contextTokens = count(pressureRecord['projectedTokens']) ?? count(pressureRecord['pressureTokens'])
  const contextLimit = count(pressureRecord['contextWindow'])
  // Nothing at all to report: no counts, no usage, no occupancy. A session with only *some* of the
  // three is a real state (a step can settle without billing, and a meter can report capacity alone),
  // so the guard asks whether there is anything rather than whether all of it is there.
  if (turns === undefined && steps === undefined
    && Object.keys(usageRecord).length === 0 && contextTokens === undefined) {
    return undefined
  }

  // Output speed: the reported output tokens over the reported decode time. Both are
  // needed, and a zero decode time is *not* an infinite speed.
  const decodeTokens = count(statsRecord['decodeTokens'])
  const decodeMs = count(statsRecord['decodeMs'])
  const tokensPerSecond = decodeTokens !== undefined && decodeMs !== undefined && decodeMs > 0
    ? decodeTokens / (decodeMs / 1000)
    : undefined

  // Cache-hit share: cache reads over the whole *billed prompt*, which is what a person means
  // by "how much was cached" — reads alone would report 100% for a tiny prompt.
  const cacheRead = count(usageRecord['cacheReadTokens'])
  const uncached = count(usageRecord['uncachedInputTokens'])
  const cacheWrite = count(usageRecord['cacheWriteTokens']) ?? 0
  const output = count(usageRecord['outputTokens'])
  const prompt = (cacheRead ?? 0) + (uncached ?? 0) + cacheWrite
  const cacheHitPercent = readCacheHitPercent(cacheRead, prompt)

  // Cumulative: the three prompt-side buckets plus output. The upstream UI shows this same
  // sum, and it is the only figure that answers "what has this conversation cost me".
  const anyUsage = cacheRead !== undefined || uncached !== undefined
    || usageRecord['cacheWriteTokens'] !== undefined || output !== undefined
  const totalTokens = anyUsage ? prompt + (output ?? 0) : undefined

  return {
    turns: turns ?? 0,
    steps: steps ?? 0,
    ...(tokensPerSecond === undefined || tokensPerSecond <= 0 ? {} : { tokensPerSecond }),
    ...(totalTokens === undefined ? {} : { totalTokens }),
    ...(cacheHitPercent === undefined ? {} : { cacheHitPercent }),
    // The occupancy: the *projected* figure when the meter has one, otherwise the sampled one, over
    // the window the model reported. Both field names are the upstream's
    // (`context-occupancy.ts`: `projectedTokens ?? pressureTokens`, `contextWindow`) — the sampled
    // number alone ignores the surface the next request will add, and reading a field the projection
    // does not have (`capacityTokens`) yields no statistic at all.
    ...(contextTokens === undefined ? {} : { contextTokens }),
    ...(contextLimit === undefined ? {} : { contextLimit }),
  }
}

/**
 * The cache-hit share, without ever rounding a partial hit up to a full one.
 *
 * The upstream client's rule (`formatCacheHitPercent`, token-format.ts) exists for a reason
 * worth keeping: a session at 99.6% would display "100%" under plain rounding, telling a person
 * their prompt was entirely cached when it was not — and cache misses are exactly the expensive
 * thing this figure is read to find. So a value that would round to 100 reports 99.9 instead.
 *
 * @param cacheRead - cache-read tokens, or `undefined` when unreported.
 * @param prompt - the billed prompt total (the three prompt-side buckets).
 * @returns whole-number percent, or `undefined` when there is no prompt to take a share of.
 */
export function readCacheHitPercent(cacheRead: number | undefined, prompt: number): number | undefined {
  if (cacheRead === undefined || prompt <= 0) return undefined
  const missed = prompt - cacheRead
  if (missed === 0) return 100
  const percent = (cacheRead / prompt) * 100
  const rounded = Math.round(percent)
  // One decimal is enough to tell "all but a sliver" from "all": the panel shows one decimal
  // rather than the upstream's escalating 99.99…, because its footer has room for a word and
  // not for an ever-longer number.
  return rounded < 100 ? rounded : Math.round(percent * 10) / 10
}

/**
 * Reduce the model catalog to picker groups.
 *
 * Every level is checked rather than trusted: a provider group with no usable models is
 * dropped instead of rendering an empty header, and a model with no id cannot be selected
 * so it is not offered.
 *
 * @param value - a `modelCatalog()` response.
 * @returns the groups, possibly empty.
 */
export function readModelGroups(value: unknown): ModelGroupView[] {
  if (typeof value !== 'object' || value === null) return []
  const groups = (value as Record<string, unknown>)['groups']
  if (!Array.isArray(groups)) return []
  const read = (item: unknown): ModelView | undefined => {
    if (typeof item !== 'object' || item === null) return undefined
    const record = item as Record<string, unknown>
    const id = typeof record['id'] === 'string' ? record['id'] : undefined
    if (id === undefined || id === '') return undefined
    const reasoning = typeof record['reasoning'] === 'object' && record['reasoning'] !== null
      ? record['reasoning'] as Record<string, unknown>
      : undefined
    const efforts: EffortView[] = Array.isArray(reasoning?.['efforts'])
      ? (reasoning?.['efforts'] as unknown[]).flatMap(entry => {
        if (typeof entry !== 'object' || entry === null) return []
        const effort = entry as Record<string, unknown>
        const effortId = typeof effort['id'] === 'string' ? effort['id'] : undefined
        if (effortId === undefined || effortId === '') return []
        const name = typeof effort['name'] === 'string' ? effort['name'] : effortId
        const description = typeof effort['description'] === 'string' ? effort['description'] : undefined
        return [{ id: effortId, name, ...(description === undefined ? {} : { description }) }]
      })
      : []
    const defaultEffort = typeof reasoning?.['defaultEffort'] === 'string' ? reasoning['defaultEffort'] : undefined
    const description = typeof record['description'] === 'string' ? record['description'] : undefined
    return {
      id,
      name: typeof record['name'] === 'string' && record['name'] !== '' ? record['name'] : id,
      ...(description === undefined ? {} : { description }),
      efforts,
      ...(defaultEffort === undefined ? {} : { defaultEffort }),
    }
  }
  return groups.flatMap(group => {
    if (typeof group !== 'object' || group === null) return []
    const record = group as Record<string, unknown>
    const provider = typeof record['id'] === 'string' ? record['id'] : undefined
    if (provider === undefined || provider === '') return []
    const models: ModelView[] = Array.isArray(record['models'])
      ? (record['models'] as unknown[]).flatMap(item => read(item) ?? [])
      : []
    return models.length === 0
      ? []
      : [{ provider, name: typeof record['name'] === 'string' && record['name'] !== '' ? record['name'] : provider, models }]
  })
}

/**
 * Read what one session has selected now.
 *
 * The public wire projection exposes `next`, already resolved by Harness from its internal
 * `pending ?? lastUsed` state (session-controller/src/model-selection-projection.ts).
 * Internal state is not a second supported wire format.
 *
 * @param value - a projections response.
 * @returns the selection, or `undefined` when this build does not report one.
 */
export function readCurrentSelection(value: unknown): ModelChoiceView | undefined {
  const values = projectionValues(value)
  if (values === undefined) return undefined
  const projection = values['modelSelection']
  if (typeof projection !== 'object' || projection === null) return undefined
  const record = projection as Record<string, unknown>
  const candidate = record['next']
  if (typeof candidate !== 'object' || candidate === null) return undefined
  const selection = candidate as Record<string, unknown>
  const provider = typeof selection['provider'] === 'string' ? selection['provider'] : undefined
  const model = typeof selection['model'] === 'string' ? selection['model'] : undefined
  if (!provider || !model) return undefined
  const effort = typeof selection['reasoningEffort'] === 'string' ? selection['reasoningEffort'] : undefined
  return { provider, model, ...(effort === undefined ? {} : { reasoningEffort: effort }) }
}

/**
 * Match Harness's Chinese UI labels without defining its selectable options.
 * Source: ui-permission-presets/src/client/{presentation,locales}.ts.
 * Only the conventional built-in label is translated; configured labels pass through.
 * @param value - stable key from the catalog.
 * @param name - host-supplied display label.
 * @returns localized built-in label or the original custom name.
 */
function permissionLabel(value: string, name: string): string {
  const labels: Record<string, readonly [string, string]> = {
    'read-only': ['Read Only', '仅可查看'],
    'workspace-write': ['Workspace Write', '工作区内修改'],
    'danger-full-access': ['Full access', '完全权限'],
  }
  const label = Object.hasOwn(labels, value) ? labels[value] : undefined
  return label !== undefined && (name === value || name === label[0]) ? label[1] : name
}

/**
 * Read the cold-safe public permission selection, without requiring a live Agent/Session.
 * @param value - a sessionController.projections response.
 * @returns the host's current value, or nothing for an unfamiliar projection.
 */
export function readCurrentPermission(value: unknown): string | undefined {
  const permissions = projectionValues(value)?.['permissions']
  if (typeof permissions !== 'object' || permissions === null) return undefined
  const current = (permissions as Record<string, unknown>)['currentValue']
  return typeof current === 'string' && current.length > 0 ? current : undefined
}

/**
 * Reduce one upstream follow frame to the wire-safe project shape.
 *
 * Unknown frame types return `undefined` rather than being forwarded: sending
 * an unrecognised frame shape to the child would invite it to invent a
 * rendering rule for data this version does not understand.
 *
 * @param frame - raw upstream frame.
 * @returns the project frame, or `undefined` when it is not one we project.
 */
export function projectFrame(frame: Record<string, unknown>): FollowFrame | undefined {
  const type = frame['type']
  if (type === 'snapshot') {
    return {
      type: 'snapshot',
      cursor: typeof frame['cursor'] === 'number' ? frame['cursor'] : -1,
      hasMore: frame['hasMore'] === true,
      records: Array.isArray(frame['records']) ? frame['records'] : [],
      projections: frame['projections'] ?? {},
      assistantStream: frame['assistantStream'] ?? null,
    }
  }
  if (type === 'event') {
    const event = frame['event']
    if (typeof event !== 'object' || event === null) return undefined
    const record = event as Record<string, unknown>
    return {
      type: 'event',
      seq: typeof record['seq'] === 'number' ? record['seq'] : -1,
      eventType: typeof record['type'] === 'string' ? record['type'] : 'unknown',
      time: typeof record['time'] === 'number' ? record['time'] : 0,
      data: record['data'] ?? null,
    }
  }
  if (type === 'assistant-stream') {
    return { type: 'assistant-stream', frame: frame['frame'] ?? null }
  }
  return undefined
}

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
 * Only the subset the first version actually calls is declared: create, list,
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
export interface SessionSummaryView {
  readonly sessionId: string
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
}

/** Structural view of the one upstream service this plugin depends on. */
interface SessionControllerLike {
  inspect(sessionId: string, signal: AbortSignal): Promise<{ events?: readonly Record<string, unknown>[] }>
  list(request: unknown, signal: AbortSignal): Promise<{ items?: readonly Record<string, unknown>[] }>
  create(request: unknown): Promise<{ sessionId?: unknown }>
  prompt(request: unknown, signal: AbortSignal): Promise<unknown>
  cancel(request: unknown): unknown
  page(request: unknown, signal: AbortSignal): Promise<{ records?: readonly unknown[]; hasMore?: unknown }>
  follow(request: unknown, signal: AbortSignal): AsyncIterable<Record<string, unknown>>
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
  return { harness: new CordisHarness(controller, registry, log), probe }
}

/**
 * Adapter over the live `sessionController` (and, when reachable, the workspace
 * registry it is built on).
 */
class CordisHarness implements QuorfloatHarness {
  readonly #controller: SessionControllerLike
  readonly #registry: WorkspaceRegistryLike | undefined
  readonly #log: AdapterLogger
  /** Counters that make upstream translation problems visible in diagnostics. */
  readonly #counters = { listCalls: 0, createCalls: 0, promptCalls: 0, cancelled: 0, followOpened: 0, followClosed: 0, cursorCalls: 0 }

  /**
   * @param controller - the live session controller service.
   * @param registry - the workspace registry, when composed.
   * @param log - logger for translation warnings.
   */
  constructor(controller: SessionControllerLike, registry: WorkspaceRegistryLike | undefined, log: AdapterLogger) {
    this.#controller = controller
    this.#registry = registry
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
    return items.map(item => {
      const sessionId = typeof item['sessionId'] === 'string' ? item['sessionId'] : ''
      const cwd = typeof item['cwd'] === 'string' ? item['cwd'] : undefined
      return {
        sessionId,
        ...(cwd === undefined ? {} : { cwd }),
        updatedAt: typeof item['updatedAt'] === 'number' ? item['updatedAt'] : 0,
        running: item['running'] === true,
        blank: item['blank'] === true,
      }
    }).filter(item => item.sessionId !== '')
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

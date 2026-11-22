/**
 * Session subscriptions and wire projection.
 *
 * This layer answers the questions the adapter deliberately does not:
 * - which sessions does this host currently follow, and with what generation;
 * - how do upstream snapshot/event/stream frames become protocol notifications;
 * - why may a prompt never be sent twice, even after a reconnect;
 * - which frames belong to a subscription that was already replaced.
 *
 * It never talks to Harness directly: it consumes {@link QuorfloatHarness} and
 * emits notifications through an injected sender, so its rules (generation
 * guards, dedup keys, sequence checks) are unit-testable without a runtime.
 */

import type {
  FollowFrame,
  FollowHandle,
  HistoryPageView,
  ModelChoiceView,
  QuorfloatHarness,
  SessionOptionsView,
  SessionStatsView,
  SessionSummaryView,
  WorkspaceView,
} from './adapter.js'
import { HarnessError, oldestSeq } from './adapter.js'

/** Minimal logger shape used here. */
export interface SessionLayerLogger {
  warn(message: string, detail?: unknown): void
  debug(message: string, detail?: unknown): void
}

/** One `session/snapshot` frame, as the adapter projects it. */
type SnapshotFrame = Extract<FollowFrame, { type: 'snapshot' }>

/**
 * Byte budget for one `session/snapshot/part` payload.
 *
 * A transport part size well below the 4 MiB frame ceiling. A snapshot used to travel as
 * one frame, and a conversation whose records passed the ceiling was dropped by the peer's
 * reader before any of it could be folded — leaving the panel blank with no reason on the
 * record. The budget leaves room for the frame envelope and the peer's write buffer.
 */
export const SNAPSHOT_PART_BUDGET_BYTES = 256 * 1024

/** Wire size of a history page's records, for deciding whether transport framing is needed. */
function historyPageBytes(records: readonly unknown[]): number {
  return Buffer.byteLength(JSON.stringify(records))
}

/** One bounded slice of a snapshot's records. */
export interface SnapshotPart {
  /** Position in the sequence, zero-based. */
  readonly index: number
  /** The records this part carries, in order. */
  readonly records: readonly unknown[]
  /** The part's wire size in bytes, as {@link planSnapshotParts} measured it. */
  readonly bytes: number
}

/**
 * Split a snapshot's records into parts that fit comfortably inside one frame.
 *
 * Sizes are measured in **bytes**, not characters: records are mostly text, and
 * `String.length` counts a CJK character as one where `Buffer.byteLength` counts three,
 * so a budget enforced on `length` would emit a frame three times the ceiling for exactly
 * the conversations this exists to deliver.
 *
 * A single record larger than the budget travels as its own part rather than being cut
 * in half — a record is the host's own unit, and splitting one would corrupt it. The
 * caller is told the measured size so that one case is findable rather than silent.
 *
 * @param records - the snapshot's records, in order.
 * @param context - the identity every part carries (sessionId, generation).
 * @param budget - per-part ceiling in bytes.
 * @returns the parts, in order, covering every record exactly once.
 */
export function planSnapshotParts(
  records: readonly unknown[],
  context: Record<string, unknown>,
  budget: number = SNAPSHOT_PART_BUDGET_BYTES,
): readonly SnapshotPart[] {
  const parts: SnapshotPart[] = []
  let current: unknown[] = []
  for (const record of records) {
    const grown = [...current, record]
    const size = snapshotPartBytes(context, parts.length, grown)
    // Flush only what is already there: a single record over the budget still needs a
    // part of its own, because there is no smaller unit to send.
    if (current.length > 0 && size > budget) {
      parts.push({ index: parts.length, records: current, bytes: snapshotPartBytes(context, parts.length, current) })
      current = [record]
      continue
    }
    current = grown
  }
  if (current.length > 0) {
    parts.push({ index: parts.length, records: current, bytes: snapshotPartBytes(context, parts.length, current) })
  }
  return parts
}

/**
 * The wire size of one candidate part.
 *
 * Measured through `JSON.stringify`, which is what the frame writer encodes: a
 * character count would miss the escaping and the multi-byte text that make the real
 * frame bigger than the string suggests.
 *
 * @param context - the identity fields the part carries.
 * @param index - the part's position, part of the payload.
 * @param records - the records the part would carry.
 * @returns the part's UTF-8 byte length.
 */
function snapshotPartBytes(
  context: Record<string, unknown>,
  index: number,
  records: readonly unknown[],
): number {
  return Buffer.byteLength(JSON.stringify({ ...context, index, records }))
}

/** Inputs the session layer needs from the plugin around it. */
export interface SessionLayerDeps {
  /** The Harness adapter. */
  harness: QuorfloatHarness
  /** Send one notification to the peer; failures are logged and swallowed. */
  notify(method: string, params: unknown): Promise<void>
  /** Workspace used when the caller asks for a session without naming one. */
  defaultWorkspaceId(): string
  /** Observation hook for status projection and tests. */
  onStateChange?(sessionId: string, reason: string, detail?: unknown): void
  log: SessionLayerLogger
}

/** A subscription epoch; incremented on every attach so stale frames are droppable. */
export interface FollowState {
  readonly sessionId: string
  readonly generation: number
  readonly handle: FollowHandle
  /** Highest durable sequence forwarded for this subscription; `-1` before the snapshot. */
  cursor: number
  /** The durable snapshot for this generation has been delivered. */
  snapshotDelivered: boolean
  closed: boolean
}

/** Result of an attach request. */
export interface AttachResult {
  readonly sessionId: string
  readonly generation: number
  /** True when an earlier subscription for the same session was replaced. */
  readonly replaced: boolean
}

/**
 * One session's prompt bookkeeping, kept so a reconnect cannot resend input.
 *
 * `requestId` is the caller's idempotency key and is generated here, once per
 * user submission. `admitted` records that the Host acknowledged it; an
 * `admitted` entry is never re-sent, and an entry that is neither admitted nor
 * failed is reported as "awaiting confirmation" instead of being retried.
 */
interface PromptRecord {
  readonly requestId: string
  readonly sessionId: string
  readonly text: string
  state: 'pending' | 'admitted' | 'failed'
  detail?: unknown
}

/**
 * The prompt key a user message was created from, when it has one.
 *
 * The shape is the Harness's own: a `user/message` carries `source: {kind: 'user', rpcId}`, and `rpcId`
 * is the `requestId` the prompt was sent with (`packages/api/session-controller/src/commands.ts`).
 *
 * @param data - one event's `data`.
 * @returns the key, or `undefined` for anything that is not a keyed user message.
 */
function promptKeyIn(data: unknown): string | undefined {
  if (typeof data !== 'object' || data === null) return undefined
  const source = (data as Record<string, unknown>)['source']
  if (typeof source !== 'object' || source === null) return undefined
  const { kind, rpcId } = source as { kind?: unknown; rpcId?: unknown }
  if (kind !== 'user' || typeof rpcId !== 'string' || rpcId === '') return undefined
  return rpcId
}

/**
 * The `data` of the event inside one snapshot record.
 *
 * A snapshot's `records` are event envelopes (`{type, event}`), while a live frame *is* the event, so
 * the two arrive in different shapes and are reduced to one here.
 *
 * @param record - one entry of a snapshot's `records`.
 * @returns the event's `data`, or `undefined` if the record is not an event envelope.
 */
function eventDataIn(record: unknown): unknown {
  if (typeof record !== 'object' || record === null) return undefined
  const event = (record as Record<string, unknown>)['event']
  if (typeof event !== 'object' || event === null) return undefined
  return (event as Record<string, unknown>)['data']
}

export class SessionLayer {
  readonly #deps: SessionLayerDeps
  readonly #follows = new Map<string, FollowState>()
  readonly #prompts = new Map<string, PromptRecord>()
  /**
   * Prompt keys each conversation has *already used*, as seen in its own history.
   *
   * Separate from {@link SessionLayer.#prompts}, which remembers only what this process sent: the
   * Harness remembers a key for the life of the conversation, so a key can be spent by a **previous
   * run of the panel**. See {@link SessionLayer.prompt} for what a spent key costs.
   */
  readonly #usedKeys = new Map<string, Set<string>>()
  /** Session ids the user is currently looking at, newest attach wins. */
  #activeSessionId: string | undefined
  /** Monotonic generation source, per layer (not per session). */
  #generation = 0

  /**
   * @param deps - adapter, sender, defaults, and logger.
   */
  constructor(deps: SessionLayerDeps) {
    this.#deps = deps
  }

  /** Session currently attached by the peer, if any. */
  get activeSessionId(): string | undefined {
    return this.#activeSessionId
  }

  /** Session ids this layer owns; used as the interaction-ownership predicate. */
  ownedSessionIds(): readonly string[] {
    return [...this.#follows.keys()]
  }

  /** Snapshot of subscription state for diagnostics. */
  describe(): Record<string, unknown> {
    return {
      activeSessionId: this.#activeSessionId ?? null,
      follows: [...this.#follows.values()].map(state => ({
        sessionId: state.sessionId,
        generation: state.generation,
        cursor: state.cursor,
        snapshotDelivered: state.snapshotDelivered,
      })),
      prompts: [...this.#prompts.values()].map(record => ({
        requestId: record.requestId,
        sessionId: record.sessionId,
        state: record.state,
      })),
    }
  }

  /** List workspaces, required to be non-empty before a session can be created. */
  listWorkspaces(): readonly WorkspaceView[] {
    return this.#deps.harness.listWorkspaces()
  }

  /**
   * Initialize (or resolve) the Harness's first-use default workspace.
   *
   * A thin pass-through, like {@link SessionLayer.listWorkspaces}: the adapter owns what
   * "initialize" means upstream, and this layer owns only the fact that the peer asked. The
   * adapter's contract is "never throws, empty list means none" — so the peer's answer is
   * always a list, and the panel's `defaultWorkspace` state is derived from whether it has one.
   */
  async initializeDefaultWorkspace(): Promise<readonly WorkspaceView[]> {
    return await this.#deps.harness.initializeDefaultWorkspace()
  }

  /** List Harness sessions. */
  async listSessions(): Promise<readonly SessionSummaryView[]> {
    return await this.#deps.harness.listSessions()
  }

  /**
   * Create a session in an explicitly chosen workspace.
   *
   * @param workspaceId - caller-selected workspace; empty selects the configured default.
   * @returns the new session identity.
   * @throws {HarnessError} with code `unavailable` when no workspace can be chosen.
   */
  async createSession(workspaceId: string): Promise<{ sessionId: string }> {
    const chosen = workspaceId !== '' ? workspaceId : this.#deps.defaultWorkspaceId()
    if (chosen === '') {
      throw new HarnessError('unavailable', 'no workspace selected and no default workspace configured', {
        hint: 'Set defaultWorkspaceId in the plugin config, or pick a workspace in the panel.',
      })
    }
    const workspace = this.#deps.harness.workspace(chosen)
    if (workspace === undefined) {
      const known = this.#deps.harness.listWorkspaces().map(entry => entry.workspaceId)
      throw new HarnessError('stale', `workspace ${chosen} is no longer registered`, { workspaceId: chosen, known })
    }
    const created = await this.#deps.harness.createSession(chosen)
    this.#deps.onStateChange?.(created.sessionId, 'created', { workspaceId: chosen })
    return created
  }

  /**
   * Attach (or re-attach) one session and start forwarding its frames.
   *
   * Re-attaching the same session replaces the previous subscription instead of
   * running two: two subscriptions would double every frame and make the cursor
   * meaningless.
   *
   * @param sessionId - durable session identity.
   * @returns the subscription identity.
   */
  async attach(sessionId: string): Promise<AttachResult> {
    const previous = this.#follows.get(sessionId)
    if (previous !== undefined) this.#closeFollow(previous, 'replaced')
    const generation = ++this.#generation
    // Frames are forwarded **in order**, one at a time. A snapshot is no longer one
    // notification but a sequence, and an event forwarded while its parts were still
    // being written would reach the peer ahead of the conversation it belongs to — and
    // be counted stale there, which is precisely the bug chunking exists to fix.
    let chain: Promise<void> = Promise.resolve()
    const handle = this.#deps.harness.follow(sessionId, frame => {
      chain = chain
        .then(() => this.#forward(sessionId, generation, frame))
        .catch(error => {
          // A subscriber that throws must not tear down the durable subscription, and
          // an unhandled rejection here would surface as a plugin crash.
          this.#deps.log.warn('forwarding a follow frame failed', { sessionId, generation, error: String(error) })
        })
    })
    const state: FollowState = {
      sessionId,
      generation,
      handle,
      cursor: -1,
      snapshotDelivered: false,
      closed: false,
    }
    this.#follows.set(sessionId, state)
    this.#activeSessionId = sessionId
    this.#deps.onStateChange?.(sessionId, 'attached', { generation, replaced: previous !== undefined })
    // The subscription is durable but its frames are asynchronous; a subscriber
    // that never receives a snapshot must be told so by the absence of frames,
    // not by a fabricated empty conversation.
    void handle.done
      .then(() => {
        if (state.closed) return
        this.#deps.log.warn('follow subscription ended unexpectedly', { sessionId, generation })
        this.#deps.onStateChange?.(sessionId, 'subscription-lost', { generation })
      })
      .catch(error => {
        // A subscription that fails to even start must be reported, not thrown
        // into the host's shared event chain.
        this.#deps.log.warn('follow subscription failed', { sessionId, generation, error: String(error) })
        this.#deps.onStateChange?.(sessionId, 'subscription-failed', { generation })
      })
    return { sessionId, generation, replaced: previous !== undefined }
  }

  /** Stop following one session; the Harness-side task keeps running. */
  detach(sessionId: string): boolean {
    const state = this.#follows.get(sessionId)
    if (state === undefined) return false
    this.#closeFollow(state, 'detached')
    this.#follows.delete(sessionId)
    if (this.#activeSessionId === sessionId) this.#activeSessionId = undefined
    return true
  }

  /**
   * Send one conversation's opening snapshot again.
   *
   * The peer asks for this when a `session/snapshot` sequence did not complete — a part
   * was refused at the frame ceiling, or the sequence stalled — and the alternative to a
   * resend is a panel that stays blank for the rest of the run. Re-attaching is the
   * recovery that exists: it is the same path that delivered the snapshot the first time,
   * so the records are re-read rather than replayed from a cache that could have gone
   * stale. The peer bounds how often it asks (per attach), so this cannot become a loop
   * between the two halves.
   *
   * @param sessionId - the conversation whose snapshot is wanted.
   * @returns the subscription identity the resend was opened on.
   */
  async requestSnapshot(sessionId: string): Promise<{ sessionId: string; generation: number; accepted: true }> {
    const result = await this.attach(sessionId)
    this.#deps.onStateChange?.(sessionId, 'snapshot-requested', { generation: result.generation })
    return { sessionId, generation: result.generation, accepted: true }
  }

  /**
   * Push one session's statistics to the peer.
   *
   * Called on attach and once per finished turn rather than per frame: the numbers only change when
   * a step closes, and the read is a projection walk. It is also fired after a prompt is admitted,
   * because that is when `running` flips and the panel's footer starts showing progress.
   *
   * A failure is logged and swallowed — statistics are an adornment, and no session call may fail
   * because a counter could not be read.
   *
   * @param sessionId - the session to report on.
   */
  async refreshStats(sessionId: string): Promise<void> {
    let stats: SessionStatsView | undefined
    try {
      stats = await this.#deps.harness.stats(sessionId)
    } catch (error) {
      this.#deps.log.debug('statistics read failed', { sessionId, error: String(error) })
      return
    }
    if (stats === undefined) return
    await this.#deps.notify('session/stats', { sessionId, stats })
  }

  /** Read the model and permission choices the panel offers. */
  async readOptions(sessionId?: string): Promise<SessionOptionsView> {
    return await this.#deps.harness.options(sessionId)
  }

  /** Switch one session's model and reasoning effort. */
  async selectModel(sessionId: string, selection: ModelChoiceView): Promise<void> {
    await this.#deps.harness.selectModel(sessionId, selection)
    // The picker's own answer is the projection the host just wrote; re-reading it here means the
    // panel shows what was *accepted* rather than what it asked for.
    this.#deps.onStateChange?.(sessionId, 'model-selected', selection)
  }

  /** Apply one permission preset. */
  async setPermission(sessionId: string, value: string): Promise<void> {
    await this.#deps.harness.setPermission(sessionId, value)
    this.#deps.onStateChange?.(sessionId, 'permission-set', { value })
  }

  /** Stop every subscription; used on plugin unload and on channel loss. */
  detachAll(reason: string): void {
    for (const state of this.#follows.values()) this.#closeFollow(state, reason)
    this.#follows.clear()
    this.#activeSessionId = undefined
  }

  /**
   * Read one page of history.
   *
   * A page is cursor-relative upstream, so the bound is resolved in this order:
   * the live subscription's cursor (the peer's own position, and the only bound
   * that can include events that arrived while it was reading), then the
   * cold-safe persisted cursor. With neither, the session has no events and an
   * empty page is the truthful answer — inventing a large bound would be
   * rejected by Harness, and guessing would risk a page the caller cannot place.
   *
   * @param sessionId - durable session identity.
   * @param beforeSeq - exclusive backwards cursor within the bound.
   * @returns a whole-turn page, or a small acknowledgment after its bounded parts; the
   *   backwards cursor is the oldest record of the assembled page.
   */
  async readHistory(
    sessionId: string,
    beforeSeq?: number,
  ): Promise<HistoryPageView & { chunked?: true; generation?: number }> {
    const subscription = this.#follows.get(sessionId)
    let throughSeq = subscription !== undefined && subscription.cursor >= 0 ? subscription.cursor : undefined
    if (throughSeq === undefined) throughSeq = await this.#deps.harness.cursor(sessionId)
    if (throughSeq === undefined || throughSeq < 0) {
      this.#deps.log.debug('history requested for a session with no events', { sessionId })
      return { records: [], hasMore: false, cursor: null }
    }
    const page = await this.#deps.harness.pageHistory(sessionId, throughSeq, beforeSeq)
    // History is paged by message count upstream. The full records travel; only the
    // transport is chunked, so an indivisible turn is never cut to fit a frame.
    const records = page.records
    if (historyPageBytes(records) <= SNAPSHOT_PART_BUDGET_BYTES) return page
    const generation = ++this.#generation
    const context = { sessionId, generation }
    const parts = planSnapshotParts(records, context)
    await this.#deps.notify('session/history/begin', {
      ...context, beforeSeq, cursor: page.cursor, hasMore: page.hasMore, parts: parts.length,
    })
    for (const part of parts) {
      await this.#deps.notify('session/history/part', {
        ...context, beforeSeq, index: part.index, records: part.records,
      })
    }
    await this.#deps.notify('session/history/end', { ...context, beforeSeq, parts: parts.length })
    return { records: [], hasMore: page.hasMore, cursor: page.cursor, chunked: true, generation }
  }

  /**
   * Admit one prompt exactly once per user submission.
   *
   * `requestId` is generated by the caller (the peer) and remembered here. A
   * repeat of the same key returns the original acceptance; a *new* key for the
   * same text is a new user action and is admitted again, because only the peer
   * can tell "the user typed it twice" from "the frame was retried".
   *
   * @param sessionId - target session.
   * @param requestId - caller-owned idempotency key.
   * @param text - prompt text.
   */
  async prompt(sessionId: string, requestId: string, text: string): Promise<{ accepted: true }> {
    const existing = this.#prompts.get(requestId)
    if (existing !== undefined) {
      if (existing.state === 'admitted') return { accepted: true }
      if (existing.state === 'pending') {
        throw new HarnessError('unavailable', 'the same prompt is still awaiting confirmation from Harness', { requestId })
      }
      throw new HarnessError('unavailable', 'this prompt already failed; submit it again to retry deliberately', {
        requestId,
        detail: existing.detail,
      })
    }
    // **A key this conversation has already spent can never be admitted**, whatever the peer believes.
    // The Harness's session controller answers `{accepted: true}` to a repeated `requestId` *without
    // admitting anything* — `hasPromptRequest` matches the `user/message` whose `source.rpcId` is that
    // key — so forwarding it would tell the peer a message was delivered that never was. A restarted
    // panel can reuse `session-…:prompt-1` for a conversation that already has it, clearing its input
    // while nothing is sent. The peer keeps the text when it hears a refusal, so refusing is the half
    // that makes the mistake recoverable; the log line is the half that makes it findable.
    if (this.#usedKeys.get(sessionId)?.has(requestId) === true) {
      this.#deps.log.warn('refused a prompt whose key this conversation has already used', {
        sessionId,
        requestId,
      })
      this.#deps.onStateChange?.(sessionId, 'prompt-key-reused', { requestId })
      // The panel's copy lives in its dictionary. This error carries a stable reason the sidecar
      // can map to a code; the message stays a technical fallback for logs and older peers.
      throw new HarnessError(
        'stale',
        'prompt-key-reused',
        { sessionId, requestId, reason: 'promptKeyReused' },
      )
    }
    const record: PromptRecord = { requestId, sessionId, text, state: 'pending' }
    this.#prompts.set(requestId, record)
    try {
      const result = await this.#deps.harness.prompt(sessionId, requestId, text)
      record.state = 'admitted'
      this.#deps.onStateChange?.(sessionId, 'prompt-admitted', { requestId })
      return result
    } catch (error) {
      record.state = 'failed'
      record.detail = error instanceof Error ? error.message : String(error)
      throw error
    }
  }

  /**
   * Remember the prompt keys a conversation's history carries.
   *
   * Best effort on purpose: the opening snapshot is paged (it reports `hasMore`), so a key spent far
   * enough back may never be seen, and a guard that cannot see a key lets it through. That direction is
   * the safe one — a missed refusal costs what a refusal would have saved, while a *wrong* refusal would
   * refuse a message that could have been sent.
   *
   * @param sessionId - the conversation the records belong to.
   * @param datas - event `data` values, from a snapshot's records or from one event frame.
   */
  #rememberUsedKeys(sessionId: string, datas: readonly unknown[]): void {
    let keys: Set<string> | undefined
    for (const data of datas) {
      const key = promptKeyIn(data)
      if (key === undefined) continue
      keys ??= this.#usedKeysFor(sessionId)
      keys.add(key)
    }
  }

  /**
   * The key set for one conversation, created on first use.
   *
   * @param sessionId - the conversation.
   * @returns its set, to add to.
   */
  #usedKeysFor(sessionId: string): Set<string> {
    const existing = this.#usedKeys.get(sessionId)
    if (existing !== undefined) return existing
    const created = new Set<string>()
    this.#usedKeys.set(sessionId, created)
    return created
  }

  /**
   * Request cancellation.
   *
   * @param sessionId - target session.
   */
  async cancel(sessionId: string): Promise<{ accepted: true }> {
    const result = await this.#deps.harness.cancel(sessionId)
    // Acceptance means the request was received. The authoritative outcome is a
    // `turn/end` event with an aborted reason; until then the peer must keep
    // showing a cancelling state, so no terminal state is published here.
    this.#deps.onStateChange?.(sessionId, 'cancel-requested', {})
    return result
  }

  /**
   * Forward one upstream frame as a protocol notification.
   *
   * @param sessionId - the session the frame belongs to.
   * @param generation - the subscription generation that produced it.
   * @param frame - the projected frame.
   */
  async #forward(sessionId: string, generation: number, frame: FollowFrame): Promise<void> {
    const state = this.#follows.get(sessionId)
    if (state === undefined || state.generation !== generation || state.closed) {
      // A frame from a replaced subscription must not reach the peer: it would
      // append content the peer already received from the newer subscription.
      this.#deps.log.debug('dropped a frame from a replaced subscription', { sessionId, generation })
      return
    }
    switch (frame.type) {
      case 'snapshot': {
        state.snapshotDelivered = true
        state.cursor = frame.cursor
        // Before the peer sees the snapshot: it may prompt the moment it has it, and a prompt judged
        // against a stale key set is a prompt that slips through the guard below.
        this.#rememberUsedKeys(sessionId, frame.records.map(record => eventDataIn(record)))
        await this.#sendSnapshot(sessionId, generation, frame, state)
        // The snapshot carries projections, but only this one: their later updates travel on a
        // different stream the panel does not open. So the first statistics are sent here, and
        // refreshed when a turn closes.
        await this.refreshStats(sessionId)
        return
      }
      case 'event': {
        if (!state.snapshotDelivered) {
          // Events before the snapshot would be applied to an unknown baseline.
          // The protocol's rule is to wait, not to guess an ordering.
          this.#deps.log.warn('dropped an event that arrived before the snapshot', { sessionId, seq: frame.seq })
          return
        }
        if (frame.seq <= state.cursor) {
          this.#deps.log.debug('dropped a duplicate event', { sessionId, seq: frame.seq, cursor: state.cursor })
          return
        }
        if (frame.seq !== state.cursor + 1) {
          // A gap means this subscription can no longer be trusted to be
          // gap-free; the peer is told to re-attach rather than shown a
          // conversation with a hole in it.
          this.#deps.log.warn('sequence gap detected; requesting resync', {
            sessionId,
            expected: state.cursor + 1,
            received: frame.seq,
          })
          await this.#deps.notify('session/resync', {
            sessionId,
            reason: 'sequence-gap',
            expected: state.cursor + 1,
            received: frame.seq,
          })
          return
        }
        state.cursor = frame.seq
        if (frame.eventType === 'user/message') this.#rememberUsedKeys(sessionId, [frame.data])
        await this.#deps.notify('session/event', {
          sessionId,
          generation,
          seq: frame.seq,
          type: frame.eventType,
          time: frame.time,
          data: frame.data,
        })
        // The one event that makes the statistics stale: `turn/end` closes the last step of a turn,
        // which is exactly when turns, steps, decode time and token usage stop changing. Awaited so
        // the panel receives the totals no later than the event that produced them.
        if (frame.eventType === 'turn/end' || frame.eventType === 'step/end') {
          await this.refreshStats(sessionId)
        }
        return
      }
      case 'assistant-stream': {
        if (!state.snapshotDelivered) return
        await this.#deps.notify('session/stream', { sessionId, generation, frame: frame.frame })
        return
      }
      default: {
        this.#deps.log.debug('ignored an unknown frame type')
      }
    }
  }

  /**
   * Send one conversation's opening snapshot as `begin` + parts + `end`.
   *
   * The chunks keep each transport frame comfortably below the frame ceiling; a single
   * record over the part budget still travels whole, and the transport fragments it.
   *
   * `state.closed` is checked between parts because a replaced subscription's remaining
   * parts belong to a conversation nobody is following: writing them would only make the
   * peer discard them (its own identity check) after spending the bandwidth.
   *
   * @param sessionId - durable session identity.
   * @param generation - the subscription generation these parts belong to.
   * @param frame - the upstream snapshot.
   * @param state - the subscription the frame arrived on.
   */
  async #sendSnapshot(
    sessionId: string,
    generation: number,
    frame: SnapshotFrame,
    state: FollowState,
  ): Promise<void> {
    const context = { sessionId, generation }
    const records = frame.records
    const parts = planSnapshotParts(records, context)
    for (const part of parts) {
      if (part.bytes > SNAPSHOT_PART_BUDGET_BYTES) {
        // Not fatal — the part still travels — but the one case the budget cannot
        // cover must be findable rather than mysterious if it ever exceeds the ceiling.
        this.#deps.log.debug('a snapshot record is larger than the part budget; it travels as its own part', {
          sessionId,
          index: part.index,
          bytes: part.bytes,
        })
      }
    }
    await this.#deps.notify('session/snapshot/begin', {
      sessionId,
      generation,
      cursor: frame.cursor,
      hasMore: frame.hasMore,
      parts: parts.length,
      projections: frame.projections,
      assistantStream: frame.assistantStream,
    })
    for (const part of parts) {
      if (state.closed) return
      await this.#deps.notify('session/snapshot/part', {
        sessionId,
        generation,
        index: part.index,
        records: part.records,
      })
    }
    if (state.closed) return
    await this.#deps.notify('session/snapshot/end', { sessionId, generation, parts: parts.length })
  }

  /**
   * Close one subscription and mark it so late frames are dropped.
   *
   * @param state - the subscription to close.
   * @param reason - why it is being closed.
   */
  #closeFollow(state: FollowState, reason: string): void {
    if (state.closed) return
    state.closed = true
    try {
      state.handle.close()
    } catch (error) {
      this.#deps.log.warn('closing a follow subscription failed', error)
    }
    this.#deps.onStateChange?.(state.sessionId, 'detached', { generation: state.generation, reason })
  }
}

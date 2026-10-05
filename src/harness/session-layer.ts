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

import type { FollowFrame, FollowHandle, QuorfloatHarness, SessionSummaryView, WorkspaceView } from './adapter.js'
import { HarnessError } from './adapter.js'

/** Minimal logger shape used here. */
export interface SessionLayerLogger {
  warn(message: string, detail?: unknown): void
  debug(message: string, detail?: unknown): void
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

export class SessionLayer {
  readonly #deps: SessionLayerDeps
  readonly #follows = new Map<string, FollowState>()
  readonly #prompts = new Map<string, PromptRecord>()
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
    const handle = this.#deps.harness.follow(sessionId, frame => {
      void this.#forward(sessionId, generation, frame)
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
   */
  async readHistory(sessionId: string, beforeSeq?: number): Promise<{ records: readonly unknown[]; hasMore: boolean }> {
    const subscription = this.#follows.get(sessionId)
    let throughSeq = subscription !== undefined && subscription.cursor >= 0 ? subscription.cursor : undefined
    if (throughSeq === undefined) throughSeq = await this.#deps.harness.cursor(sessionId)
    if (throughSeq === undefined || throughSeq < 0) {
      this.#deps.log.debug('history requested for a session with no events', { sessionId })
      return { records: [], hasMore: false }
    }
    return await this.#deps.harness.pageHistory(sessionId, throughSeq, beforeSeq)
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
        await this.#deps.notify('session/snapshot', {
          sessionId,
          generation,
          cursor: frame.cursor,
          hasMore: frame.hasMore,
          records: frame.records,
          projections: frame.projections,
          assistantStream: frame.assistantStream,
        })
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
        await this.#deps.notify('session/event', {
          sessionId,
          generation,
          seq: frame.seq,
          type: frame.eventType,
          time: frame.time,
          data: frame.data,
        })
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

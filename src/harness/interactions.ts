/**
 * Approval and user-question answerers.
 *
 * Both upstream events are **agent-scoped waterfalls**: a listener may claim a
 * request by returning a value, or delegate by calling `next()`. This file
 * claims a request only when all of the following hold, and delegates the rest:
 *
 * 1. the owning session is one this host is currently tracking;
 * 2. the peer is connected and the channel is handshaken;
 * 3. the request has not already been settled by someone else.
 *
 * The third rule is what makes "the main window and the floating panel both show
 * the same request, but only one answer takes effect" true rather than hopeful:
 * a claimed request is removed from the pending map the moment it settles, so a
 * late answer finds nothing to settle and is refused instead of re-executed.
 * A session this host does not own is never claimed, so the in-window answerer
 * keeps working exactly as before.
 */

import type { Disposer, Logger, PluginContext } from '../cordis-types.js'

/** Approval outcomes accepted by the upstream waterfall. */
export type ApprovalOutcome = 'allowed-once' | 'rejected' | 'cancelled' | 'unavailable'

/** Payload of an upstream `approval/request`. */
export interface ApprovalRequestEvent {
  readonly agent?: { readonly id?: unknown }
  readonly toolName?: unknown
  readonly callId?: unknown
  readonly reason?: unknown
  readonly signal?: AbortSignal
}

/** One question inside an upstream `user-questions/request`. */
export interface QuestionItem {
  readonly id: string
  readonly question: string
  readonly detail?: string
  readonly header?: string
  readonly options?: readonly { readonly label: string; readonly description?: string }[]
  readonly multiSelect?: boolean
  readonly intent?: unknown
}

/** Payload of an upstream `user-questions/request`. */
export interface QuestionsRequestEvent {
  readonly agent?: { readonly id?: unknown }
  readonly questions?: readonly QuestionItem[]
  readonly signal?: AbortSignal
}

/** Answer shape returned to the user-questions waterfall. */
export interface QuestionsAnswer {
  readonly answers: readonly {
    readonly id: string
    readonly selected: readonly string[]
    readonly custom?: string
  }[]
}

/** What the peer is allowed to answer. */
export type PeerAnswer =
  | { readonly kind: 'approval'; readonly outcome: 'allowed-once' | 'rejected' }
  | {
      readonly kind: 'question'
      readonly answers: readonly { readonly id: string; readonly selected: readonly string[]; readonly custom?: string }[]
    }

/** A request currently waiting for the peer. */
interface PendingInteraction {
  readonly interactionId: string
  readonly sessionId: string
  readonly kind: 'approval' | 'question'
  settle(answer: PeerAnswer): void
  abort(reason: string): void
}

/** Inputs this file needs from the plugin around it. */
export interface InteractionsDeps {
  /** The Cordis context that owns the answerer registrations. */
  ctx: PluginContext
  /** Session ids this host currently tracks; the ownership predicate. */
  ownedSessionIds(): readonly string[]
  /** Send one notification to the peer. */
  notify(method: string, params: unknown): Promise<void>
  /** Publish a status change for the status/settings surface. */
  onStateChange?(sessionId: string, reason: string, detail?: unknown): void
  log: Logger
}

/** Result of attempting to answer from the peer side. */
export interface AnswerResult {
  readonly accepted: boolean
  readonly reason?: string
}

let counter = 0

/** Allocate a process-unique interaction id. */
function nextInteractionId(kind: 'approval' | 'question'): string {
  counter += 1
  return `${kind}-${Date.now().toString(36)}-${counter}`
}

export class Interactions {
  readonly #deps: InteractionsDeps
  readonly #pending = new Map<string, PendingInteraction>()
  /** Registrations to release on unload. */
  readonly #disposers: Disposer[] = []
  /** Counters that make answer routing visible in diagnostics. */
  readonly #counters = {
    claimed: 0,
    delegated: 0,
    answered: 0,
    refused: 0,
    aborted: 0,
    registrationFailures: 0,
  }
  #registered = false

  /**
   * @param deps - context, ownership predicate, sender, and logger.
   */
  constructor(deps: InteractionsDeps) {
    this.#deps = deps
  }

  /** True when both answerers were registered. */
  get registered(): boolean {
    return this.#registered
  }

  /** Diagnostic view of interaction routing. */
  describe(): Record<string, unknown> {
    return {
      registered: this.#registered,
      pending: [...this.#pending.values()].map(entry => ({
        interactionId: entry.interactionId,
        sessionId: entry.sessionId,
        kind: entry.kind,
      })),
      counters: { ...this.#counters },
    }
  }

  /**
   * Register both answerers on the owning context.
   *
   * Registration is best-effort: on a Harness build that does not compose these
   * events, the plugin must still run its text path, and the settings surface
   * reports that approvals cannot be answered from the panel.
   *
   * @returns whether both registrations succeeded.
   */
  register(): boolean {
    if (this.#registered) return true
    let failures = 0
    const attempt = (event: string, listener: (...args: any[]) => unknown): void => {
      try {
        const dispose = this.#deps.ctx.on(event, listener)
        this.#disposers.push(dispose)
      } catch (error) {
        failures += 1
        this.#deps.log.warn(`could not register an answerer for ${event}`, error)
      }
    }
    attempt('approval/request', this.#approvalAnswerer())
    attempt('user-questions/request', this.#questionAnswerer())
    this.#counters.registrationFailures = failures
    this.#registered = failures === 0
    this.#deps.log.info('interaction answerers registered', { failures })
    return this.#registered
  }

  /** Release both registrations. */
  unregister(): void {
    for (const dispose of this.#disposers.splice(0)) {
      try {
        dispose()
      } catch (error) {
        this.#deps.log.warn('releasing an answerer registration failed', error)
      }
    }
    this.abortAll('plugin unloading')
    this.#registered = false
  }

  /**
   * Settle one pending interaction from the peer.
   *
   * @param interactionId - identity returned in `interaction/open`.
   * @param answer - the peer's answer payload.
   * @returns whether an answer was accepted; a refusal is not an error.
   */
  answer(interactionId: string, answer: unknown): AnswerResult {
    const pending = this.#pending.get(interactionId)
    if (pending === undefined) {
      // Already answered, already cancelled, or answered by another surface.
      // Refusing is the correct response: re-applying it would re-execute a
      // decision the authoritative side already settled.
      this.#counters.refused += 1
      return { accepted: false, reason: 'no such pending interaction (already settled, cancelled, or never owned here)' }
    }
    const parsed = parsePeerAnswer(pending.kind, answer)
    if (parsed === undefined) {
      this.#counters.refused += 1
      return { accepted: false, reason: `answer does not match ${articleFor(pending.kind)} ${pending.kind} interaction` }
    }
    this.#pending.delete(interactionId)
    this.#counters.answered += 1
    pending.settle(parsed)
    return { accepted: true }
  }

  /**
   * Withdraw every pending request, e.g. when the peer went away.
   *
   * @param reason - why the requests are being withdrawn.
   */
  abortAll(reason: string): void {
    for (const [id, pending] of this.#pending) {
      this.#pending.delete(id)
      this.#counters.aborted += 1
      pending.abort(reason)
    }
  }

  /**
   * Build the `approval/request` listener.
   *
   * @returns a listener that claims owned requests and delegates the rest.
   */
  #approvalAnswerer(): (request: ApprovalRequestEvent, next: () => Promise<ApprovalOutcome>) => Promise<ApprovalOutcome> {
    return async (request, next) => {
      const sessionId = sessionOf(request)
      if (sessionId === undefined || !this.#owns(sessionId)) {
        this.#counters.delegated += 1
        return await next()
      }
      this.#counters.claimed += 1
      const interactionId = nextInteractionId('approval')
      const outcome = await this.#await(
        {
          interactionId,
          sessionId,
          kind: 'approval',
          payload: {
            toolName: typeof request.toolName === 'string' ? request.toolName : 'unknown',
            callId: typeof request.callId === 'string' ? request.callId : null,
            reason: typeof request.reason === 'string' ? request.reason : null,
          },
        },
        request.signal,
      )
      if (outcome === undefined) return 'cancelled'
      if (outcome.kind !== 'approval') {
        this.#deps.log.warn('a question answer arrived for an approval request; failing closed')
        return 'unavailable'
      }
      return outcome.outcome
    }
  }

  /**
   * Build the `user-questions/request` listener.
   *
   * @returns a listener that claims owned requests and delegates the rest.
   */
  #questionAnswerer(): (request: QuestionsRequestEvent, next: () => Promise<QuestionsAnswer>) => Promise<QuestionsAnswer> {
    return async (request, next) => {
      const sessionId = sessionOf(request)
      if (sessionId === undefined || !this.#owns(sessionId)) {
        this.#counters.delegated += 1
        return await next()
      }
      const questions = Array.isArray(request.questions) ? request.questions : []
      if (questions.length === 0) {
        // Nothing to ask: delegate rather than claim a malformed request.
        this.#counters.delegated += 1
        return await next()
      }
      this.#counters.claimed += 1
      const interactionId = nextInteractionId('question')
      const outcome = await this.#await(
        {
          interactionId,
          sessionId,
          kind: 'question',
          payload: { questions },
        },
        request.signal,
      )
      if (outcome === undefined) {
        // Withdrawn: the upstream contract has no "cancelled" answer shape, so
        // the request is passed on instead of being invented as an empty answer.
        return await next()
      }
      if (outcome.kind !== 'question') {
        this.#deps.log.warn('an approval answer arrived for a question request; delegating')
        return await next()
      }
      return { answers: outcome.answers }
    }
  }

  /**
   * Publish one request to the peer and wait for its answer.
   *
   * @param request - identity, session, kind, and payload.
   * @param signal - upstream cancellation lifetime.
   * @returns the peer's answer, or `undefined` when the request was withdrawn.
   */
  async #await(
    request: { interactionId: string; sessionId: string; kind: 'approval' | 'question'; payload: unknown },
    signal: AbortSignal | undefined,
  ): Promise<PeerAnswer | undefined> {
    if (signal?.aborted === true) return undefined
    const settled = Promise.withResolvers<PeerAnswer>()
    let finished = false
    const pending: PendingInteraction = {
      interactionId: request.interactionId,
      sessionId: request.sessionId,
      kind: request.kind,
      settle: answer => {
        if (finished) return
        finished = true
        cleanup()
        settled.resolve(answer)
      },
      abort: reason => {
        if (finished) return
        finished = true
        cleanup()
        this.#deps.log.info('withdrawing a pending interaction', { interactionId: request.interactionId, reason })
        settled.resolve(undefined as unknown as PeerAnswer)
      },
    }
    const onAbort = (): void => {
      if (finished) return
      this.#pending.delete(request.interactionId)
      finished = true
      cleanup()
      // The asker withdrew the question: settle as cancelled so the upstream
      // waterfall is not left waiting on a request nobody can answer.
      settled.resolve(undefined as unknown as PeerAnswer)
    }
    const cleanup = (): void => {
      signal?.removeEventListener('abort', onAbort)
    }
    signal?.addEventListener('abort', onAbort, { once: true })
    this.#pending.set(request.interactionId, pending)
    this.#deps.onStateChange?.(request.sessionId, 'interaction-open', {
      interactionId: request.interactionId,
      kind: request.kind,
    })
    try {
      await this.#deps.notify('interaction/open', {
        interactionId: request.interactionId,
        sessionId: request.sessionId,
        kind: request.kind,
        payload: request.payload,
      })
    } catch (error) {
      // If the peer cannot even be told, the request must not hang the turn.
      this.#deps.log.warn('could not publish an interaction to quorfloat', error)
      this.#pending.delete(request.interactionId)
      if (!finished) {
        finished = true
        cleanup()
        return undefined
      }
    }
    const answer = await settled.promise
    return finished && answer === undefined ? undefined : answer
  }

  /**
   * Test the ownership predicate.
   *
   * @param sessionId - session identity from the request.
   * @returns whether this host tracks that session.
   */
  #owns(sessionId: string): boolean {
    return this.#deps.ownedSessionIds().includes(sessionId)
  }
}

/** Choose the indefinite article for an interaction kind, for readable messages. */
function articleFor(kind: 'approval' | 'question'): string {
  return kind === 'approval' ? 'an' : 'a'
}

/**
 * Read the owning session out of an agent-scoped request.
 *
 * An agent's `id` is its session id, which is what the in-window answerer also
 * uses to decide whether a request belongs to the session it is displaying.
 *
 * @param request - either upstream request shape.
 * @returns the session id, or `undefined` when the request carries no agent.
 */
function sessionOf(request: { readonly agent?: { readonly id?: unknown } }): string | undefined {
  const id = request.agent?.id
  return typeof id === 'string' && id !== '' ? id : undefined
}

/**
 * Validate a peer answer against the interaction kind.
 *
 * @param kind - the pending interaction's kind.
 * @param answer - raw peer payload.
 * @returns the validated answer, or `undefined` when it does not fit.
 */
function parsePeerAnswer(kind: 'approval' | 'question', answer: unknown): PeerAnswer | undefined {
  if (typeof answer !== 'object' || answer === null) return undefined
  const record = answer as Record<string, unknown>
  if (kind === 'approval') {
    const outcome = record['outcome']
    if (outcome === 'allowed-once' || outcome === 'rejected') return { kind: 'approval', outcome }
    return undefined
  }
  const answers = record['answers']
  if (!Array.isArray(answers)) return undefined
  const parsed: { id: string; selected: readonly string[]; custom?: string }[] = []
  for (const entry of answers) {
    if (typeof entry !== 'object' || entry === null) return undefined
    const item = entry as Record<string, unknown>
    if (typeof item['id'] !== 'string' || !Array.isArray(item['selected'])) return undefined
    const selected = item['selected'].filter((value): value is string => typeof value === 'string')
    const custom = typeof item['custom'] === 'string' ? item['custom'] : undefined
    parsed.push(custom === undefined ? { id: item['id'], selected } : { id: item['id'], selected, custom })
  }
  return { kind: 'question', answers: parsed }
}

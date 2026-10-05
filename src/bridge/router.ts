/**
 * Method routing for the host side of the protocol.
 *
 * The router owns method names, parameter validation, and the handshake
 * payloads. It delegates anything that touches Harness state to small injected
 * interfaces, so this file can be exercised without a Harness runtime and the
 * `src/index.ts` wiring stays the only place that knows about real services.
 */

import { ChannelError } from './errors.js'
import { PROTOCOL_VERSION } from '../protocol.js'
import type { QuorfloatConfig } from '../config.js'

/** What the quorfloat process reports about itself in `hello`. */
export interface HelloParams {
  readonly protocol?: unknown
  readonly quorfloatVersion?: unknown
  readonly platform?: unknown
  readonly arch?: unknown
  readonly capabilities?: unknown
  readonly hotkey?: unknown
}

/** Result of a successful handshake, kept for diagnostics. */
export interface HandshakeFacts {
  readonly protocol: string
  readonly quorfloatVersion: string
  readonly platform: string
  readonly arch: string
  readonly capabilities: readonly string[]
  /** `null` when no hotkey was requested, otherwise what the peer reported. */
  readonly hotkey: { readonly requested: string; readonly registered: boolean } | null
  readonly at: number
}

/** Workspace listing entry, already projected away from Harness internals. */
export interface WorkspaceView {
  readonly workspaceId: string
  readonly path: string
  readonly title: string
}

/** Session listing entry, already projected away from Harness internals. */
export interface SessionSummaryView {
  readonly sessionId: string
  readonly cwd?: string
  readonly updatedAt: number
  readonly running: boolean
  readonly blank: boolean
}

/** A page of durable history records, in the shape the wire can carry. */
export interface HistoryPageView {
  readonly records: readonly unknown[]
  readonly hasMore: boolean
}

/** Answer accepted for one pending approval or user question. */
export type InteractionAnswer =
  | { readonly kind: 'approval'; readonly outcome: 'allowed-once' | 'rejected' }
  | { readonly kind: 'question'; readonly answers: readonly { readonly id: string; readonly selected: readonly string[]; readonly custom?: string }[] }

/** Everything the router needs from the rest of the host plugin. */
export interface RouterHost {
  /** Effective configuration currently in force. */
  config(): QuorfloatConfig
  /** The running Harness host version, reported to the peer for diagnostics. */
  hostVersion(): string
  /** Identity of this channel instance; changes on every restart. */
  channelSessionId(): string
  /** List Harness workspaces the user can choose from. */
  listWorkspaces(): Promise<readonly WorkspaceView[]>
  /** List sessions visible in the Harness session list. */
  listSessions(cursor?: string): Promise<{ items: readonly SessionSummaryView[] }>
  /** Create or idempotently adopt one session inside a workspace. */
  createSession(workspaceId: string): Promise<{ sessionId: string }>
  /** Start (or restart) the follow subscription for one session. */
  attachSession(sessionId: string, fromSeq?: number): Promise<{ sessionId: string }>
  /** Read one page of durable history. */
  readHistory(sessionId: string, beforeSeq?: number): Promise<HistoryPageView>
  /** Admit one prompt; `requestId` is the caller's idempotency key. */
  prompt(sessionId: string, requestId: string, text: string): Promise<{ accepted: true }>
  /** Request cancellation of the active turn. */
  cancel(sessionId: string): Promise<{ accepted: true }>
  /** Answer one pending interaction owned by this host. */
  answerInteraction(interactionId: string, answer: unknown): Promise<{ accepted: boolean }>
  /**
   * Record one UI surface's presence report.
   *
   * Called from the browser half only; the panel's own visibility arrives over
   * the stdio protocol instead.
   */
  reportPresence(
    surface: string,
    report: { visible: boolean; focused: boolean; seq: number; at?: number },
  ): Promise<{ accepted: boolean; reason?: string }>
  /** Free-form diagnostics for the settings/status surface. */
  diagnostics(): Record<string, unknown>
}

/** Fields the router validates out of an inbound parameter object. */
function requireString(params: unknown, field: string, context: string): string {
  if (typeof params !== 'object' || params === null) {
    throw new ChannelError('unavailable', `${context}: params must be an object`)
  }
  const value = (params as Record<string, unknown>)[field]
  if (typeof value !== 'string' || value === '') {
    throw new ChannelError('unavailable', `${context}: ${field} must be a non-empty string`)
  }
  return value
}

/** Read an optional integer field, rejecting a present-but-invalid value. */
function optionalInt(params: unknown, field: string, context: string): number | undefined {
  if (typeof params !== 'object' || params === null) return undefined
  const value = (params as Record<string, unknown>)[field]
  if (value === undefined || value === null) return undefined
  if (!Number.isInteger(value)) {
    throw new ChannelError('unavailable', `${context}: ${field} must be an integer when present`)
  }
  return value as number
}

/**
 * Host-side method router.
 *
 * `hello` is the only method with protocol-level meaning; every other method is
 * forwarded to {@link RouterHost}. Methods are answered even while the
 * handshake is still in progress, because refusing early frames would turn a
 * slow start into a spurious failure.
 */
export class HostRouter {
  readonly #host: RouterHost
  #handshake: HandshakeFacts | undefined

  /**
   * @param host - the host capabilities the router exposes to the peer.
   */
  constructor(host: RouterHost) {
    this.#host = host
  }

  /** Handshake facts once `hello` succeeded; `undefined` before that. */
  get handshake(): HandshakeFacts | undefined {
    return this.#handshake
  }

  /** True once a compatible `hello` has been accepted. */
  get handshaken(): boolean {
    return this.#handshake !== undefined
  }

  /**
   * Handle one inbound request.
   *
   * @param method - JSON-RPC method name.
   * @param params - request parameters.
   * @returns the method result.
   * @throws {ChannelError} for unknown methods, bad parameters, or host failures.
   */
  async handle(method: string, params: unknown): Promise<unknown> {
    switch (method) {
      case 'hello':
        return this.#hello(params)
      case 'window/visibility':
        return { visible: this.#readVisibility(params) }
      case 'workspaces/list':
        return { items: await this.#host.listWorkspaces() }
      case 'sessions/list':
        return await this.#host.listSessions(this.#optionalString(params, 'cursor'))
      case 'session/create':
        return await this.#host.createSession(requireString(params, 'workspaceId', 'session/create'))
      case 'session/attach':
        return await this.#host.attachSession(
          requireString(params, 'sessionId', 'session/attach'),
          optionalInt(params, 'fromSeq', 'session/attach'),
        )
      case 'session/history':
        return await this.#host.readHistory(
          requireString(params, 'sessionId', 'session/history'),
          optionalInt(params, 'beforeSeq', 'session/history'),
        )
      case 'session/prompt':
        return await this.#host.prompt(
          requireString(params, 'sessionId', 'session/prompt'),
          requireString(params, 'requestId', 'session/prompt'),
          requireString(params, 'text', 'session/prompt'),
        )
      case 'session/cancel':
        return await this.#host.cancel(requireString(params, 'sessionId', 'session/cancel'))
      case 'presence/report':
        // The stdio route exists for tests and for a peer that reports the panel's
        // own presence. The browser half uses the gateway service instead, because
        // window visibility only exists in the page.
        return await this.#host.reportPresence(
          requireString(params, 'surface', 'presence/report'),
          readPresenceReport(params),
        )
      case 'interaction/answer':
        return await this.#host.answerInteraction(
          requireString(params, 'interactionId', 'interaction/answer'),
          (params as Record<string, unknown>)['answer'],
        )
      case 'diag/snapshot':
        return { config: this.#host.config(), handshake: this.#handshake ?? null, ...this.#host.diagnostics() }
      default:
        throw new ChannelError('unavailable', `unsupported method: ${method}`)
    }
  }

  /**
   * Validate the peer's `hello` and answer with this host's facts.
   *
   * @param params - `hello` parameters.
   * @returns the handshake response sent back to the peer.
   */
  #hello(params: unknown): Record<string, unknown> {
    const hello = (typeof params === 'object' && params !== null ? params : {}) as HelloParams
    const protocol = typeof hello.protocol === 'string' ? hello.protocol : ''
    if (protocol !== PROTOCOL_VERSION) {
      throw new ChannelError('protocol-mismatch', `unsupported protocol version: ${protocol || '(missing)'}`, {
        supported: [PROTOCOL_VERSION],
        received: protocol,
      })
    }
    const capabilities = Array.isArray(hello.capabilities)
      ? hello.capabilities.filter((entry): entry is string => typeof entry === 'string')
      : []
    const hotkey = readHotkey(hello.hotkey)
    this.#handshake = {
      protocol,
      quorfloatVersion: typeof hello.quorfloatVersion === 'string' ? hello.quorfloatVersion : 'unknown',
      platform: typeof hello.platform === 'string' ? hello.platform : 'unknown',
      arch: typeof hello.arch === 'string' ? hello.arch : 'unknown',
      capabilities,
      hotkey,
      at: Date.now(),
    }
    const config = this.#host.config()
    return {
      protocol: PROTOCOL_VERSION,
      hostVersion: this.#host.hostVersion(),
      // The session identity of this channel instance. It is what makes an
      // interaction answer verifiable: a stale channel id must not be able to
      // settle a request owned by a newer channel.
      sessionId: this.#host.channelSessionId(),
      capabilities: ['workspace', 'session', 'history', 'prompt', 'cancel', 'interaction'],
      window: config.window,
      hotkey: config.hotkey,
    }
  }

  /**
   * Record the peer's visibility report.
   *
   * @param params - `window/visibility` parameters.
   * @returns the acknowledged visibility flag.
   */
  #readVisibility(params: unknown): boolean {
    if (typeof params !== 'object' || params === null) {
      throw new ChannelError('unavailable', 'window/visibility: params must be an object')
    }
    const visible = (params as Record<string, unknown>)['visible']
    if (typeof visible !== 'boolean') {
      throw new ChannelError('unavailable', 'window/visibility: visible must be a boolean')
    }
    return visible
  }

  /**
   * Read an optional string parameter.
   *
   * @param params - parameter object.
   * @param field - field name.
   * @returns the string, or `undefined` when absent.
   */
  #optionalString(params: unknown, field: string): string | undefined {
    if (typeof params !== 'object' || params === null) return undefined
    const value = (params as Record<string, unknown>)[field]
    return typeof value === 'string' ? value : undefined
  }
}

/**
 * Read and validate one presence report from a browser surface.
 *
 * Both facts are required together: `visible` alone cannot distinguish "occluded
 * on Windows" from "in front of the user", and `focused` alone is true for a
 * window the user has on another display without looking at it.
 *
 * @param params - the raw request parameters.
 * @returns the validated report.
 * @throws {ChannelError} when a required field is missing or mistyped.
 */
function readPresenceReport(params: unknown): { visible: boolean; focused: boolean; seq: number; at?: number } {
  if (typeof params !== 'object' || params === null) {
    throw new ChannelError('unavailable', 'presence/report: params must be an object')
  }
  const record = params as Record<string, unknown>
  if (typeof record['visible'] !== 'boolean') {
    throw new ChannelError('unavailable', 'presence/report: visible must be a boolean')
  }
  if (typeof record['focused'] !== 'boolean') {
    throw new ChannelError('unavailable', 'presence/report: focused must be a boolean')
  }
  if (!Number.isInteger(record['seq'])) {
    throw new ChannelError('unavailable', 'presence/report: seq must be an integer')
  }
  const at = record['at']
  if (at !== undefined && !Number.isFinite(at)) {
    throw new ChannelError('unavailable', 'presence/report: at must be a finite number when present')
  }
  return at === undefined
    ? { visible: record['visible'], focused: record['focused'], seq: record['seq'] as number }
    : { visible: record['visible'], focused: record['focused'], seq: record['seq'] as number, at: at as number }
}

/**
 * Normalize the peer's hotkey report.
 *
 * @param value - raw `hello.hotkey` value.
 * @returns the normalized report, or `null` when the peer sent none.
 */
function readHotkey(value: unknown): { requested: string; registered: boolean } | null {
  if (typeof value !== 'object' || value === null) return null
  const record = value as Record<string, unknown>
  if (typeof record['requested'] !== 'string') return null
  return { requested: record['requested'], registered: record['registered'] === true }
}

/**
 * The stdio channel to one quorfloat process.
 *
 * Responsibilities, and only these:
 * - frame the child's stdout into JSON-RPC messages and classify rejections;
 * - route inbound requests to host-provided handlers and answer them;
 * - keep a request id → promise map with per-request timeouts;
 * - apply bounded write backpressure instead of queueing without limit;
 * - escalate shutdown: `shutdown` request → SIGTERM → SIGKILL, and report what
 *   actually happened rather than assuming a signal was enough.
 *
 * Business rules (sessions, interactions, configuration) belong to the caller;
 * this file never imports a Harness service.
 */

import type { ChildProcessWithoutNullStreams } from 'node:child_process'
import { createInterface } from 'node:readline'

import {
  ErrorCode,
  FrameDecoder,
  encodeFrame,
  failure,
  request as buildRequest,
  success,
  notification as buildNotification,
  type DecodedFrame,
  type FrameRejection,
  type RpcFailure,
  type RpcMessage,
  type RpcNotification,
  type RpcRequest,
  type RpcResponse,
  type RequestId,
} from '../protocol.js'
import { ChannelError, rpcCodeFor, type ChannelErrorCode } from './errors.js'

/** Lifecycle of one channel: created → started → closing → closed. */
export type ChannelState = 'created' | 'started' | 'closing' | 'closed'

/** Answer for one inbound request. */
export type RequestHandler = (params: unknown, method: string) => Promise<unknown> | unknown

export interface ChannelOptions {
  /** How long one request may wait for its response before failing. */
  readonly requestTimeoutMs: number
  /** Total bytes of unflushed stdin we tolerate before failing a write. */
  readonly maxWriteBufferBytes?: number
  /** Called for every inbound notification. Throwing here is contained. */
  readonly onNotification?: (method: string, params: unknown) => void
  /** Called for every rejected frame so the owner can log or count it. */
  readonly onRejectedFrame?: (rejection: FrameRejection) => void
  /** Called once when the channel can no longer be used. */
  readonly onClosed?: (reason: ChannelError) => void
}

/** One in-flight outbound request. */
interface PendingRequest {
  readonly method: string
  readonly resolve: (value: unknown) => void
  readonly reject: (error: ChannelError) => void
  readonly timer: NodeJS.Timeout
}

const DEFAULT_MAX_WRITE_BUFFER_BYTES = 4 * 1024 * 1024

export class QuorfloatChannel {
  readonly #child: ChildProcessWithoutNullStreams
  readonly #options: ChannelOptions
  readonly #decoder = new FrameDecoder()
  readonly #pending = new Map<RequestId, PendingRequest>()
  readonly #handlers = new Map<string, RequestHandler>()
  #nextId = 1
  #state: ChannelState = 'created'
  #closeReason: ChannelError | undefined
  #lastFrameAt = 0
  #writtenBytes = 0
  #readBytes = 0
  #rejectedFrames = 0
  #exitResult: { code: number | null; signal: NodeJS.Signals | null } | undefined

  /**
   * @param child - the spawned quorfloat process, with piped stdio.
   * @param options - timeouts, callbacks, and buffer bounds.
   */
  constructor(child: ChildProcessWithoutNullStreams, options: ChannelOptions) {
    this.#child = child
    this.#options = options
    for (const [method, handler] of Object.entries(DEFAULT_HANDLERS)) {
      this.#handlers.set(method, handler)
    }
  }

  /** Current lifecycle state. */
  get state(): ChannelState {
    return this.#state
  }

  /** Process id of the peer, or `undefined` once it has been reaped. */
  get pid(): number | undefined {
    return this.#child.pid
  }

  /** Timestamp (ms) of the most recent inbound frame; `0` before the first one. */
  get lastFrameAt(): number {
    return this.#lastFrameAt
  }

  /** Bytes read from the peer's stdout so far. */
  get readBytes(): number {
    return this.#readBytes
  }

  /** Bytes written to the peer's stdin so far (frames that were flushed). */
  get writtenBytes(): number {
    return this.#writtenBytes
  }

  /** Number of frames rejected since the channel started. */
  get rejectedFrames(): number {
    return this.#rejectedFrames
  }

  /** Resolution of the child's exit, when it has already exited. */
  get exitResult(): { code: number | null; signal: NodeJS.Signals | null } | undefined {
    return this.#exitResult
  }

  /**
   * Register or replace the handler for one inbound method.
   *
   * @param method - the JSON-RPC method name.
   * @param handler - called with the request parameters.
   */
  handle(method: string, handler: RequestHandler): void {
    this.#handlers.set(method, handler)
  }

  /** Attach stream listeners. Safe to call once. */
  start(): void {
    if (this.#state !== 'created') return
    this.#state = 'started'

    this.#child.stdout.on('data', (chunk: Buffer) => {
      this.#readBytes += chunk.length
      this.#lastFrameAt = Date.now()
      for (const frame of this.#decoder.push(chunk)) this.#dispatch(frame)
    })
    this.#child.stdout.on('error', (error: Error) => {
      this.#fail(new ChannelError('channel-closed', `stdout error: ${error.message}`))
    })

    // stderr is diagnostics only. It is forwarded line by line so quorfloat's
    // logs stay readable in the Harness log without ever touching stdout.
    const stderr = createInterface({ input: this.#child.stderr, crlfDelay: Infinity })
    stderr.on('line', (line: string) => this.#options.onNotification?.('$stderr', line))

    this.#child.on('error', (error: Error) => {
      this.#fail(new ChannelError('spawn-failed', `process error: ${error.message}`))
    })
    this.#child.on('exit', (code, signal) => {
      this.#exitResult = { code, signal }
      this.#fail(
        new ChannelError('helper-exited', `quorfloat exited (code=${String(code)}, signal=${String(signal)})`, {
          code,
          signal,
        }),
      )
    })
  }

  /**
   * Send one request and await its response.
   *
   * @param method - JSON-RPC method to call.
   * @param params - request parameters, omitted when `undefined`.
   * @param timeoutMs - override for the configured per-request budget.
   * @returns the response result.
   * @throws {ChannelError} on timeout, channel closure, or a peer error response.
   */
  async request<R = unknown>(method: string, params?: unknown, timeoutMs?: number): Promise<R> {
    if (this.#state !== 'started') {
      throw new ChannelError('channel-closed', `cannot call ${method}: channel is ${this.#state}`)
    }
    const id = this.#nextId++
    const message = buildRequest(id, method, params)
    const budget = timeoutMs ?? this.#options.requestTimeoutMs
    const answer = new Promise<unknown>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.#pending.delete(id)
        reject(
          new ChannelError('request-timeout', `${method} was not answered within ${budget}ms`, { method, id, budget }),
        )
      }, budget)
      timer.unref?.()
      this.#pending.set(id, { method, resolve, reject, timer })
    })
    try {
      await this.#write(message)
    } catch (error) {
      const pending = this.#pending.get(id)
      if (pending !== undefined) {
        clearTimeout(pending.timer)
        this.#pending.delete(id)
      }
      throw error
    }
    const result = await answer
    return result as R
  }

  /**
   * Send one notification (no response expected).
   *
   * @param method - JSON-RPC method name.
   * @param params - notification parameters.
   */
  async notify(method: string, params?: unknown): Promise<void> {
    if (this.#state !== 'started') return
    await this.#write(buildNotification(method, params))
  }

  /**
   * Close the channel: request a graceful stop, then escalate.
   *
   * The returned result reports what actually happened, because "we sent
   * SIGTERM" is not evidence that the process is gone.
   *
   * @param graceMs - wait budget for each escalation step.
   * @returns how the process ended.
   */
  async close(graceMs: number): Promise<{ exited: boolean; code: number | null; signal: NodeJS.Signals | null; escalated: boolean }> {
    if (this.#state === 'closed') {
      return { exited: true, code: this.#exitResult?.code ?? null, signal: this.#exitResult?.signal ?? null, escalated: false }
    }
    if (this.#state === 'created') {
      // Never started: nothing was written, so a signal is the only option.
      this.#state = 'closing'
      this.#child.kill('SIGTERM')
      const exited = await this.#waitForExit(graceMs)
      if (!exited) this.#child.kill('SIGKILL')
      const final = exited || (await this.#waitForExit(graceMs))
      this.#fail(new ChannelError('channel-closed', 'channel closed before start'))
      return { exited: final, code: this.#exitResult?.code ?? null, signal: this.#exitResult?.signal ?? null, escalated: !exited }
    }

    let escalated = false
    // The shutdown request is sent while the channel is still usable: refusing
    // our own request here would turn every orderly stop into a signal, which is
    // exactly the outcome this method exists to avoid.
    try {
      await this.request('shutdown', { reason: 'host-closing' }, graceMs)
    } catch {
      // The peer may already be gone or too busy to answer; the signal path below
      // is the authority, so this failure is not itself an error.
    }
    this.#state = 'closing'
    if (!(await this.#waitForExit(graceMs))) {
      escalated = true
      this.#child.kill('SIGTERM')
      if (!(await this.#waitForExit(graceMs))) {
        this.#child.kill('SIGKILL')
        await this.#waitForExit(graceMs)
      }
    }
    this.#fail(new ChannelError('channel-closed', 'channel closed by host'))
    return {
      exited: this.#exitResult !== undefined || this.#child.exitCode !== null || this.#child.signalCode !== null,
      code: this.#exitResult?.code ?? null,
      signal: this.#exitResult?.signal ?? null,
      escalated,
    }
  }

  /**
   * Wait until the child process has exited.
   *
   * @param budgetMs - maximum wait.
   * @returns whether the process exited inside the budget.
   */
  async #waitForExit(budgetMs: number): Promise<boolean> {
    if (this.#exitResult !== undefined || this.#child.exitCode !== null) return true
    if (budgetMs <= 0) return false
    return await new Promise<boolean>((resolve) => {
      const timer = setTimeout(() => {
        this.#child.off('exit', onExit)
        resolve(false)
      }, budgetMs)
      timer.unref?.()
      const onExit = (): void => {
        clearTimeout(timer)
        resolve(true)
      }
      this.#child.once('exit', onExit)
    })
  }

  /**
   * Write one frame, honouring a bounded queue.
   *
   * @param message - the message to encode and send.
   * @throws {ChannelError} when the channel is unusable or the peer is too far behind.
   */
  async #write(message: RpcMessage): Promise<void> {
    if (this.#state === 'closed' || this.#child.stdin.destroyed) {
      throw new ChannelError('channel-closed', 'cannot write: channel is closed')
    }
    const frame = encodeFrame(message)
    const bytes = Buffer.byteLength(frame)
    if (this.#child.stdin.writableLength + bytes > (this.#options.maxWriteBufferBytes ?? DEFAULT_MAX_WRITE_BUFFER_BYTES)) {
      throw new ChannelError('unavailable', 'quorfloat is not consuming frames fast enough', {
        queuedBytes: this.#child.stdin.writableLength,
        frameBytes: bytes,
      })
    }
    this.#writtenBytes += bytes
    // `stdin.write` only reports false when the kernel buffer is full; awaiting
    // 'drain' then keeps memory bounded instead of buffering without limit.
    if (!this.#child.stdin.write(frame)) {
      await new Promise<void>((resolve, reject) => {
        const onDrain = (): void => {
          cleanup()
          resolve()
        }
        const onError = (error: Error): void => {
          cleanup()
          reject(new ChannelError('channel-closed', `stdin error: ${error.message}`))
        }
        const cleanup = (): void => {
          this.#child.stdin.off('drain', onDrain)
          this.#child.stdin.off('error', onError)
        }
        this.#child.stdin.once('drain', onDrain)
        this.#child.stdin.once('error', onError)
      })
    }
  }

  /**
   * Handle one decoded frame.
   *
   * @param frame - decoded frame from the child's stdout.
   */
  #dispatch(frame: DecodedFrame): void {
    if (!frame.ok) {
      this.#rejectedFrames += 1
      this.#options.onRejectedFrame?.(frame.rejection)
      // A malformed frame is a peer bug, not a reason to drop the channel: the
      // documented behaviour is to ignore it and keep serving valid frames.
      return
    }
    switch (frame.kind) {
      case 'success': {
        const response = frame.message as RpcResponse
        this.#settle(response)
        return
      }
      case 'failure': {
        const response = frame.message as RpcFailure
        this.#settle(response)
        return
      }
      case 'notification': {
        const note = frame.message as RpcNotification
        this.#contain(() => this.#options.onNotification?.(note.method, note.params ?? {}))
        return
      }
      case 'request': {
        void this.#answer(frame.message as RpcRequest)
        return
      }
    }
  }

  /**
   * Resolve or reject the pending request a response belongs to.
   *
   * @param response - the peer's response frame.
   */
  #settle(response: RpcResponse): void {
    const pending = this.#pending.get(response.id as RequestId)
    if (pending === undefined) return
    this.#pending.delete(response.id as RequestId)
    clearTimeout(pending.timer)
    if ('error' in response) {
      const code = codeFromRpc(response.error.code)
      pending.reject(
        new ChannelError(code, `${pending.method} failed: ${response.error.message}`, {
          rpcCode: response.error.code,
          data: response.error.data,
        }),
      )
      return
    }
    pending.resolve((response as { result: unknown }).result)
  }

  /**
   * Run a registered handler for an inbound request and answer it.
   *
   * @param request - the inbound request frame.
   */
  async #answer(request: RpcRequest): Promise<void> {
    const handler = this.#handlers.get(request.method)
    if (handler === undefined) {
      await this.#safeWrite(failure(request.id, ErrorCode.MethodNotFound, `unsupported method: ${request.method}`))
      return
    }
    try {
      const result = await handler(request.params ?? {}, request.method)
      await this.#safeWrite(success(request.id, result ?? {}))
    } catch (error) {
      const channelError = error instanceof ChannelError
        ? error
        : new ChannelError('unavailable', error instanceof Error ? error.message : String(error))
      await this.#safeWrite(failure(request.id, rpcCodeFor(channelError.code), channelError.message, channelError.detail))
    }
  }

  /**
   * Write a response, tolerating a peer that has already gone away.
   *
   * @param message - response message.
   */
  async #safeWrite(message: RpcMessage): Promise<void> {
    try {
      await this.#write(message)
    } catch {
      // Losing the ability to answer a request is already reported through the
      // close path; a second throw here would only mask the original reason.
    }
  }

  /**
   * Run best-effort owner code without letting it break frame handling.
   *
   * @param body - callback to run.
   */
  #contain(body: () => void): void {
    try {
      body()
    } catch {
      // Owner callbacks are observers; a throwing observer must not corrupt the
      // frame loop or leak into the Harness shared event chain.
    }
  }

  /**
   * Move the channel to a terminal state exactly once.
   *
   * @param reason - why the channel ended.
   */
  #fail(reason: ChannelError): void {
    if (this.#state === 'closed') return
    this.#state = 'closed'
    this.#closeReason = reason
    this.#decoder.reset()
    for (const [id, pending] of this.#pending) {
      clearTimeout(pending.timer)
      this.#pending.delete(id)
      pending.reject(reason)
    }
    this.#options.onClosed?.(reason)
  }
}

/** Default handlers every channel understands, independent of the session layer. */
const DEFAULT_HANDLERS: Record<string, RequestHandler> = {
  ping: () => ({ pong: Date.now() }),
  'diag/log': (params: unknown) => {
    void params
    return { accepted: true }
  },
}

/**
 * Map an inbound JSON-RPC error code back onto the channel taxonomy.
 *
 * @param code - numeric code from the peer.
 * @returns the closest {@link ChannelErrorCode}.
 */
function codeFromRpc(code: number): ChannelErrorCode {
  switch (code) {
    case ErrorCode.ProtocolMismatch:
      return 'protocol-mismatch'
    case ErrorCode.RequestTimeout:
      return 'request-timeout'
    case ErrorCode.ChannelClosed:
      return 'channel-closed'
    case ErrorCode.Unavailable:
      return 'unavailable'
    case ErrorCode.Stale:
      return 'stale'
    case ErrorCode.ProtocolViolation:
      return 'protocol-violation'
    default:
      return 'protocol-violation'
  }
}

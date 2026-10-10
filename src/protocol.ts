/**
 * Cross-process protocol: JSON-RPC 2.0 over NDJSON on the child's stdin/stdout.
 *
 * Wire rules (the Rust half mirrors them in `quorfloat/src/ipc/protocol.rs`):
 * - one UTF-8 JSON value per line; `\n` inside a JSON string is escaped by JSON
 *   itself, so a frame boundary is always a literal `0x0A` byte;
 * - the child's **stdout carries protocol frames only**; every log line goes to
 *   its stderr;
 * - reads are byte streams: one frame may arrive split across many chunks, and
 *   many frames may arrive inside one chunk.
 *
 * This module owns framing and message construction. It knows nothing about
 * sessions, windows, or which process is on the other end.
 */

/** Protocol version negotiated at handshake; independent of the Harness host protocol. */
export const PROTOCOL_VERSION = 'quorfloat/1'

/** Frame size ceiling; a longer line is a protocol violation, not a big message. */
export const MAX_FRAME_BYTES = 1024 * 1024

/** JSON-RPC 2.0 reserved codes, plus this project's own range. */
export const ErrorCode = {
  /** Malformed JSON, or a value that is not a JSON-RPC message. */
  ParseError: -32700,
  /** Well-formed JSON that is not a valid request object. */
  InvalidRequest: -32600,
  MethodNotFound: -32601,
  InvalidParams: -32602,
  InternalError: -32603,
  /** Peer speaks a protocol version we do not implement. */
  ProtocolMismatch: -32001,
  /** The request was not answered inside the configured timeout. */
  RequestTimeout: -32002,
  /** The channel closed, or the peer process exited, before an answer. */
  ChannelClosed: -32003,
  /** The operation is unavailable (service missing, no workspace, no owner). */
  Unavailable: -32004,
  /** The caller's view was stale: identity, ownership, or state no longer valid. */
  Stale: -32005,
  /** The peer reported success but the follow-up state contradicts it. */
  ProtocolViolation: -32006,
} as const

export type ErrorCodeValue = (typeof ErrorCode)[keyof typeof ErrorCode]

/** A JSON-RPC request id: we only ever generate integers. */
export type RequestId = number

export interface RpcRequest<P = unknown> {
  readonly jsonrpc: '2.0'
  readonly id: RequestId
  readonly method: string
  readonly params?: P
}

export interface RpcNotification<P = unknown> {
  readonly jsonrpc: '2.0'
  readonly method: string
  readonly params?: P
}

export interface RpcErrorObject {
  readonly code: number
  readonly message: string
  readonly data?: unknown
}

export interface RpcSuccess<R = unknown> {
  readonly jsonrpc: '2.0'
  readonly id: RequestId
  readonly result: R
}

export interface RpcFailure {
  readonly jsonrpc: '2.0'
  readonly id: RequestId | null
  readonly error: RpcErrorObject
}

export type RpcResponse<R = unknown> = RpcSuccess<R> | RpcFailure

export type RpcMessage<R = unknown> =
  | RpcRequest
  | RpcNotification
  | RpcSuccess<R>
  | RpcFailure

/** Build a request object. */
export function request<P>(id: RequestId, method: string, params?: P): RpcRequest<P> {
  return params === undefined
    ? { jsonrpc: '2.0', id, method }
    : { jsonrpc: '2.0', id, method, params }
}

/** Build a notification object (a request without an id). */
export function notification<P>(method: string, params?: P): RpcNotification<P> {
  return params === undefined ? { jsonrpc: '2.0', method } : { jsonrpc: '2.0', method, params }
}

/** Build a success response. */
export function success<R>(id: RequestId, result: R): RpcSuccess<R> {
  return { jsonrpc: '2.0', id, result }
}

/** Build a failure response. */
export function failure(id: RequestId | null, code: number, message: string, data?: unknown): RpcFailure {
  return data === undefined
    ? { jsonrpc: '2.0', id, error: { code, message } }
    : { jsonrpc: '2.0', id, error: { code, message, data } }
}

/** Message classification used by both sides of the channel. */
export type MessageKind = 'request' | 'notification' | 'success' | 'failure'

/** Why a frame was rejected; the labels are stable for diagnostics and tests. */
export type FrameRejection =
  | { readonly kind: 'invalid-json'; readonly detail: string }
  | { readonly kind: 'invalid-message'; readonly detail: string }
  | { readonly kind: 'oversized'; readonly bytes: number }

/** Classify a parsed JSON value, or explain why it is not a JSON-RPC message. */
export function classify(value: unknown): { kind: MessageKind; message: RpcMessage } | FrameRejection {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    return { kind: 'invalid-message', detail: 'frame is not a JSON object' }
  }
  const candidate = value as Record<string, unknown>
  if (candidate['jsonrpc'] !== '2.0') {
    return { kind: 'invalid-message', detail: 'jsonrpc must be the string "2.0"' }
  }
  const hasId = Object.hasOwn(candidate, 'id')
  const hasMethod = Object.hasOwn(candidate, 'method')
  const hasResult = Object.hasOwn(candidate, 'result')
  const hasError = Object.hasOwn(candidate, 'error')

  if (hasMethod) {
    if (typeof candidate['method'] !== 'string' || candidate['method'] === '') {
      return { kind: 'invalid-message', detail: 'method must be a non-empty string' }
    }
    if (hasId) {
      if (!Number.isInteger(candidate['id'])) {
        return { kind: 'invalid-message', detail: 'request id must be an integer' }
      }
      return { kind: 'request', message: candidate as unknown as RpcRequest }
    }
    return { kind: 'notification', message: candidate as unknown as RpcNotification }
  }

  if (hasResult && hasError) {
    return { kind: 'invalid-message', detail: 'response carries both result and error' }
  }
  if (hasError) {
    const error = candidate['error']
    if (typeof error !== 'object' || error === null || typeof (error as RpcErrorObject).code !== 'number') {
      return { kind: 'invalid-message', detail: 'error must carry a numeric code' }
    }
    const id = candidate['id']
    if (!(id === null || Number.isInteger(id))) {
      return { kind: 'invalid-message', detail: 'response id must be an integer or null' }
    }
    return { kind: 'failure', message: candidate as unknown as RpcFailure }
  }
  if (hasResult) {
    if (!Number.isInteger(candidate['id'])) {
      return { kind: 'invalid-message', detail: 'response id must be an integer' }
    }
    return { kind: 'success', message: candidate as unknown as RpcSuccess }
  }
  return { kind: 'invalid-message', detail: 'frame is neither a request, a notification, nor a response' }
}

/** One decoded frame: either a message or a rejection with a stable label. */
export type DecodedFrame =
  | { readonly ok: true; readonly kind: MessageKind; readonly message: RpcMessage; readonly bytes: number }
  | { readonly ok: false; readonly rejection: FrameRejection; readonly bytes: number }

/**
 * Encode one message as a protocol frame (a JSON line, `\n` terminated).
 *
 * @param message - the JSON-RPC message to send.
 * @returns the UTF-8 frame, newline included.
 */
export function encodeFrame(message: RpcMessage): string {
  return `${JSON.stringify(message)}\n`
}

/**
 * Incremental NDJSON decoder.
 *
 * Bytes are buffered until a `0x0A` arrives, so a frame split across chunks is
 * reassembled and several frames in one chunk are emitted separately. A frame
 * that exceeds {@link MAX_FRAME_BYTES} before its terminator, or that is not
 * valid UTF-8, is reported and dropped; the decoder never grows without bound
 * and never throws at the caller.
 */
export class FrameDecoder {
  readonly #maxBytes: number
  #buffer: Buffer = Buffer.alloc(0)

  /**
   * @param maxBytes - per-frame ceiling in bytes; defaults to {@link MAX_FRAME_BYTES}.
   */
  constructor(maxBytes: number = MAX_FRAME_BYTES) {
    this.#maxBytes = maxBytes
  }

  /** Bytes currently buffered; useful for diagnostics and tests. */
  get bufferedBytes(): number {
    return this.#buffer.length
  }

  /**
   * Feed one chunk and collect every frame it completes.
   *
   * @param chunk - bytes as read from the pipe.
   * @returns decoded frames in arrival order.
   */
  push(chunk: Buffer | string): DecodedFrame[] {
    const bytes = typeof chunk === 'string' ? Buffer.from(chunk, 'utf8') : chunk
    this.#buffer = this.#buffer.length === 0 ? bytes : Buffer.concat([this.#buffer, bytes])
    const frames: DecodedFrame[] = []
    for (;;) {
      const newline = this.#buffer.indexOf(0x0a)
      if (newline === -1) {
        if (this.#buffer.length > this.#maxBytes) {
          frames.push({
            ok: false,
            rejection: { kind: 'oversized', bytes: this.#buffer.length },
            bytes: this.#buffer.length,
          })
          this.#buffer = Buffer.alloc(0)
        }
        return frames
      }
      const line = this.#buffer.subarray(0, newline)
      this.#buffer = this.#buffer.subarray(newline + 1)
      if (line.length > this.#maxBytes) {
        frames.push({ ok: false, rejection: { kind: 'oversized', bytes: line.length }, bytes: line.length })
        continue
      }
      frames.push(this.#decodeLine(line))
    }
  }

  /** Drop any partial frame; used when a channel is abandoned. */
  reset(): void {
    this.#buffer = Buffer.alloc(0)
  }

  /**
   * Decode one complete line.
   *
   * @param line - bytes between two newlines, terminator excluded.
   * @returns the decoded frame or its rejection.
   */
  #decodeLine(line: Buffer): DecodedFrame {
    // A trailing CR makes the frame a valid NDJSON line for peers that emit
    // CRLF; it is not part of the JSON payload.
    const trimmed = line.length > 0 && line[line.length - 1] === 0x0d ? line.subarray(0, line.length - 1) : line
    let text: string
    try {
      text = new TextDecoder('utf-8', { fatal: true }).decode(trimmed)
    } catch {
      return { ok: false, rejection: { kind: 'invalid-json', detail: 'frame is not valid UTF-8' }, bytes: line.length }
    }
    if (text.trim() === '') {
      return { ok: false, rejection: { kind: 'invalid-message', detail: 'empty frame' }, bytes: line.length }
    }
    let parsed: unknown
    try {
      parsed = JSON.parse(text)
    } catch (error) {
      return {
        ok: false,
        rejection: { kind: 'invalid-json', detail: error instanceof Error ? error.message : String(error) },
        bytes: line.length,
      }
    }
    const classified = classify(parsed)
    if (!('message' in classified)) {
      return { ok: false, rejection: classified, bytes: line.length }
    }
    return { ok: true, kind: classified.kind, message: classified.message, bytes: line.length }
  }
}

/** Narrow a decoded request frame. */
export function asRequest(frame: DecodedFrame): RpcRequest | undefined {
  return frame.ok && frame.kind === 'request' ? (frame.message as RpcRequest) : undefined
}

/** Narrow a decoded notification frame. */
export function asNotification(frame: DecodedFrame): RpcNotification | undefined {
  return frame.ok && frame.kind === 'notification' ? (frame.message as RpcNotification) : undefined
}

/** Narrow a decoded response frame. */
export function asResponse(frame: DecodedFrame): RpcResponse | undefined {
  return frame.ok && (frame.kind === 'success' || frame.kind === 'failure')
    ? (frame.message as RpcResponse)
    : undefined
}

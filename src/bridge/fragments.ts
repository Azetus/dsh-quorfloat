// Fragmentation for JSON-RPC messages larger than one NDJSON frame.
//
// The fragment envelope is transport-only: a peer reassembles the original JSON text
// before classifying or dispatching it. That keeps record shape, sequence numbers and
// cursors untouched, and it works for requests, responses, notifications and prompts
// alike.

import {
  FRAGMENT_METHOD,
  FRAGMENT_RAW_BYTES,
  FRAGMENT_TIMEOUT_MS,
  MAX_LOGICAL_MESSAGE_BYTES,
  classify,
  type DecodedFrame,
  type FrameRejection,
  type RpcMessage,
  type RpcNotification,
} from '../protocol.js'

/** One fragment message, as sent on the wire. */
export interface FragmentParams {
  readonly id: string
  readonly index: number
  readonly count: number
  readonly data: string
}

/** Largest number of fragments one message may occupy. */
const MAX_FRAGMENTS = Math.ceil(MAX_LOGICAL_MESSAGE_BYTES / FRAGMENT_RAW_BYTES)

/**
 * Split one JSON-RPC message into transport fragments.
 *
 * @param message - the logical message.
 * @param id - per-sequence identity; unique within the channel lifetime.
 * @returns the fragment notifications in order.
 */
export function fragmentMessage(message: RpcMessage, id: string): RpcNotification<FragmentParams>[] {
  const json = JSON.stringify(message)
  const bytes = Buffer.from(json, 'utf8')
  if (bytes.length > MAX_LOGICAL_MESSAGE_BYTES) {
    throw new Error(`message exceeds ${String(MAX_LOGICAL_MESSAGE_BYTES)} bytes`)
  }
  const count = Math.max(1, Math.ceil(bytes.length / FRAGMENT_RAW_BYTES))
  const out: RpcNotification<FragmentParams>[] = []
  for (let index = 0; index < count; index += 1) {
    const start = index * FRAGMENT_RAW_BYTES
    const end = Math.min(bytes.length, start + FRAGMENT_RAW_BYTES)
    out.push({
      jsonrpc: '2.0',
      method: FRAGMENT_METHOD,
      params: { id, index, count, data: bytes.subarray(start, end).toString('base64') },
    })
  }
  return out
}

/** What accepting one fragment produced. */
export type FragmentOutcome =
  | { readonly kind: 'pending' }
  | { readonly kind: 'rejected'; readonly rejection: FrameRejection }
  | { readonly kind: 'complete'; readonly frame: DecodedFrame }

interface Pending {
  readonly count: number
  readonly chunks: (Buffer | undefined)[]
  bytes: number
  readonly timer: NodeJS.Timeout
}

/**
 * Reassemble one fragmented JSON-RPC message.
 *
 * Writes are serialized by the channel, so at most one sequence is active per direction.
 * A sequence that does not finish within the timeout is rejected and discarded.
 */
export class FragmentAssembler {
  readonly #onTimeout: (rejection: FrameRejection) => void
  #pending: { readonly id: string; readonly state: Pending } | null = null

  /**
   * @param onTimeout - called when an unfinished sequence expires.
   */
  constructor(onTimeout: (rejection: FrameRejection) => void = () => {}) {
    this.#onTimeout = onTimeout
  }

  /** Drop any incomplete sequence without reporting it. */
  reset(): void {
    if (this.#pending !== null) clearTimeout(this.#pending.state.timer)
    this.#pending = null
  }

  /**
   * Feed one fragment envelope.
   *
   * @param params - the notification parameters.
   * @returns pending, a rejection, or the reassembled frame.
   */
  accept(params: unknown): FragmentOutcome {
    const fragment = this.#read(params)
    if (fragment === null) return { kind: 'rejected', rejection: { kind: 'invalid-message', detail: 'invalid fragment parameters' } }
    if (this.#pending !== null && this.#pending.id !== fragment.id) {
      this.reset()
    }
    if (this.#pending === null) {
      if (fragment.index !== 0) return { kind: 'rejected', rejection: { kind: 'invalid-message', detail: 'fragment sequence did not start at zero' } }
      if (fragment.count < 1 || fragment.count > MAX_FRAGMENTS) {
        return { kind: 'rejected', rejection: { kind: 'invalid-message', detail: 'fragment count is out of range' } }
      }
      const timer = setTimeout(() => {
        this.#pending = null
        this.#onTimeout({ kind: 'invalid-message', detail: 'fragment sequence timed out' })
      }, FRAGMENT_TIMEOUT_MS)
      timer.unref?.()
      this.#pending = {
        id: fragment.id,
        state: { count: fragment.count, chunks: new Array<Buffer | undefined>(fragment.count).fill(undefined), bytes: 0, timer },
      }
    }
    const pending = this.#pending
    if (pending === null) return { kind: 'rejected', rejection: { kind: 'invalid-message', detail: 'fragment state disappeared' } }
    if (fragment.count !== pending.state.count) {
      this.reset()
      return { kind: 'rejected', rejection: { kind: 'invalid-message', detail: 'fragment count changed mid-sequence' } }
    }
    if (fragment.index < 0 || fragment.index >= pending.state.count || pending.state.chunks[fragment.index] !== undefined) {
      this.reset()
      return { kind: 'rejected', rejection: { kind: 'invalid-message', detail: 'fragment index is duplicate or out of range' } }
    }
    const chunk = Buffer.from(fragment.data, 'base64')
    pending.state.bytes += chunk.length
    if (pending.state.bytes > MAX_LOGICAL_MESSAGE_BYTES) {
      this.reset()
      return { kind: 'rejected', rejection: { kind: 'invalid-message', detail: 'fragment sequence exceeds the logical message limit' } }
    }
    pending.state.chunks[fragment.index] = chunk
    if (pending.state.chunks.some(part => part === undefined)) return { kind: 'pending' }
    clearTimeout(pending.state.timer)
    const bytes = Buffer.concat(pending.state.chunks as Buffer[])
    this.#pending = null
    let parsed: unknown
    try {
      parsed = JSON.parse(bytes.toString('utf8'))
    } catch {
      return { kind: 'rejected', rejection: { kind: 'invalid-json', detail: 'reassembled frame is not JSON' } }
    }
    const classified = classify(parsed)
    if (!('message' in classified)) return { kind: 'rejected', rejection: classified }
    return {
      kind: 'complete',
      frame: { ok: true, kind: classified.kind, message: classified.message, bytes: bytes.length },
    }
  }

  /** Validate one fragment envelope. */
  #read(params: unknown): FragmentParams | null {
    if (typeof params !== 'object' || params === null || Array.isArray(params)) return null
    const value = params as Record<string, unknown>
    const id = value['id']
    const index = value['index']
    const count = value['count']
    const data = value['data']
    if (typeof id !== 'string' || id === '') return null
    if (!Number.isInteger(index) || !Number.isInteger(count)) return null
    if (typeof data !== 'string') return null
    return { id, index: index as number, count: count as number, data }
  }
}

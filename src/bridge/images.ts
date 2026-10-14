/** Temporary bounded image snapshots; each response stays below the wire frame ceiling. */
import { randomUUID } from 'node:crypto'
import { ChannelError } from './errors.js'

export const MAX_IMAGE_BYTES = 8 * 1024 * 1024
const CHUNK_BYTES = 256 * 1024
const MAX_TRANSFERS = 2
const TRANSFER_TTL_MS = 30_000

export interface LocalImage { readonly data: Uint8Array; readonly mimeType: string }
type Transfer = LocalImage & { sessionId: string; path: string; timer: ReturnType<typeof setTimeout> }

/** One store per channel generation, with at most two reads/snapshots retained. */
export class ImageChunks {
  readonly #transfers = new Map<string, Transfer>()
  #reading = 0

  async read(params: unknown, reader: (sessionId: string, path: string, signal: AbortSignal) => Promise<LocalImage>): Promise<unknown> {
    const input = params as Record<string, unknown> | null
    const sessionId = input?.['sessionId']
    const path = input?.['path']
    if (typeof sessionId !== 'string' || !sessionId || typeof path !== 'string' || !path) {
      throw new ChannelError('unavailable', 'image requires sessionId and path', { reason: 'path' })
    }
    let token = input?.['token']
    if (input?.['release'] === true) {
      if (typeof token === 'string' && this.#transfers.get(token)?.sessionId === sessionId) this.#release(token)
      return { released: true }
    }
    const offset = input?.['offset'] ?? 0
    if (typeof offset !== 'number' || !Number.isSafeInteger(offset) || offset < 0) {
      throw new ChannelError('unavailable', 'invalid image offset', { reason: 'path' })
    }
    if (token === undefined || token === null) {
      if (offset !== 0 || this.#reading + this.#transfers.size >= MAX_TRANSFERS) {
        throw new ChannelError('unavailable', 'image transfer unavailable', { reason: 'busy' })
      }
      this.#reading += 1
      try {
        const image = await reader(sessionId, path, AbortSignal.timeout(10_000))
        if (image.data.byteLength > MAX_IMAGE_BYTES) throw new ChannelError('unavailable', 'image exceeds byte limit', { reason: 'tooLarge' })
        const identity = randomUUID()
        token = identity
        const timer = setTimeout(() => { this.#release(identity) }, TRANSFER_TTL_MS)
        timer.unref()
        this.#transfers.set(identity, { ...image, sessionId, path, timer })
      } finally { this.#reading -= 1 }
    }
    const transfer = typeof token === 'string' ? this.#transfers.get(token) : undefined
    if (!transfer || transfer.sessionId !== sessionId || transfer.path !== path || offset >= transfer.data.byteLength) {
      throw new ChannelError('unavailable', 'image transfer expired or mismatched', { reason: 'readFailed' })
    }
    const end = Math.min(offset + CHUNK_BYTES, transfer.data.byteLength)
    const response = { token, offset, nextOffset: end, totalBytes: transfer.data.byteLength,
      mimeType: transfer.mimeType, data: Buffer.from(transfer.data.subarray(offset, end)).toString('base64'),
      done: end === transfer.data.byteLength }
    if (response.done) this.#release(token as string)
    return response
  }

  #release(token: string): void {
    const transfer = this.#transfers.get(token)
    if (transfer) clearTimeout(transfer.timer)
    this.#transfers.delete(token)
  }
}

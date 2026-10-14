/** Bounded image reads through the filesystem composed by the Harness host. */
import { ChannelError } from '../bridge/errors.js'
import { MAX_IMAGE_BYTES, type LocalImage } from '../bridge/images.js'

interface FileSystemLike {
  resolve(path: string, options: { cwd?: string; signal: AbortSignal }): Promise<unknown>
  readBytes(target: unknown, signal: AbortSignal, maxBytes: number): Promise<Uint8Array>
}

/** Paths arrive decoded once by the Markdown frontend, never as file URLs. */
export function validateImagePath(path: string): void {
  if (!path || /[\u0000-\u001f\u007f]/u.test(path) || /^[\\/]{2}/u.test(path)
    || path.startsWith('~') || (/^[a-z][a-z\d+.-]*:/iu.test(path) && !/^[a-z]:[\\/]/iu.test(path))) {
    throw new ChannelError('unavailable', 'unsupported image path', { reason: 'path' })
  }
}

function mediaType(bytes: Uint8Array): string | undefined {
  const header = Buffer.from(bytes.subarray(0, 12))
  if (header.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]))) return 'image/png'
  if (header[0] === 255 && header[1] === 216 && header[2] === 255) return 'image/jpeg'
  if (['GIF87a', 'GIF89a'].includes(header.subarray(0, 6).toString('ascii'))) return 'image/gif'
  if (header.subarray(0, 4).toString('ascii') === 'RIFF' && header.subarray(8, 12).toString('ascii') === 'WEBP') return 'image/webp'
  return undefined
}

/** Resolve in the host's execution world; absolute paths may lie outside cwd. */
export async function readLocalImage(
  get: (name: string) => unknown, path: string, cwd: string | undefined, signal: AbortSignal,
): Promise<LocalImage> {
  validateImagePath(path)
  const absolute = path.startsWith('/') || /^[a-z]:[\\/]/iu.test(path)
  if (!absolute && !cwd) throw new ChannelError('unavailable', 'session has no cwd', { reason: 'cwd' })
  const fs = get('fs') as FileSystemLike | undefined
  if (!fs?.resolve || !fs.readBytes) throw new ChannelError('unavailable', 'filesystem service unavailable', { reason: 'unavailable' })
  const attachments = get('attachments') as { imageLimits?: { maxImageBytes?: number } } | undefined
  const limit = attachments?.imageLimits?.maxImageBytes
  const maxBytes = typeof limit === 'number' && Number.isSafeInteger(limit) && limit > 0
    ? Math.min(limit, MAX_IMAGE_BYTES) : MAX_IMAGE_BYTES
  try {
    const target = await fs.resolve(path, cwd === undefined ? { signal } : { cwd, signal })
    const data = await fs.readBytes(target, signal, maxBytes)
    if (data.byteLength > maxBytes) throw new ChannelError('unavailable', 'image exceeds byte limit', { reason: 'tooLarge' })
    const mimeType = mediaType(data)
    if (!mimeType) throw new ChannelError('unavailable', 'unsupported image content', { reason: 'format' })
    return { data, mimeType }
  } catch (error) {
    if (error instanceof ChannelError) throw error
    const code = (error as { code?: unknown } | null)?.code
    const reason = code === 'FS_NOT_FOUND' ? 'missing' : code === 'FS_TOO_LARGE' ? 'tooLarge' : 'readFailed'
    throw new ChannelError('unavailable', 'local image read failed', { reason })
  }
}

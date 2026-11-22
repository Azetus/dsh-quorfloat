// Display-only previews for structured payloads.
//
// The snapshot keeps the full value; this helper only decides how much of it to paint.
// A preview is never written back to the wire.

/** Characters shown before a structured payload is collapsed. */
export const JSON_PREVIEW_CHARS = 20_000

/** One display preview and the size of the full value. */
export interface TextPreview {
  /** The prefix to draw, or the full text when it fits. */
  readonly text: string
  /** Full length in code points. */
  readonly total: number
  /** Whether anything was left out of `text`. */
  readonly truncated: boolean
}

/**
 * Bound a structured payload for display.
 *
 * @param text - the full payload.
 * @param limit - maximum code points to draw.
 * @returns the prefix, its full length, and whether it was shortened.
 */
export function previewText(text: string, limit = JSON_PREVIEW_CHARS): TextPreview {
  const chars = Array.from(text)
  if (chars.length <= limit) return { text, total: chars.length, truncated: false }
  return { text: chars.slice(0, limit).join(''), total: chars.length, truncated: true }
}

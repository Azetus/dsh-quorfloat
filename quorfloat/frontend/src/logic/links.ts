/** Absolute web addresses that may be handed to the system browser. */
export function externalWebUrl(value: string): string | null {
  try {
    const url = new URL(value)
    return (url.protocol === 'https:' || url.protocol === 'http:') && url.hostname !== ''
      ? url.href
      : null
  } catch {
    return null
  }
}

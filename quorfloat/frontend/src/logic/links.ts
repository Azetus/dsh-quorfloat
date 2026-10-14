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

/** Decode an authored local destination once; file URLs and network paths stay inert. */
export function localImagePath(value: string): string | null {
  if (value.includes('?') || value.includes('#')) return null
  try {
    const path = decodeURIComponent(value)
    if (!path || /[\u0000-\u001f\u007f]/u.test(path) || /^[\\/]{2}/u.test(path)
      || path.startsWith('~') || (/^[a-z][a-z\d+.-]*:/iu.test(path) && !/^[a-z]:[\\/]/iu.test(path))) return null
    return path
  } catch { return null }
}

// How much room the native window currently offers the panel.
//
// It is the panel's clamp as well as its style: the cap is the product limit
// (`settings.maxHeight`), but the window it has to live in may be smaller, and a viewport
// can change under the webview without a snapshot (a display change, the user resizing).

import { useEffect, useState } from 'react'

/**
 * Track the window's inner height.
 *
 * @returns the current inner height in CSS pixels.
 */
export function useViewportHeight(): number {
  const [height, setHeight] = useState(() => window.innerHeight)
  useEffect(() => {
    const onResize = (): void => { setHeight(window.innerHeight) }
    window.addEventListener('resize', onResize)
    return () => { window.removeEventListener('resize', onResize) }
  }, [])
  return height
}

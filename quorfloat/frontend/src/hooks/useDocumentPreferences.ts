// The two preferences the document itself carries: the palette (`light-dark()` follows
// `color-scheme`) and reduced motion (the `q-reduce-motion` class).

import { useEffect } from 'react'
import { themeColorScheme } from '../lib/settings'
import type { Snapshot } from '../lib/state'

/**
 * Apply the host's appearance preferences to `<html>`.
 *
 * One property switches both palettes, and one class switches every transition off — the
 * same two knobs the old renderer set on every render.
 *
 * @param settings - the snapshot's settings.
 */
export function useDocumentPreferences(settings: Snapshot['settings']): void {
  useEffect(() => {
    document.documentElement.style.colorScheme = themeColorScheme(settings.theme)
    document.documentElement.classList.toggle('q-reduce-motion', settings.reduceMotion)
  }, [settings.theme, settings.reduceMotion])
}

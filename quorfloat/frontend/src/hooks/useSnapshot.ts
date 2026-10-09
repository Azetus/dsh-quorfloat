// The shell's snapshots in, one piece of React state out.
//
// The shell is the authority: every snapshot replaces the last one wholesale, and nothing
// here merges or edits it. That is what makes "the panel shows what the host said" true by
// construction rather than by discipline.

import { useEffect, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import type { Snapshot } from '../lib/state'

/**
 * Subscribe to `quorfloat/state`.
 *
 * @returns the latest snapshot, or null before the first one arrives.
 */
export function useSnapshot(): Snapshot | null {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null)
  useEffect(() => {
    const pending = listen<Snapshot>('quorfloat/state', event => { setSnapshot(event.payload) })
    return () => { void pending.then(unlisten => { unlisten() }) }
  }, [])
  return snapshot
}

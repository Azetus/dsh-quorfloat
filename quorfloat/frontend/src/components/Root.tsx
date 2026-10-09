// The page's root: the shell's snapshots in, the panel out — and nothing at all before the
// first snapshot arrives (the native window is transparent until then).

import { useSnapshot } from '../hooks/useSnapshot'
import { App } from './App'

/**
 * Subscribe to the shell and render the panel.
 *
 * @returns the panel, or an empty stage before the first snapshot.
 */
export function Root() {
  const state = useSnapshot()
  if (state === null) return <div className="q-stage" />
  return <App state={state} />
}

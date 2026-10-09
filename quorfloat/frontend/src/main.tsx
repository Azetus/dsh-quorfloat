// The page's entry: mount React, and say so when there is no shell to talk to.
//
// The shell's snapshots and the panel's own events are the whole contract (see
// `components/App.tsx` and `hooks/`); this file exists to give the tree a document.

import { createRoot } from 'react-dom/client'
import { api } from './api'
import { Root } from './components/Root'

const host = document.getElementById('root')
if (host === null) throw new Error('#root is missing from the document')

createRoot(host).render(<Root />)

// When the page runs in a plain browser (no Tauri), `invoke` has nothing to reach; the log
// is a diagnostic for that case and nothing more.
if (!('__TAURI__' in window)) {
  void api.log('frontend running without the shell').catch(() => {})
}

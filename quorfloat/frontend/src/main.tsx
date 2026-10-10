// The page's entry: mount React, and say so when there is no shell to talk to.
//
// The shell's snapshots and the panel's own events are the whole contract (see
// `components/App.tsx` and `hooks/`); this file exists to give the tree a document.

import { createRoot } from 'react-dom/client'
import { api } from './api'
import { Root } from './components/Root'
import { reportedLanguages } from './lib/languages'

const host = document.getElementById('root')
if (host === null) throw new Error('#root is missing from the document')

// The shell's only source for "nobody has chosen a language yet": `navigator.languages`
// is a fact of this webview, so it has to leave from here. Sent once, and before the
// first render, because the first launch's language is seeded from it; a shell that is
// not there, or an invoke that is refused, is caught and simply leaves the report
// unmade — the shell then keeps the `en` it started with.
void api.reportLanguages(reportedLanguages()).catch(() => {})

createRoot(host).render(<Root />)

// When the page runs in a plain browser (no Tauri), `invoke` has nothing to reach; the log
// is a diagnostic for that case and nothing more.
if (!('__TAURI__' in window)) {
  void api.log('frontend running without the shell').catch(() => {})
}

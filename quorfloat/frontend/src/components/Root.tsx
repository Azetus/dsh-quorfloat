// The page's root: the shell's snapshots in, the panel out — and nothing at all before the
// first snapshot arrives (the native window is transparent until then).

import { useSnapshot } from '../hooks/useSnapshot'
import { languageOf } from '../lib/i18n'
import { App } from './App'
import { I18nProvider } from './common/I18nProvider'

/**
 * Subscribe to the shell and render the panel.
 *
 * The snapshot's `settings.language` is put in force here, above everything that draws a
 * word; `languageOf` reads an absent or unknown value as `'zh'`.
 *
 * @returns the panel, or an empty stage before the first snapshot.
 */
export function Root() {
  const state = useSnapshot()
  if (state === null) return <div className="q-stage" />
  return (
    <I18nProvider language={languageOf(state.settings.language)}>
      <App state={state} />
    </I18nProvider>
  )
}

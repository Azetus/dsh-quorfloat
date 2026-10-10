// The React binding of the dictionary: which language is in force, and the `t` a
// component renders with.
//
// The language is context rather than a prop because it is ambient — every leaf that
// draws a word needs it, and threading it through twenty components would make the
// language a parameter of the layout. The default is `'zh'`, so a component rendered
// outside the provider (a test, or a page whose snapshot has not arrived) still draws
// the current copy instead of throwing.

import { createContext, useCallback, useContext } from 'react'
import {
  resolveText, translate, type Language, type MessageKey, type Params, type Text,
} from '../lib/i18n'

/** The language in force. `'zh'` until a snapshot says otherwise. */
export const I18nContext = createContext<Language>('zh')

/** The language in force, for code that needs the id rather than the words. */
export function useLanguage(): Language {
  return useContext(I18nContext)
}

/** Resolve a key or a {@link Text} in the language in force. */
export type Translate = (text: MessageKey | Text, params?: Params) => string

/**
 * The `t` every component renders through.
 *
 * A plain string is a dictionary key; a {@link Text} passes its raw data through and
 * looks its key up. The returned function is stable for as long as the language is, so a
 * component may put it in a dependency array.
 *
 * @returns the translate function.
 */
export function useT(): Translate {
  const language = useLanguage()
  return useCallback<Translate>(
    (text, params) => (typeof text === 'string'
      ? translate(language, text, params)
      : resolveText(language, text)),
    [language],
  )
}

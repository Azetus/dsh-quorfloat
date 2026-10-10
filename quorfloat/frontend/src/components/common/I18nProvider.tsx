// Puts the snapshot's language in force for the tree below.

import type { ReactNode } from 'react'
import { I18nContext } from '../../hooks/useI18n'
import type { Language } from '../../lib/i18n'

/** Props for {@link I18nProvider}. */
export interface I18nProviderProps {
  /** The language every descendant renders in. */
  readonly language: Language
  /** The tree that renders words. */
  readonly children: ReactNode
}

/**
 * Render the language context.
 *
 * @param props - the language and the tree.
 * @returns the provider.
 */
export function I18nProvider({ language, children }: I18nProviderProps) {
  return <I18nContext.Provider value={language}>{children}</I18nContext.Provider>
}

// The language picker's menu: the two values the wire field carries, each named in its own
// language.
//
// Mirrors `PermissionMenu`: a heading, then one `MenuOption` per choice, with the one in
// force marked selected (the design's `aria-pressed` + check glyph). The labels are the
// dictionary's endonyms — `language.zh` / `language.en` are deliberately identical in both
// dictionaries, because a reader who cannot read the current language still has to
// recognise 中文 (or English) in the list.

import { api } from '../../api'
import { useT } from '../../hooks/useI18n'
import type { Language, MessageKey } from '../../lib/i18n'
import { MenuHeading } from './MenuHeading'
import { MenuOption } from './MenuOption'

/** Props for {@link LanguageMenu}. */
export interface LanguageMenuProps {
  /** The language in force, as the snapshot reports it. */
  readonly language: Language
  /** Close the menu after a pick. */
  readonly onClose: () => void
}

const CHOICES: readonly { readonly value: Language; readonly label: MessageKey }[] = [
  { value: 'zh', label: 'language.zh' },
  { value: 'en', label: 'language.en' },
]

/**
 * Render the language choices.
 *
 * The write goes through the same path the theme chips use — `set_preferences`, validated
 * and stored by the shell; the panel keeps no copy of a setting the host owns.
 *
 * @param props - the language in force and the close handler.
 * @returns the menu contents.
 */
export function LanguageMenu({ language, onClose }: LanguageMenuProps) {
  const t = useT()
  return (
    <>
      <MenuHeading title={t('settings.language')} />
      {CHOICES.map(choice => (
        <MenuOption
          key={choice.value}
          label={t(choice.label)}
          selected={choice.value === language}
          onPick={() => {
            onClose()
            void api.setPreferences({ language: choice.value })
          }}
        />
      ))}
    </>
  )
}

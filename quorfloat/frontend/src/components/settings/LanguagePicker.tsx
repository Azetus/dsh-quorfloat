// The settings row's language control: a compact picker — current value plus a caret —
// that opens the two-language list.
//
// Mirrors `ModelPicker` / `PermissionPicker` / `WorkspacePicker`: the same `PickerPopover`
// shell (the design's `.q-picker` trigger, `.q-popover` card, click-outside and Escape
// rules) and the same `MenuOption` rows, so it is the panel's own vocabulary rather than a
// new control. `q-right` is the existing class that anchors the popover to the wrapper's
// right edge, which is where the settings row sits.
//
// A native `<select>` is not usable here: its option list is drawn by the platform
// and cannot take `--q-bg` / `--q-line` / `--q-soft`, the `MenuOption` check glyph, or the
// `aria-pressed` current item, so matching the panel would mean either new CSS the design
// does not define or a visibly foreign widget.

import { useT } from '../../hooks/useI18n'
import type { Language } from '../../lib/i18n'
import { PickerPopover } from '../chrome/PickerPopover'
import { Icon } from '../common/Icon'
import { LanguageMenu } from '../menus/LanguageMenu'

/** Props for {@link LanguagePicker}. */
export interface LanguagePickerProps {
  /** The language in force, as the snapshot reports it. */
  readonly language: Language
  /** Whether this popover is the open one (the app owns that state, like every picker). */
  readonly open: boolean
  /** Toggle request from the trigger. */
  readonly onToggle: () => void
  /** Close the popover. */
  readonly onClose: () => void
}

/**
 * Render the language picker.
 *
 * @param props - the language in force and the popover handlers.
 * @returns the picker.
 */
export function LanguagePicker({ language, open, onToggle, onClose }: LanguagePickerProps) {
  const t = useT()
  return (
    <PickerPopover
      id="q-language"
      menuId="q-language-menu"
      label={t('settings.language')}
      menuLabel={t('settings.language')}
      menuClass="q-right"
      open={open}
      onToggle={onToggle}
      onClose={onClose}
      trigger={
        <>
          <span id="q-language-name">{t(language === 'en' ? 'language.en' : 'language.zh')}</span>
          <span id="q-language-chevron" className="q-chevron"><Icon name="chevron-down" /></span>
        </>
      }
    >
      <LanguageMenu language={language} onClose={onClose} />
    </PickerPopover>
  )
}

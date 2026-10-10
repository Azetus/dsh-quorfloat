// The settings page: appearance, language, the summon shortcut, and the keep-open switch.

import { useT } from '../../hooks/useI18n'
import { languageOf } from '../../lib/i18n'
import type { Snapshot } from '../../lib/state'
import { HotkeyField } from './HotkeyField'
import { KeepOpenSwitch } from './KeepOpenSwitch'
import { LanguagePicker } from './LanguagePicker'
import { ThemeChips } from './ThemeChips'

/** Props for {@link SettingsPage}. */
export interface SettingsPageProps {
  /** The snapshot. */
  readonly state: Snapshot
  /** Whether the conversation page is up instead (this page is hidden, not unmounted). */
  readonly hidden: boolean
  /** Whether the shortcut field is waiting for a chord. */
  readonly recording: boolean
  /** Begin recording a chord. */
  readonly onRecord: () => void
  /** Return to the conversation. */
  readonly onBack: () => void
  /** Which popover is open, if any. */
  readonly openMenu: string | null
  /** Toggle one popover by trigger id. */
  readonly onToggleMenu: (id: string) => void
  /** Close whichever popover is open. */
  readonly onCloseMenu: () => void
}

/**
 * Render the settings page.
 *
 * It *replaces* the conversation rather than floating over it (the design's own choice),
 * and every control applies immediately and survives a restart. The language picker's open
 * state lives with the app's other popovers, so one Escape rule serves them all.
 *
 * @param props - snapshot, recording state, and handlers.
 * @returns the page.
 */
export function SettingsPage({
  state, hidden, recording, onRecord, onBack, openMenu, onToggleMenu, onCloseMenu,
}: SettingsPageProps) {
  const t = useT()
  return (
    <div id="q-settings" className="q-settings" hidden={hidden}>
      <div className="q-settings-title">
        <span>{t('settings.title')}</span>
        <button id="q-settings-back" type="button" onClick={onBack}>{t('settings.back')}</button>
      </div>
      <div className="q-setting">
        <span>{t('settings.appearance')}<small>{t('settings.appearanceDetail')}</small></span>
        <ThemeChips theme={state.settings.theme} />
      </div>
      <div className="q-setting">
        <span>{t('settings.language')}<small>{t('settings.languageDetail')}</small></span>
        <LanguagePicker
          language={languageOf(state.settings.language)}
          open={openMenu === 'q-language'}
          onToggle={() => { onToggleMenu('q-language') }}
          onClose={onCloseMenu}
        />
      </div>
      <div className="q-setting">
        <label htmlFor="q-shortcut">{t('settings.hotkey')}<small>{t('settings.hotkeyDetail')}</small></label>
        <HotkeyField
          requested={state.hotkey.requested}
          reason={state.hotkey.reason}
          recording={recording}
          onRecord={onRecord}
        />
      </div>
      <div className="q-setting">
        <span>{t('settings.keepOpen')}<small>{t('settings.keepOpenDetail')}</small></span>
        <KeepOpenSwitch hideOnBlur={state.settings.hideOnBlur} />
      </div>
    </div>
  )
}

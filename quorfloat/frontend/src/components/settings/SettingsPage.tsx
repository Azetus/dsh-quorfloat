// The settings page: appearance, the summon shortcut, and the keep-open switch.

import type { Snapshot } from '../../lib/state'
import { HotkeyField } from './HotkeyField'
import { KeepOpenSwitch } from './KeepOpenSwitch'
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
}

/**
 * Render the settings page.
 *
 * It *replaces* the conversation rather than floating over it (the design's own choice),
 * and every control applies immediately and survives a restart.
 *
 * @param props - snapshot, recording state, and handlers.
 * @returns the page.
 */
export function SettingsPage({ state, hidden, recording, onRecord, onBack }: SettingsPageProps) {
  return (
    <div id="q-settings" className="q-settings" hidden={hidden}>
      <div className="q-settings-title">
        <span>悬浮窗设置</span>
        <button id="q-settings-back" type="button" onClick={onBack}>返回对话</button>
      </div>
      <div className="q-setting">
        <span>外观<small>明暗主题跟随系统或手动指定</small></span>
        <ThemeChips theme={state.settings.theme} />
      </div>
      <div className="q-setting">
        <label htmlFor="q-shortcut">呼出快捷键<small>点击后按下组合键</small></label>
        <HotkeyField
          requested={state.hotkey.requested}
          reason={state.hotkey.reason}
          recording={recording}
          onRecord={onRecord}
        />
      </div>
      <div className="q-setting">
        <span>失焦时保持展开<small>切换应用后仍保留悬浮窗</small></span>
        <KeepOpenSwitch hideOnBlur={state.settings.hideOnBlur} />
      </div>
    </div>
  )
}

// The panel's top bar: brand, the two context pickers, and the window tools.

import type { Snapshot } from '../../lib/state'
import { Icon } from '../common/Icon'
import { SessionPicker } from './SessionPicker'
import { WorkspacePicker } from './WorkspacePicker'

/** Props for {@link TopBar}. */
export interface TopBarProps {
  /** The snapshot. */
  readonly state: Snapshot
  /** The workspace the next conversation would use (the user's pick, or the shell's). */
  readonly targetWorkspace: string | null
  /** Which popover is open, if any. */
  readonly openMenu: string | null
  /** Whether the settings page is up. */
  readonly settingsOpen: boolean
  /** Toggle one popover by trigger id. */
  readonly onToggleMenu: (id: string) => void
  /** Close whichever popover is open. */
  readonly onCloseMenu: () => void
  /** Adopt a workspace as the new-conversation target. */
  readonly onChooseWorkspace: (workspaceId: string) => void
  /** Open or close the settings page. */
  readonly onToggleSettings: () => void
  /** Hide the panel. */
  readonly onHide: () => void
}

/**
 * Render the top bar.
 *
 * The bar itself is the window's drag region (`data-tauri-drag-region="deep"`), so the
 * pickers inside it are the only places a press is not a drag.
 *
 * @param props - snapshot and handlers.
 * @returns the top bar.
 */
export function TopBar({
  state, targetWorkspace, openMenu, settingsOpen, onToggleMenu, onCloseMenu,
  onChooseWorkspace, onToggleSettings, onHide,
}: TopBarProps) {
  return (
    <div className="q-top" data-tauri-drag-region="deep">
      <span className="q-brand">DeepSeek</span>
      <div className="q-context">
        <WorkspacePicker
          state={state}
          targetWorkspace={targetWorkspace}
          open={openMenu === 'q-workspace'}
          onToggle={() => { onToggleMenu('q-workspace') }}
          onClose={onCloseMenu}
          onChoose={onChooseWorkspace}
        />
        <span className="q-divider" aria-hidden="true">/</span>
        <SessionPicker
          state={state}
          targetWorkspace={targetWorkspace}
          open={openMenu === 'q-session'}
          onToggle={() => { onToggleMenu('q-session') }}
          onClose={onCloseMenu}
        />
      </div>
      <div className="q-tools">
        <button
          id="q-settings-open"
          type="button"
          className="q-close"
          aria-label="悬浮窗设置"
          aria-pressed={settingsOpen}
          onClick={onToggleSettings}
        >
          <Icon name="settings-2" />
        </button>
        <button id="q-close" type="button" className="q-close" aria-label="收起悬浮窗" onClick={onHide}>
          <Icon name="x" />
        </button>
      </div>
    </div>
  )
}

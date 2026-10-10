// The top bar's workspace picker.

import { api } from '../../api'
import { useT } from '../../hooks/useI18n'
import type { Snapshot } from '../../lib/state'
import { pinnedWorkspaceMark, workspaceTitle } from '../../lib/view-text'
import { Icon } from '../common/Icon'
import { PickerPopover } from './PickerPopover'
import { WorkspaceMenu } from '../menus/WorkspaceMenu'

/** Props for {@link WorkspacePicker}. */
export interface WorkspacePickerProps {
  /** The snapshot. */
  readonly state: Snapshot
  /** The workspace the next conversation would use (the user's pick, or the shell's). */
  readonly targetWorkspace: string | null
  /** Whether this popover is the open one. */
  readonly open: boolean
  /** Toggle request from the trigger. */
  readonly onToggle: () => void
  /** Close the popover. */
  readonly onClose: () => void
  /** Adopt a workspace as the new-conversation target. */
  readonly onChoose: (workspaceId: string) => void
}

/**
 * Render the workspace entry and its menu.
 *
 * @param props - snapshot, open state, and handlers.
 * @returns the picker.
 */
export function WorkspacePicker({ state, targetWorkspace, open, onToggle, onClose, onChoose }: WorkspacePickerProps) {
  const t = useT()
  const marked = state.session.following === null && pinnedWorkspaceMark(state, targetWorkspace)
  return (
    <PickerPopover
      id="q-workspace"
      menuId="q-workspace-menu"
      label={t('workspace.choose')}
      menuLabel={t('workspace.menuLabel')}
      open={open}
      onToggle={() => {
        // The host sends the workspace list only when asked, and the panel asks when the user
        // first looks at the picker. Without this the picker drew an empty list forever — the
        // old renderer asked here, and the React migration dropped the call (2026-10-09).
        if (state.session.workspaces.length === 0) void api.requestWorkspaces()
        onToggle()
      }}
      onClose={onClose}
      trigger={
        <>
          <span id="q-workspace-icon"><Icon name="folder" /></span>
          <span className="q-name" id="q-workspace-name">{t(workspaceTitle(state, targetWorkspace))}</span>
          <span id="q-workspace-pinmark" className="q-pinmark" hidden={!marked}>
            {marked && <Icon name="pin" />}
          </span>
          <span id="q-workspace-chevron" className="q-chevron"><Icon name="chevron-down" /></span>
        </>
      }
    >
      <WorkspaceMenu state={state} targetWorkspace={targetWorkspace} onChoose={onChoose} onClose={onClose} />
    </PickerPopover>
  )
}

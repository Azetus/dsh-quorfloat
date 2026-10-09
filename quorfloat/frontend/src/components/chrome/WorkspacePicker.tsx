// The top bar's workspace picker.

import type { Snapshot } from '../../lib/state'
import { pinnedWorkspaceMark, workspaceTitle } from '../../lib/view-text'
import { Icon } from '../common/Icon'
import { PickerPopover } from './PickerPopover'
import { WorkspaceMenu } from '../menus/WorkspaceMenu'

/** Props for {@link WorkspacePicker}. */
export interface WorkspacePickerProps {
  /** The snapshot. */
  readonly state: Snapshot
  /** The frontend's own preselect for a new conversation. */
  readonly workspaceChoice: string | null
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
export function WorkspacePicker({ state, workspaceChoice, open, onToggle, onClose, onChoose }: WorkspacePickerProps) {
  const marked = state.session.following === null && pinnedWorkspaceMark(state, workspaceChoice)
  return (
    <PickerPopover
      id="q-workspace"
      menuId="q-workspace-menu"
      label="选择工作区"
      menuLabel="工作区选择"
      open={open}
      onToggle={onToggle}
      onClose={onClose}
      trigger={
        <>
          <span id="q-workspace-icon"><Icon name="folder" /></span>
          <span className="q-name" id="q-workspace-name">{workspaceTitle(state, workspaceChoice)}</span>
          <span id="q-workspace-pinmark" className="q-pinmark" hidden={!marked}>
            {marked && <Icon name="pin" />}
          </span>
          <span id="q-workspace-chevron" className="q-chevron"><Icon name="chevron-down" /></span>
        </>
      }
    >
      <WorkspaceMenu state={state} workspaceChoice={workspaceChoice} onChoose={onChoose} onClose={onClose} />
    </PickerPopover>
  )
}

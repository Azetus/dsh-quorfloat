// The workspace picker's menu.

import { api } from '../../api'
import { workspaceLabel } from '../../lib/labels'
import type { Snapshot } from '../../lib/state'
import { workspaceTitle } from '../../lib/view-text'
import { MenuHeading } from './MenuHeading'
import { MenuHint } from './MenuHint'
import { MenuOption } from './MenuOption'
import { MenuRow } from './MenuRow'
import { MenuSeparator } from './MenuSeparator'
import { PinButton } from './PinButton'

/** Props for {@link WorkspaceMenu}. */
export interface WorkspaceMenuProps {
  /** The snapshot. */
  readonly state: Snapshot
  /** The frontend's own preselect for a new conversation. */
  readonly workspaceChoice: string | null
  /** Adopt a workspace as the new-conversation target. */
  readonly onChoose: (workspaceId: string) => void
  /** Close the menu after an action that should close it. */
  readonly onClose: () => void
}

/**
 * Render the workspace list.
 *
 * Choosing a workspace while following a conversation *is* the new-conversation action
 * (2026-10-09): it leaves the conversation with a detach and preselects this directory.
 * The pin button is what persists; it exists only in new-conversation state, because with
 * a session pin the workspace field is that session's projection, not a choice.
 *
 * @param props - snapshot, preselect, and handlers.
 * @returns the menu contents.
 */
export function WorkspaceMenu({ state, workspaceChoice, onChoose, onClose }: WorkspaceMenuProps) {
  return (
    <>
      <MenuHeading title="工作区" detail={workspaceTitle(state, workspaceChoice)} />
      {state.session.workspaces.map(workspace => {
        const label = workspaceLabel(workspace.title, workspace.path)
        const selected = state.session.following === null && workspaceChoice === workspace.workspaceId
        const pinned = state.session.pinnedWorkspace === workspace.workspaceId
        return (
          <MenuRow key={workspace.workspaceId}>
            <MenuOption
              label={label}
              selected={selected}
              title={workspace.path}
              symbol="folder"
              onPick={() => {
                onChoose(workspace.workspaceId)
                onClose()
              }}
            />
            {state.session.following === null && (
              <PinButton
                name={`工作区 ${label}`}
                pinned={pinned}
                onPick={() => {
                  void api.pinWorkspace(pinned ? null : workspace.workspaceId)
                  onClose()
                }}
              />
            )}
          </MenuRow>
        )
      })}
      <MenuSeparator />
      <MenuHint text="选择工作区后，新会话在此开始。" />
    </>
  )
}

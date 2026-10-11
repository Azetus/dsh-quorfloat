// The workspace picker's menu.

import { api } from '../../api'
import { useT } from '../../hooks/useI18n'
import { workspaceLabel } from '../../logic/labels'
import type { Snapshot } from '../../logic/state'
import { workspaceTitle } from '../../logic/view-text'
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
  /** The workspace the next conversation would use (the user's pick, or the shell's). */
  readonly targetWorkspace: string | null
  /** Adopt a workspace as the new-conversation target. */
  readonly onChoose: (workspaceId: string) => void
  /** Close the menu after an action that should close it. */
  readonly onClose: () => void
}

/**
 * Render the workspace list.
 *
 * Choosing a workspace while following a conversation *is* the new-conversation action:
 * it leaves the conversation with a detach and preselects this directory.
 * The pin button is what persists; it exists only in new-conversation state, because with
 * a session pin the workspace field is that session's projection, not a choice.
 *
 * @param props - snapshot, preselect, and handlers.
 * @returns the menu contents.
 */
export function WorkspaceMenu({ state, targetWorkspace, onChoose, onClose }: WorkspaceMenuProps) {
  const t = useT()
  return (
    <>
      <MenuHeading title={t('workspace.heading')} detail={t(workspaceTitle(state, targetWorkspace))} />
      {state.session.workspaces.map(workspace => {
        const label = workspaceLabel(workspace.title, workspace.path)
        const selected = state.session.following === null && targetWorkspace === workspace.workspaceId
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
                name={t('workspace.pinName', { name: label })}
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
      <MenuHint text={t(workspaceHint(state))} />
    </>
  )
}

/**
 * What the workspace menu says when there is nothing to pick.
 *
 * Three different facts, and the whole point of `workspacesLoaded` is that they are not one:
 * a list that has not been answered yet needs patience; a list that was answered empty is
 * either waiting for the host's default (the request is out) or a refusal the user can only
 * fix in the Harness. The `workspace.needHost` prompt belongs to the refusal and to nothing
 * else — showing it while the default is still being created would send the user away from a
 * panel that was about to have a workspace.
 *
 * @param state - the snapshot.
 * @returns the dictionary key for the hint.
 */
function workspaceHint(state: Snapshot): 'workspace.listPending' | 'workspace.needHost' | 'workspace.chooseHint' {
  if (!state.session.workspacesLoaded) return 'workspace.listPending'
  if (state.session.workspaces.length > 0) return 'workspace.chooseHint'
  return state.session.defaultWorkspace.state === 'failed' ? 'workspace.needHost' : 'workspace.listPending'
}

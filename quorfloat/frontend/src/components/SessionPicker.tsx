// The top bar's conversation picker.

import type { Snapshot } from '../lib/state'
import { sessionTitle } from '../lib/view-text'
import { Icon } from './Icon'
import { PickerPopover } from './PickerPopover'
import { SessionMenu } from './SessionMenu'

/** Props for {@link SessionPicker}. */
export interface SessionPickerProps {
  /** The snapshot. */
  readonly state: Snapshot
  /** The frontend's own workspace preselect, for the menu's heading context. */
  readonly workspaceChoice: string | null
  /** Whether this popover is the open one. */
  readonly open: boolean
  /** Toggle request from the trigger. */
  readonly onToggle: () => void
  /** Close the popover. */
  readonly onClose: () => void
}

/**
 * Render the conversation entry and its menu.
 *
 * @param props - snapshot and handlers.
 * @returns the picker.
 */
export function SessionPicker({ state, workspaceChoice, open, onToggle, onClose }: SessionPickerProps) {
  const pinned = state.session.pinned !== null
  return (
    <PickerPopover
      id="q-session"
      menuId="q-session-menu"
      label="选择会话"
      menuLabel="会话选择"
      wrapClass="q-session-wrap"
      open={open}
      onToggle={onToggle}
      onClose={onClose}
      trigger={
        <>
          <span id="q-session-icon"><Icon name="message-square" /></span>
          <span className="q-name" id="q-session-name">{sessionTitle(state)}</span>
          <span id="q-session-pinmark" className="q-pinmark" hidden={!pinned}>
            {pinned && <Icon name="pin" />}
          </span>
          <span id="q-session-chevron" className="q-chevron"><Icon name="chevron-down" /></span>
        </>
      }
    >
      <SessionMenu state={state} workspaceChoice={workspaceChoice} onClose={onClose} />
    </PickerPopover>
  )
}

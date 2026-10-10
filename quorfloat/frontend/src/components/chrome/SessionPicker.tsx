// The top bar's conversation picker.

import { useT } from '../../hooks/useI18n'
import type { Snapshot } from '../../lib/state'
import { sessionTitle } from '../../lib/view-text'
import { Icon } from '../common/Icon'
import { PickerPopover } from './PickerPopover'
import { SessionMenu } from '../menus/SessionMenu'

/** Props for {@link SessionPicker}. */
export interface SessionPickerProps {
  /** The snapshot. */
  readonly state: Snapshot
  /** The workspace the next conversation would use, for the menu's heading context. */
  readonly targetWorkspace: string | null
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
export function SessionPicker({ state, targetWorkspace, open, onToggle, onClose }: SessionPickerProps) {
  const t = useT()
  const pinned = state.session.pinned !== null
  return (
    <PickerPopover
      id="q-session"
      menuId="q-session-menu"
      label={t('session.pickerLabel')}
      menuLabel={t('session.menuLabel')}
      wrapClass="q-session-wrap"
      open={open}
      onToggle={onToggle}
      onClose={onClose}
      trigger={
        <>
          <span id="q-session-icon"><Icon name="message-square" /></span>
          <span className="q-name" id="q-session-name">{t(sessionTitle(state))}</span>
          <span id="q-session-pinmark" className="q-pinmark" hidden={!pinned}>
            {pinned && <Icon name="pin" />}
          </span>
          <span id="q-session-chevron" className="q-chevron"><Icon name="chevron-down" /></span>
        </>
      }
    >
      <SessionMenu state={state} targetWorkspace={targetWorkspace} onClose={onClose} />
    </PickerPopover>
  )
}

// The top bar's conversation picker.

import { useT } from '../../hooks/useI18n'
import type { Scope } from '../../logic/scope'
import type { Snapshot } from '../../logic/state'
import { sessionTitle } from '../../logic/view-text'
import { Icon } from '../common/Icon'
import { PickerPopover } from './PickerPopover'
import { SessionMenu } from '../menus/SessionMenu'

/** Props for {@link SessionPicker}. */
export interface SessionPickerProps {
  /** The snapshot. */
  readonly state: Snapshot
  /** The scope the menu lists: the workspace field's own value, sticky while it is unresolved. */
  readonly scope: Scope | null
  /** Open a conversation and leave any explicit browsing choice. */
  readonly onSelect: (sessionId: string) => void
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
 * @param props - snapshot, scope, and handlers.
 * @returns the picker.
 */
export function SessionPicker({ state, scope, onSelect, open, onToggle, onClose }: SessionPickerProps) {
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
      <SessionMenu state={state} scope={scope} onSelect={onSelect} onClose={onClose} />
    </PickerPopover>
  )
}

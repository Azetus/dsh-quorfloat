// The runtime row's permission picker — the footer entry whose glyph follows the
// permission actually in force.

import { useT } from '../../hooks/useI18n'
import { permissionIcon } from '../../lib/icons'
import type { Snapshot } from '../../lib/state'
import { Icon } from '../common/Icon'
import { PickerPopover } from './PickerPopover'
import { PermissionMenu } from '../menus/PermissionMenu'

/** Props for {@link PermissionPicker}. */
export interface PermissionPickerProps {
  /** The snapshot. */
  readonly state: Snapshot
  /** Whether this popover is the open one. */
  readonly open: boolean
  /** Toggle request from the trigger. */
  readonly onToggle: () => void
  /** Close the popover. */
  readonly onClose: () => void
}

/**
 * Render the permission entry and its menu.
 *
 * The entry and the selected row of its own menu must agree: one permission has one glyph,
 * and a footer showing the uniform shield over a menu that draws an eye for the same
 * permission reads as two different states.
 *
 * @param props - snapshot and handlers.
 * @returns the picker.
 */
export function PermissionPicker({ state, open, onToggle, onClose }: PermissionPickerProps) {
  const t = useT()
  const options = state.session.options
  const name = options?.permissions.find(p => p.value === options.permission)?.name ?? '—'
  return (
    <PickerPopover
      id="q-permission"
      menuId="q-permission-menu"
      label={t('permission.pickerLabel')}
      menuLabel={t('permission.menuLabel')}
      wrapClass="q-permission-wrap"
      menuClass="q-up"
      open={open}
      onToggle={onToggle}
      onClose={onClose}
      trigger={
        <>
          <span id="q-permission-icon" className="q-permission-icon">
            <Icon name={permissionIcon(options?.permission ?? null)} />
          </span>
          <span id="q-permission-name">{name}</span>
          <span id="q-permission-chevron" className="q-chevron"><Icon name="chevron-down" /></span>
        </>
      }
    >
      <PermissionMenu state={state} onClose={onClose} />
    </PickerPopover>
  )
}

// The permission picker's menu: the three built-in presets, each with its own glyph.

import { api } from '../../api'
import { useT } from '../../hooks/useI18n'
import { permissionIcon } from '../../lib/icons'
import type { Snapshot } from '../../lib/state'
import { MenuHeading } from './MenuHeading'
import { MenuHint } from './MenuHint'
import { MenuOption } from './MenuOption'

/** Props for {@link PermissionMenu}. */
export interface PermissionMenuProps {
  /** The snapshot. */
  readonly state: Snapshot
  /** Close the menu after a pick. */
  readonly onClose: () => void
}

/**
 * Render the permission presets the host offers.
 *
 * The glyph follows the preset's *value* (see `permissionIcon`), so a host that renames
 * its presets still shows the right one.
 *
 * @param props - snapshot and the close handler.
 * @returns the menu contents.
 */
export function PermissionMenu({ state, onClose }: PermissionMenuProps) {
  const t = useT()
  const options = state.session.options
  return (
    <>
      <MenuHeading title={t('permission.heading')} />
      {options === null
        ? <MenuHint text={t('permission.catalogPending')} />
        : options.permissions.map(permission => (
          <MenuOption
            key={permission.value}
            label={permission.name}
            selected={options.permission === permission.value}
            description={permission.description ?? ''}
            symbol={permissionIcon(permission.value)}
            onPick={() => {
              onClose()
              void api.setPermission(permission.value)
            }}
          />
        ))}
    </>
  )
}

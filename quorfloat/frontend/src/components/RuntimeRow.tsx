// The conversation page's bottom row: permission on the left, model and effort on the right.

import type { Snapshot } from '../lib/state'
import { ModelPicker } from './ModelPicker'
import { PermissionPicker } from './PermissionPicker'

/** Props for {@link RuntimeRow}. */
export interface RuntimeRowProps {
  /** The snapshot. */
  readonly state: Snapshot
  /** Which popover is open, if any. */
  readonly openMenu: string | null
  /** Toggle one popover by trigger id. */
  readonly onToggleMenu: (id: string) => void
  /** Close whichever popover is open. */
  readonly onCloseMenu: () => void
}

/**
 * Render the runtime row.
 *
 * It belongs to the conversation page only: the settings page replaces the whole body
 * (`#q-main`), and these controls describe the conversation, not the panel.
 *
 * @param props - snapshot, menu state, and handlers.
 * @returns the row.
 */
export function RuntimeRow({ state, openMenu, onToggleMenu, onCloseMenu }: RuntimeRowProps) {
  return (
    <div className="q-runtime">
      <PermissionPicker
        state={state}
        open={openMenu === 'q-permission'}
        onToggle={() => { onToggleMenu('q-permission') }}
        onClose={onCloseMenu}
      />
      <ModelPicker
        state={state}
        open={openMenu === 'q-config'}
        onToggle={() => { onToggleMenu('q-config') }}
        onClose={onCloseMenu}
      />
    </div>
  )
}

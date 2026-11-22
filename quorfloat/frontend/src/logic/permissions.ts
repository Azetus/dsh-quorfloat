// The display name of a permission preset.
//
// The catalog's `value` is the stable identity; its `name` is the host's label and may be
// written in whatever language the Harness happens to use. The panel therefore names the
// three built-in presets from its own dictionary and leaves every other label alone — a
// configured preset keeps the name the user gave it.

import { msg, raw, type Text } from './i18n'

/**
 * Resolve one permission preset's label for the panel.
 *
 * @param value - the preset's stable machine value from `session/options`.
 * @param name - the host-supplied display label, used for everything outside the built-ins.
 * @returns the dictionary-backed label, or the host label verbatim.
 */
export function permissionLabel(value: string, name: string): Text {
  switch (value) {
    case 'read-only':
      return msg('permission.readOnly')
    case 'workspace-write':
      return msg('permission.workspaceWrite')
    case 'danger-full-access':
      return msg('permission.fullAccess')
    default:
      return raw(name)
  }
}

// The runtime row's model + reasoning-effort picker.

import type { Snapshot } from '../../lib/state'
import { Icon } from '../common/Icon'
import { ModelMenu } from '../menus/ModelMenu'
import { PickerPopover } from './PickerPopover'

/** Props for {@link ModelPicker}. */
export interface ModelPickerProps {
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
 * Render the model entry and its menu.
 *
 * The effort beside the model is blank while the host has not reported one — the entry
 * never guesses.
 *
 * @param props - snapshot and handlers.
 * @returns the picker.
 */
export function ModelPicker({ state, open, onToggle, onClose }: ModelPickerProps) {
  const options = state.session.options
  const current = options?.current
  const model = options?.models.find(m => m.id === current?.model && m.provider === current?.provider)
  const effort = model?.efforts.find(e => e.id === current?.reasoningEffort)?.name ?? ''
  return (
    <PickerPopover
      id="q-config"
      menuId="q-config-menu"
      label="模型与推理等级"
      menuLabel="模型与推理等级设置"
      wrapClass="q-config-wrap"
      menuClass="q-right q-up q-config-menu"
      open={open}
      onToggle={onToggle}
      onClose={onClose}
      trigger={
        <>
          <span id="q-model-name">{model?.name ?? '—'}</span>
          <span id="q-effort-name" className="q-config-effort">{effort}</span>
          <span id="q-config-chevron" className="q-chevron"><Icon name="chevron-down" /></span>
        </>
      }
    >
      <ModelMenu state={state} onClose={onClose} />
    </PickerPopover>
  )
}

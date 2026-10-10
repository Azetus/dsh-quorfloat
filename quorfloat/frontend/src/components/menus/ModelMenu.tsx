// The model + reasoning-effort menu.

import { api } from '../../api'
import { useT } from '../../hooks/useI18n'
import type { Snapshot } from '../../lib/state'
import { MenuHeading } from './MenuHeading'
import { MenuHint } from './MenuHint'
import { MenuOption } from './MenuOption'
import { MenuSeparator } from './MenuSeparator'

/** Props for {@link ModelMenu}. */
export interface ModelMenuProps {
  /** The snapshot. */
  readonly state: Snapshot
  /** Close the menu after a pick. */
  readonly onClose: () => void
}

/**
 * Render the model catalog and the current model's effort levels.
 *
 * The host's catalog carries a description per model and per effort, and the design's own
 * sub-list deliberately spends no line on it (`option(label, selected, onPick)`): a list of
 * names is scanned, a list of paragraphs is read. User decided on 2026-10-09 to drop them
 * from both lists (see `progress.md`).
 *
 * @param props - snapshot and the close handler.
 * @returns the menu contents.
 */
export function ModelMenu({ state, onClose }: ModelMenuProps) {
  const t = useT()
  const options = state.session.options
  if (options === null) {
    return (
      <>
        <MenuHeading title={t('model.heading')} />
        <MenuHint text={t('model.catalogPending')} />
      </>
    )
  }
  const current = options.current
  const currentModel = options.models.find(m => m.id === current?.model && m.provider === current?.provider)
  const efforts = currentModel?.efforts ?? []
  return (
    <>
      {/* The design's back row. Our menu is one flat list (the design's two-level
          navigation was flattened when the panel was built), so this is a heading, not a
          control — a dead button would be a lie about what the panel can do. */}
      <div className="q-config-back">{t('model.backRow')}</div>
      <MenuHeading title={t('model.heading')} />
      {options.models.map(model => (
        <MenuOption
          key={`${model.provider}/${model.id}`}
          label={model.name}
          selected={current?.model === model.id && current?.provider === model.provider}
          onPick={() => {
            onClose()
            void api.selectModel(model.provider, model.id, current?.reasoningEffort ?? model.defaultEffort ?? null)
          }}
        />
      ))}
      <MenuSeparator />
      <MenuHeading title={t('model.effortHeading')} />
      {efforts.map(effort => (
        <MenuOption
          key={effort.id}
          label={effort.name}
          selected={current?.reasoningEffort === effort.id}
          onPick={() => {
            onClose()
            if (current !== null) void api.selectModel(current.provider, current.model, effort.id)
          }}
        />
      ))}
      {efforts.length === 0 && <MenuHint text={t('model.noEfforts')} />}
    </>
  )
}

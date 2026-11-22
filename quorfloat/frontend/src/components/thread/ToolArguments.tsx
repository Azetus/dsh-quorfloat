// The JSON arguments of one tool call, bounded only for display.

import { useMemo, useState } from 'react'
import { useT } from '../../hooks/useI18n'
import { previewText } from '../../logic/text-preview'

/** Props for {@link ToolArguments}. */
export interface ToolArgumentsProps {
  /** The full JSON arguments text from the snapshot. */
  readonly text: string
}

/**
 * Render a tool call's arguments, with an explicit full-view toggle for long payloads.
 *
 * @param props - the full arguments text.
 * @returns the preview or the expanded text.
 */
export function ToolArguments({ text }: ToolArgumentsProps) {
  const t = useT()
  const preview = useMemo(() => previewText(text), [text])
  const [full, setFull] = useState(false)
  if (!preview.truncated || full) {
    return (
      <span className="q-call-args">
        {` ${full ? text : preview.text}`}
        {full && (
          <button type="button" className="q-preview-toggle" onClick={() => { setFull(false) }}>
            {t('working.showLess')}
          </button>
        )}
      </span>
    )
  }
  return (
    <span className="q-call-args">
      {` ${preview.text}`}
      <span className="q-preview-note">{t('working.truncated', { total: preview.total })}</span>
      <button type="button" className="q-preview-toggle" onClick={() => { setFull(true) }}>
        {t('working.showFull')}
      </button>
    </span>
  )
}

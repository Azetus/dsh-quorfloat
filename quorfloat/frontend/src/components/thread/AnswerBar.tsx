// The bar under an answer: its state, and the copy action once it is finished.

import { useState } from 'react'
import { useT } from '../../hooks/useI18n'

/** Props for {@link AnswerBar}. */
export interface AnswerBarProps {
  /** The answer's raw Markdown, for the clipboard. */
  readonly markdown: string
}

/**
 * Render the answer bar.
 *
 * It is the answer's own footer, so it exists only once the answer is finished: while the text
 * streams there is nothing to report and a row below the text would only jitter.
 *
 * The answer is copied as the raw Markdown, not the rendered text — that is what a reader
 * pasting it somewhere else wants back.
 *
 * @param props - the raw Markdown.
 * @returns the bar.
 */
export function AnswerBar({ markdown }: AnswerBarProps) {
  const t = useT()
  const [note, setNote] = useState<string | null>(null)
  return (
    <div className="q-answerbar">
      <span>{t('answer.done')}</span>
      {markdown.trim() !== '' && (
        <button
          type="button"
          onClick={() => {
            navigator.clipboard
              .writeText(markdown)
              .then(() => { setNote(t('answer.copied')) })
              .catch(() => { setNote(t('answer.copyManually')) })
          }}
        >
          {note ?? t('answer.copy')}
        </button>
      )}
    </div>
  )
}

// The bar under an answer: its state, and the copy action once it is finished.

import { useState } from 'react'

/** Props for {@link AnswerBar}. */
export interface AnswerBarProps {
  /** Whether the turn is still being worked on. */
  readonly working: boolean
  /** The answer's raw Markdown, for the clipboard. */
  readonly markdown: string
}

/**
 * Render the answer bar.
 *
 * The answer is copied as the raw Markdown, not the rendered text — that is what a reader
 * pasting it somewhere else wants back.
 *
 * @param props - working state and the raw Markdown.
 * @returns the bar.
 */
export function AnswerBar({ working, markdown }: AnswerBarProps) {
  const [note, setNote] = useState<string | null>(null)
  return (
    <div className="q-answerbar">
      <span>{working ? '正在生成' : '回答完成'}</span>
      {!working && markdown.trim() !== '' && (
        <button
          type="button"
          onClick={() => {
            navigator.clipboard
              .writeText(markdown)
              .then(() => { setNote('已复制') })
              .catch(() => { setNote('请手动选择复制') })
          }}
        >
          {note ?? '复制回答'}
        </button>
      )}
    </div>
  )
}

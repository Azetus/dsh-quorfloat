// One question of a card: heading, text, detail, the choices, and the free-text answer.

import { useLayoutEffect, useRef } from 'react'
import { useT } from '../../hooks/useI18n'
import type { QuestionDraft } from '../../lib/questions'
import type { InteractionQuestion } from '../../lib/state'
import { QuestionChoice } from './QuestionChoice'

/** Props for {@link QuestionBlock}. */
export interface QuestionBlockProps {
  /** The question. */
  readonly question: InteractionQuestion
  /** What the user has done about it so far. */
  readonly draft: QuestionDraft
  /** Whether the block is read-only (submitting, or already resolved). */
  readonly frozen: boolean
  /** Pick or unpick one option. */
  readonly onPick: (label: string) => void
  /** Record the free-text answer. */
  readonly onCustom: (text: string) => void
  /** The field grew: the panel has to remeasure (the shell sizes the native window). */
  readonly onGrow: () => void
}

/**
 * Render one question.
 *
 * A question with no options is a free-text question (`ask()` allows one), so the field is
 * the whole answer rather than an "other" row — and it is framed like one.
 *
 * @param props - the question, its draft, and the handlers.
 * @returns the block.
 */
export function QuestionBlock({ question, draft, frozen, onPick, onCustom, onGrow }: QuestionBlockProps) {
  const t = useT()
  const field = useRef<HTMLTextAreaElement>(null)

  // Two lines of room, then the field scrolls: a card that grew without bound would push
  // the conversation out of a panel whose height is decided by the shell, not by this box.
  useLayoutEffect(() => {
    const input = field.current
    if (input === null) return
    input.style.height = 'auto'
    input.style.height = `${Math.min(input.scrollHeight, 58)}px`
    input.style.overflowY = input.scrollHeight > 58 ? 'auto' : 'hidden'
  })

  const freeText = question.options.length === 0
  return (
    <div className="q-question-block">
      {question.header !== null && <div className="q-question-head">{question.header}</div>}
      <div className="q-question-text">{question.question}</div>
      {question.detail !== null && question.detail !== '' && (
        <div className="q-question-detail">{question.detail}</div>
      )}
      {question.options.map(option => (
        <QuestionChoice
          key={option.label}
          label={option.label}
          description={option.description}
          selected={draft.selected.includes(option.label)}
          multiSelect={question.multiSelect}
          disabled={frozen}
          onPick={() => { onPick(option.label) }}
        />
      ))}
      <textarea
        ref={field}
        className={freeText ? 'q-question-field q-question-field-free' : 'q-question-field'}
        rows={1}
        aria-label={freeText ? question.question : t('question.otherAnswerLabel', { question: question.question })}
        placeholder={freeText ? t('question.answerPlaceholder') : t('question.otherPlaceholder')}
        value={draft.custom}
        disabled={frozen}
        onInput={event => {
          onCustom(event.currentTarget.value)
          onGrow()
        }}
      />
    </div>
  )
}

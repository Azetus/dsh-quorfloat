// A model question, with the widgets that answer it.
//
// The asker's own vocabulary: `selected` holds option *labels*, `custom` is the free-text
// answer, and one request may carry several questions whose answers travel together in one
// `answers` array — which is why this card submits once, for all of them.
//
// The asker's `intent` (e.g. `plan-review`) is deliberately ignored: upstream says an intent
// changes presentation only, never the encoding, and a UI that does not know the tag renders
// the generic option list — which is exactly this card. A card this build cannot answer
// (`actionable: false`: too many questions, or one without an id) keeps the old behaviour and
// says so, because an answer must cover what the user was shown.

import { useState } from 'react'
import { api } from '../../api'
import {
  answerPayload, emptyDrafts, isComplete, setCustom, toggleChoice,
  type QuestionDraft, type QuestionDrafts,
} from '../../lib/questions'
import type { Interaction, InteractionQuestion } from '../../lib/state'
import { Icon } from '../common/Icon'
import { QuestionBlock } from './QuestionBlock'

/** Props for {@link QuestionCard}. */
export interface QuestionCardProps {
  /** The pending (or settled) question. */
  readonly interaction: Interaction
  /** A field in the card grew: the panel has to remeasure. */
  readonly onGrow: () => void
}

/**
 * The drafts a settled card shows: what was sent, so the card can still say what it asked.
 *
 * @param answers - the answers the shell echoed back.
 * @returns one draft per answered question.
 */
function draftsFrom(answers: Interaction['answers']): QuestionDrafts {
  const drafts: Record<string, QuestionDraft> = {}
  for (const answer of answers) {
    drafts[answer.id] = { selected: answer.selected, custom: answer.custom ?? '' }
  }
  return drafts
}

/**
 * Render the card.
 *
 * @param props - the interaction and the remeasure hook.
 * @returns the card.
 */
export function QuestionCard({ interaction, onGrow }: QuestionCardProps) {
  const questions: InteractionQuestion[] = interaction.questions
  const [drafts, setDrafts] = useState<QuestionDrafts>(() => emptyDrafts(questions))

  const frozen = interaction.state !== 'pending'
  const answerable = interaction.actionable && interaction.state === 'pending'
  // A settled card shows what was answered; a pending one shows the user's edits. Once the
  // shell has applied an answer, its echo is the authority, not what this component held.
  const shown = interaction.state === 'applied' ? draftsFrom(interaction.answers) : drafts
  const complete = isComplete(questions, shown)

  /** What the state line says, if anything. */
  const stateLine = (): { readonly text: string; readonly refused: boolean } | null => {
    if (interaction.state === 'submitting') return { text: '提交中…', refused: false }
    if (interaction.state === 'applied') return { text: '已提交', refused: false }
    if (interaction.state === 'refused') {
      return { text: interaction.refusalReason ?? '请求已失效', refused: true }
    }
    if (!interaction.actionable) return { text: '这个问题请在 Harness 窗口回答', refused: false }
    if (!complete) return { text: '请先回答所有问题', refused: false }
    return null
  }
  const line = stateLine()

  return (
    <div className="q-card">
      <div className="q-card-title">
        <Icon name="message-square" />
        <strong>需要你的回答</strong>
      </div>
      {questions.map(question => (
        <QuestionBlock
          key={question.id}
          question={question}
          draft={shown[question.id] ?? { selected: [], custom: '' }}
          frozen={frozen || !interaction.actionable}
          onPick={label => { setDrafts(current => toggleChoice(question, current, label)) }}
          onCustom={text => { setDrafts(current => setCustom(question, current, text)) }}
          onGrow={onGrow}
        />
      ))}
      {interaction.questionCount > questions.length && (
        <div className="q-card-reason">
          {`还有 ${interaction.questionCount - questions.length} 个问题没有显示。`}
        </div>
      )}
      {line !== null && (
        <div className={line.refused ? 'q-card-state q-refused' : 'q-card-state'}>{line.text}</div>
      )}
      <div className="q-card-actions">
        {answerable ? (
          <button
            type="button"
            className="q-allow"
            disabled={!complete}
            onClick={() => { void api.answerQuestion(interaction.id, answerPayload(questions, shown)) }}
          >
            提交
          </button>
        ) : (
          <button
            type="button"
            disabled={interaction.state === 'submitting'}
            onClick={() => { void api.dismissInteraction(interaction.id) }}
          >
            {interaction.actionable ? '关闭' : '知道了'}
          </button>
        )}
      </div>
    </div>
  )
}

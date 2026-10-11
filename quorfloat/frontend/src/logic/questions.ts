// The question card's decisions, kept pure so the widgets above them are only widgets.
//
// Upstream's answer shape is `{ id, selected: string[], custom? }` where `selected` holds
// option *labels* (not ids), so the draft is a small map from question id to the labels
// picked plus the free-text answer — and building the payload is a pure function of the
// two, never a read of the DOM.

import type { Interaction } from './state'

/** One question of a card, as the shell serialises it. */
export type Question = Interaction['questions'][number]

/** What the user has done about one question so far. */
export interface QuestionDraft {
  /** Option labels picked, in the order offered. */
  readonly selected: readonly string[]
  /** The free-text answer ("其他…"), untrimmed as typed. */
  readonly custom: string
}

/** Every question's draft, keyed by question id. */
export type QuestionDrafts = Readonly<Record<string, QuestionDraft>>

/** One answer, in the host's vocabulary. */
export interface QuestionAnswer {
  /** The question being answered. */
  readonly id: string
  /** Option labels the user picked. */
  readonly selected: readonly string[]
  /** Free text, present only when the user typed some. */
  readonly custom?: string
}

/** An empty draft for one question. */
const EMPTY: QuestionDraft = { selected: [], custom: '' }

/**
 * Start every question's draft empty.
 *
 * @param questions - the card's questions.
 * @returns one draft per question id.
 */
export function emptyDrafts(questions: readonly Question[]): QuestionDrafts {
  const drafts: Record<string, QuestionDraft> = {}
  for (const question of questions) drafts[question.id] = EMPTY
  return drafts
}

/**
 * Whether one question has been answered.
 *
 * A question counts as answered when something was picked or something was typed: the
 * host accepts an empty selection as long as there is custom text, and vice versa.
 *
 * @param draft - the question's draft, if any.
 * @returns whether it can be sent.
 */
export function isAnswered(draft: QuestionDraft | undefined): boolean {
  if (draft === undefined) return false
  return draft.selected.length > 0 || draft.custom.trim() !== ''
}

/**
 * Whether every question of the card has an answer.
 *
 * All of them, because the payload is one `answers` array for the whole card: sending a
 * partial one would answer questions the user never saw a widget for.
 *
 * @param questions - the card's questions.
 * @param drafts - the drafts so far.
 * @returns whether the card can be submitted.
 */
export function isComplete(questions: readonly Question[], drafts: QuestionDrafts): boolean {
  return questions.length > 0 && questions.every(question => isAnswered(drafts[question.id]))
}

/**
 * Pick or unpick one option.
 *
 * Single-select replaces the selection (a radio, even though it is drawn as a list);
 * multi-select toggles, and keeps the offered order so the payload does not depend on
 * click order.
 *
 * @param question - the question being answered.
 * @param drafts - the drafts so far.
 * @param label - the option's label.
 * @returns the drafts with this pick applied.
 */
export function toggleChoice(
  question: Question,
  drafts: QuestionDrafts,
  label: string,
): QuestionDrafts {
  const current = drafts[question.id] ?? EMPTY
  const next = current.selected.includes(label)
    ? current.selected.filter(value => value !== label)
    : [...current.selected, label]
  // The payload follows the order the options were offered in, not the click order.
  const ordered = question.options
    .filter(option => next.includes(option.label))
    .map(option => option.label)
  const selected = question.multiSelect === true ? ordered : [label]
  return { ...drafts, [question.id]: { ...current, selected } }
}

/**
 * Record the free-text answer of one question.
 *
 * @param question - the question being answered.
 * @param drafts - the drafts so far.
 * @param custom - what the field now holds.
 * @returns the drafts with this text applied.
 */
export function setCustom(
  question: Question,
  drafts: QuestionDrafts,
  custom: string,
): QuestionDrafts {
  const current = drafts[question.id] ?? EMPTY
  return { ...drafts, [question.id]: { ...current, custom } }
}

/**
 * Build the wire answers.
 *
 * @param questions - the card's questions, in display order.
 * @param drafts - the drafts.
 * @returns one answer per question, with blank custom text left out.
 */
export function answerPayload(
  questions: readonly Question[],
  drafts: QuestionDrafts,
): QuestionAnswer[] {
  return questions.map(question => {
    const draft = drafts[question.id] ?? EMPTY
    const custom = draft.custom.trim()
    return custom === ''
      ? { id: question.id, selected: draft.selected }
      : { id: question.id, selected: draft.selected, custom }
  })
}

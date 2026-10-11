// The turn folding rule, built from the documented contract. One turn is three parts:
//
// - the **question** — the turn's first user message;
// - one **fold** — every piece of LLM output that is not the final answer:
//   reasoning, tool calls and their results, intermediate narration;
// - the **final answer** — the turn's *last* assistant message that has text and
//   no tool-call; its own reasoning goes into the fold.
//
// A message ending in a tool call is still working, so it is never the answer.
// The rules are the project's own: nothing here aligns with the harness's own
// display logic.
//
// The fold key is the question **plus its position** in the entry list: the same
// question asked twice is two turns, and when the transcript trims the oldest
// lines the positions shift and the open/closed memory is *forgotten*, never
// reassigned to another turn.

import type { Block, Entry } from './state'

export interface WorkingLine {
  kind: 'reasoning' | 'text' | 'call' | 'tool'
  /** Tool name, for `call` and `tool` lines. */
  name: string | null
  text: string
  /** The JSON arguments text, for `call` lines. */
  arguments: string | null
  isError: boolean
}

export interface Turn {
  /** The question, verbatim. */
  question: string
  /** Everything the model produced except the final answer. */
  working: WorkingLine[]
  /** The final answer's text blocks, or null while it has not landed. */
  answer: { blocks: Block[]; streaming: boolean } | null
  /** Whether the turn is still being worked on (for the fold's label). */
  inProgress: boolean
  /** The fold's identity: question + position, per the module rules. */
  id: string
}

/** A lifecycle line that is not part of any turn, drawn on its own. */
export interface LooseLine {
  text: string
}

export interface ThreadView {
  turns: Turn[]
  loose: LooseLine[]
}

/**
 * Fold a flat transcript into turns.
 *
 * @param entries - the settled transcript lines, oldest first.
 * @param live - the stream currently being generated, if any.
 * @param turnActive - whether the host says a turn is in progress.
 * @returns the thread the panel draws.
 */
export function foldTurns(entries: Entry[], live: Entry | null, turnActive: boolean): ThreadView {
  const turns: Turn[] = []
  const loose: LooseLine[] = []

  const pushLoose = (text: string) => loose.push({ text })
  // The turn being assembled. A turn exists from its question onwards; entries
  // that arrive before any question are lifecycle lines (loose).
  let current: Turn | null = null
  // Where the current turn's answer text would go if it were demoted: right
  // after the message's own reasoning, which is where it sat chronologically.
  let answerTextAt = 0

  const ensureTurn = (question: string, position: number): Turn => {
    const turn: Turn = {
      question,
      working: [],
      answer: null,
      inProgress: false,
      id: `${position}\u0000${question}`,
    }
    turns.push(turn)
    answerTextAt = 0
    return turn
  }

  /** Demote a settled answer to working narration, at its chronological place. */
  const demoteAnswer = (turn: Turn) => {
    if (turn.answer === null) return
    turn.working.splice(answerTextAt, 0, ...answerToWorking(turn.answer))
    turn.answer = null
  }

  const workingFromBlocks = (blocks: Block[]): WorkingLine[] =>
    blocks.map(block => {
      switch (block.kind) {
        case 'reasoning':
          return { kind: 'reasoning', name: null, text: block.text, arguments: null, isError: false }
        case 'text':
          return { kind: 'text', name: null, text: block.text, arguments: null, isError: false }
        case 'call':
          return { kind: 'call', name: block.name, text: block.name, arguments: block.arguments, isError: false }
      }
    })

  entries.forEach((entry, index) => {
    switch (entry.kind) {
      case 'user':
        // A user message starts the next turn, closing the one before it.
        current = ensureTurn(entry.text, index)
        break
      case 'assistant': {
        const text = entry.blocks.filter(block => block.kind === 'text')
        const hasCall = entry.blocks.some(block => block.kind === 'call')
        if (text.length === 0 && !hasCall) {
          // Pure reasoning with nothing to say: fold it, no answer candidate.
          if (current !== null) current.working.push(...workingFromBlocks(entry.blocks))
          else pushLoose(plainText(entry.blocks))
          break
        }
        if (hasCall) {
          // Ends in (or contains) a tool call: still working, everything folds.
          if (current !== null) current.working.push(...workingFromBlocks(entry.blocks))
          else pushLoose(plainText(entry.blocks))
          break
        }
        // A text answer candidate: the last one wins, and any earlier candidate
        // becomes working narration. Its own reasoning goes to the fold.
        if (current === null) {
          pushLoose(plainText(entry.blocks))
          break
        }
        demoteAnswer(current)
        const reasoning = entry.blocks.filter(block => block.kind === 'reasoning')
        const kept = entry.blocks.filter(block => block.kind !== 'reasoning')
        current.working.push(...workingFromBlocks(reasoning))
        answerTextAt = current.working.length
        current.answer = { blocks: kept, streaming: entry.streaming }
        break
      }
      case 'tool':
        if (current !== null) {
          current.working.push({
            kind: 'tool',
            name: entry.name,
            text: entry.text,
            arguments: null,
            isError: entry.isError,
          })
        } else {
          pushLoose(entry.text)
        }
        break
      case 'system':
        // A lifecycle line: drawn, but never part of a turn's fold.
        pushLoose(entry.text)
        break
    }
  })

  // The live stream is the answer under construction: its text is the
  // in-progress answer, its reasoning belongs to the fold.
  if (live !== null && live.kind === 'assistant') {
    const text = live.blocks.filter(block => block.kind === 'text')
    const reasoning = live.blocks.filter(block => block.kind === 'reasoning')
    const hasCall = live.blocks.some(block => block.kind === 'call')
    // Read through the turns array rather than the loop's `current`: TypeScript
    // cannot see the closure's assignments, so `current` stays narrowed to its
    // initial null here.
    const currentTurn = turns.at(-1) ?? null
    if (currentTurn === null) {
      if (text.length > 0) pushLoose(plainText(live.blocks))
    } else if (hasCall) {
      currentTurn.working.push(...workingFromBlocks(live.blocks))
    } else {
      currentTurn.working.push(...workingFromBlocks(reasoning))
      if (currentTurn.answer !== null) demoteAnswer(currentTurn)
      answerTextAt = currentTurn.working.length
      currentTurn.answer = text.length > 0 ? { blocks: text, streaming: true } : null
    }
  }

  // The last turn is being worked on while the host says so — or while the
  // stream is live with no settled answer.
  const last = turns.at(-1)
  if (last !== undefined) {
    last.inProgress = turnActive || live !== null
  }

  return { turns, loose }
}

/** A settled answer demoted to working narration, verbatim. */
function answerToWorking(answer: { blocks: Block[] }): WorkingLine[] {
  return answer.blocks.map(block => {
    switch (block.kind) {
      case 'text':
        return { kind: 'text', name: null, text: block.text, arguments: null, isError: false }
      case 'reasoning':
        return { kind: 'reasoning', name: null, text: block.text, arguments: null, isError: false }
      case 'call':
        return { kind: 'call', name: block.name, text: block.name, arguments: block.arguments, isError: false }
    }
  })
}

/** The readable text of blocks, for a line drawn outside any turn. */
function plainText(blocks: Block[]): string {
  return blocks
    .filter(block => block.kind === 'text' || block.kind === 'reasoning')
    .map(block => block.text)
    .join('\n')
}

// One block of the working (folded) part of a turn: reasoning, narration, a tool call, or
// a tool result.

import type { Turn } from '../lib/turns'

/** Props for {@link WorkingLine}. */
export interface WorkingLineProps {
  /** The block to draw. */
  readonly line: Turn['working'][number]
}

/**
 * Render one working line.
 *
 * Reasoning and narration are their own classes (the design colours reasoning), a call
 * shows the tool's name plus its arguments, and a tool result is marked as an error when
 * the host says so.
 *
 * @param props - the block.
 * @returns the line.
 */
export function WorkingLine({ line }: WorkingLineProps) {
  switch (line.kind) {
    case 'reasoning':
      return <div className="q-working-line q-reasoning">{line.text}</div>
    case 'text':
      return <div className="q-working-line">{line.text}</div>
    case 'call':
      return (
        <div className="q-working-line q-call">
          <span className="q-tool-name">{`⚙ ${line.name ?? '工具'}`}</span>
          {line.arguments !== null && <span className="q-call-args">{` ${line.arguments}`}</span>}
        </div>
      )
    case 'tool':
      return (
        <div className={line.isError ? 'q-working-line q-tool q-error' : 'q-working-line q-tool'}>
          <span className="q-tool-name">{line.name !== null ? `↳ ${line.name}` : '↳ 工具结果'}</span>
          {` ${line.text}`}
        </div>
      )
  }
}

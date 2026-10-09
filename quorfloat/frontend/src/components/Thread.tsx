// The conversation body: the scroll area, and the window of turns it actually renders.
//
// Windowing (2026-10-09): the shell pushes a fresh snapshot on every stream event, so the
// panel used to rebuild the whole transcript each time — 23ms at 200 turns, 97ms at 1000,
// linear in the conversation and spent almost entirely on turns nobody can see. Only the
// rows inside the viewport (plus a few) are mounted now, and a row's height is measured
// rather than guessed, because an answer is as tall as its Markdown says.

import { useCallback, useLayoutEffect, useRef } from 'react'
import { useVirtualizer } from '@tanstack/react-virtual'
import type { Snapshot } from '../lib/state'
import { foldTurns, type Turn as TurnModel } from '../lib/turns'
import { Turn } from './Turn'

/** Props for {@link Thread}. */
export interface ThreadProps {
  /** The snapshot. */
  readonly state: Snapshot
  /** The fold state, keyed by turn identity. */
  readonly folds: ReadonlyMap<string, boolean>
  /** Toggle one turn's fold. */
  readonly onToggleFold: (key: string) => void
}

/** One row of the thread: a loose question line, or a turn. */
type Row =
  | { readonly kind: 'loose'; readonly key: string; readonly text: string }
  | { readonly kind: 'turn'; readonly key: string; readonly turn: TurnModel; readonly sep: boolean }

/** How many rows beyond the viewport stay mounted: enough that fast scrolling does not
 *  show a blank strip, few enough that a long conversation still costs nothing. */
const OVERSCAN = 3

/** A row's height before it has ever been measured. Turns are tall and paragraphs are
 *  short; the estimate only decides what the first paint of an unvisited region looks
 *  like, because every mounted row is measured for real. */
const ESTIMATE = { loose: 32, turn: 220 } as const

/**
 * Render the thread.
 *
 * Scroll rule (the panel's own): while the reader is at the bottom, new content keeps them
 * there; once they scroll up, the panel does not yank them back.
 *
 * @param props - snapshot and fold state.
 * @returns the scroll area.
 */
export function Thread({ state, folds, onToggleFold }: ThreadProps) {
  const scrollRef = useRef<HTMLDivElement>(null)
  // Whether the reader is at the bottom. Starts true: the first paint should show the end
  // of the conversation, which is what a chat panel is for.
  const follow = useRef(true)

  const view = foldTurns(state.transcript.entries, state.transcript.live, state.transcript.turnActive)
  const rows: Row[] = []
  view.loose.forEach((line, index) => {
    rows.push({ kind: 'loose', key: `loose-${index}`, text: line.text })
  })
  view.turns.forEach((turn, index) => {
    rows.push({
      kind: 'turn',
      key: turn.id,
      turn,
      // The design's separator between turns. It is decided by the row's place in the
      // *conversation*, not by which rows happen to be mounted: a turn that follows
      // another turn carries it (`.q-turn + .q-turn`'s rule, which windowing cannot
      // express). A loose question line between two turns breaks it, as it always did.
      sep: index > 0,
    })
  })

  // The rows are rebuilt on every commit (the snapshot is new each time), but the
  // virtualizer's callbacks must *not* be: a new function identity re-subscribes its
  // observers on every render, and its measurement cache is keyed through `getItemKey`.
  // They read the current rows through a ref instead.
  const latest = useRef(rows)
  latest.current = rows
  const getScrollElement = useCallback(() => scrollRef.current, [])
  const estimateSize = useCallback(
    (index: number) => (latest.current[index]?.kind === 'loose' ? ESTIMATE.loose : ESTIMATE.turn),
    [],
  )
  const getItemKey = useCallback((index: number) => latest.current[index]?.key ?? index, [])

  const virtual = useVirtualizer({
    count: rows.length,
    getScrollElement,
    estimateSize,
    getItemKey,
    overscan: OVERSCAN,
    // Where the reader is looking when the panel first paints. The scroll position is set
    // in the layout effect below, but the *range* is computed before that, from this
    // number: without it the first paint mounts the head of the conversation and shows a
    // blank viewport until the browser reports the scroll it was just given. "At the
    // bottom" is expressed as an offset past the end because the total height is still an
    // estimate at this point — the range clamps to the last row either way.
    initialOffset: () => (follow.current ? Number.MAX_SAFE_INTEGER : 0),
  })

  useLayoutEffect(() => {
    const scroll = scrollRef.current
    if (scroll === null || !follow.current) return
    scroll.scrollTop = scroll.scrollHeight
  })

  return (
    <div
      id="q-thread-scroll"
      className="q-thread-scroll"
      ref={scrollRef}
      onScroll={event => {
        const el = event.currentTarget
        follow.current = el.scrollTop + el.clientHeight >= el.scrollHeight - 4
      }}
    >
      <div id="q-thread" className="q-thread">
        {rows.length > 0 && (
          <div className="q-thread-items" style={{ height: virtual.getTotalSize() }}>
            {virtual.getVirtualItems().map(item => {
              const row = rows[item.index]
              if (row === undefined) return null
              return (
                <div
                  key={item.key}
                  data-index={item.index}
                  ref={virtual.measureElement}
                  className="q-thread-item"
                  style={{ transform: `translateY(${item.start}px)` }}
                >
                  {row.kind === 'loose'
                    ? <div className="q-question">{row.text}</div>
                    : (
                      <Turn
                        turn={row.turn}
                        sep={row.sep}
                        open={folds.get(row.turn.id) ?? false}
                        onToggle={() => { onToggleFold(row.turn.id) }}
                      />
                    )}
                </div>
              )
            })}
          </div>
        )}
      </div>
    </div>
  )
}

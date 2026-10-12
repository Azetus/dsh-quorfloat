// The conversation body: the scroll area, and the window of turns it actually renders.
//
// Windowing: the shell pushes a fresh snapshot on every stream event, so rendering the whole
// transcript each time would be linear in the conversation and spent almost entirely on turns
// nobody can see. Only the rows inside the viewport (plus a few) are mounted, and a row's
// height is measured rather than guessed, because an answer is as tall as its Markdown says.

import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react'
import { api } from '../../api'
import { setBottomPin } from '../../logic/bottom-pin'
import { useVirtualizer } from '@tanstack/react-virtual'
import type { Snapshot } from '../../logic/state'
import { foldTurns, type Turn as TurnModel } from '../../logic/turns'
import { useT } from '../../hooks/useI18n'
import { JumpToBottom } from './JumpToBottom'
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

/**
 * The line a page load is anchored to.
 *
 * Everything here is the form that survives rows arriving *above* the reader: the row's index
 * inside its own block (loose lines and turns are two blocks — see `foldTurns`), so the new
 * index can be computed afterwards without depending on a key that shifts; the row's offset in
 * the items container; and the scroll position that offset was seen at.
 */
interface EarlierAnchor {
  /** Which block the line belongs to. */
  readonly block: 'loose' | 'turn'
  /** Its index within that block. */
  readonly local: number
  /** Its offset in the items container, before the page arrived. */
  readonly start: number
  /** The scroll offset, before the page arrived. */
  readonly scrollTop: number
  /** The block sizes before the page arrived, so the new index can be derived. */
  readonly loose: number
  readonly turns: number
}

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
 * there; once they scroll up, the panel does not yank them back. The rule extends to content
 * arriving *above* them: loading an older page compensates the scroll offset by exactly the
 * height the page added above the retained line, so the words on screen do not move.
 *
 * @param props - snapshot and fold state.
 * @returns the scroll area.
 */
export function Thread({ state, folds, onToggleFold }: ThreadProps) {
  const t = useT()
  const scrollRef = useRef<HTMLDivElement>(null)
  // Whether the reader is at the bottom. Starts true: the first paint should show the end
  // of the conversation, which is what a chat panel is for.
  const follow = useRef(true)
  // The same fact as state, because the way back down is a control the reader has to see.
  const [following, setFollowing] = useState(true)
  /** The box height at the last pin: what a change in it has to be taken out of. */
  const lastHeight = useRef<number | null>(null)
  /** The line a pending older page is anchored to, if one is being held. */
  const anchor = useRef<EarlierAnchor | null>(null)
  /** The last scroll offset this component wrote, so its own writes are not read as the reader's. */
  const written = useRef<number | null>(null)

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
      // express). A loose question line between two turns breaks it.
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

  /**
   * Retain the line the reader is looking at.
   *
   * The first row at or below the viewport's top edge, in the form that survives rows
   * arriving above it: its index inside its own block, its offset, and the scroll position.
   * Called when the reader asks for an older page, and again if they scroll while one is in
   * flight — a page that lands after the reader has moved must not undo the move.
   */
  const captureAnchor = () => {
    const scroll = scrollRef.current
    if (scroll === null) return
    const items = virtual.getVirtualItems()
    const item = items.find(candidate => candidate.start + candidate.size > scroll.scrollTop) ?? items[0]
    if (item === undefined) return
    const loose = item.index < view.loose.length
    anchor.current = {
      block: loose ? 'loose' : 'turn',
      local: loose ? item.index : item.index - view.loose.length,
      start: item.start,
      scrollTop: scroll.scrollTop,
      loose: view.loose.length,
      turns: view.turns.length,
    }
  }

  // Hold the retained line still while older content lands above it.
  //
  // The compensation is the row's own offset delta — the measured height of everything the page
  // put above it — rather than the content height delta: a streaming answer at the tail grows
  // the content at the same moment, and taking that out of the offset would move the reader by
  // exactly the tail's growth. Runs after every commit, because the offsets it reads are the
  // virtualizer's measurements and they settle over a few commits.
  useLayoutEffect(() => {
    const pending = anchor.current
    const scroll = scrollRef.current
    if (pending === null || scroll === null) return
    // A window that got smaller is not the same conversation (a switch, or a reset): the
    // retained line no longer means anything, and holding it would drag the reader somewhere
    // arbitrary.
    if (view.loose.length < pending.loose || view.turns.length < pending.turns) {
      anchor.current = null
      return
    }
    const index = pending.block === 'loose'
      ? pending.local + (view.loose.length - pending.loose)
      : view.loose.length + pending.local + (view.turns.length - pending.turns)
    const offset = virtual.getOffsetForIndex(index)?.[0]
    if (offset === undefined) return
    const target = pending.scrollTop + (offset - pending.start)
    if (Math.abs(scroll.scrollTop - target) >= 0.5) {
      written.current = target
      scroll.scrollTop = target
    }
    // The page has landed and its height is compensated, so the gesture is over: the line is
    // released, and everything after this behaves like ordinary reading (the reader is no
    // longer following the tail — asking for older content ends that — so nothing moves them).
    const grew = view.loose.length > pending.loose || view.turns.length > pending.turns
    if (grew && !loadingEarlier) anchor.current = null
  })

  // Re-read the mounted rows' sizes after every commit.
  //
  // The virtualizer remembers a row's height under its key, and a row can shrink while it is not
  // on screen: an earlier turn's answer settles into its final shape, a fold closes, a card
  // leaves. A row that is not mounted has no resize observer watching it, so a remembered height
  // that is too large stays too large — the spacer below the content carries the error, and the
  // reader gets a band of empty space under the last turn, with the panel pinned at its cap.
  // Measuring the few rows that are actually mounted costs nothing and makes a
  // stale size impossible; the measurement is idempotent, so it cannot feed itself.
  useLayoutEffect(() => {
    const thread = scrollRef.current?.querySelector('#q-thread')
    if (thread === null || thread === undefined) return
    for (const row of thread.querySelectorAll<HTMLElement>('.q-thread-item')) {
      virtual.measureElement(row)
    }
  })

  // The other half of the promise: whoever moves the reader (today, the composer growing as
  // the user types, which comes out of this box while the panel is at its cap) asks for the
  // pin instead of waiting for a render. See `logic/bottom-pin.ts`.
  //
  // Both readers are served, because a box that changes around somebody is two different
  // problems: the one at the bottom wants to stay at the bottom, and the one reading further up
  // wants the words not to move. The box's top edge (and so the first visible line) moves down
  // by the composer's growth while its bottom edge stays put, so keeping a line on screen means
  // scrolling *forward* by that much: `scrollTop += previous - height`. The browser's own
  // scroll anchoring does not cover a container being resized, which is why this is here.
  useEffect(() => {
    setBottomPin(() => {
      const scroll = scrollRef.current
      if (scroll === null) return
      const height = scroll.clientHeight
      const previous = lastHeight.current
      lastHeight.current = height
      if (follow.current) {
        scroll.scrollTop = scroll.scrollHeight
        return
      }
      if (previous !== null && previous !== height) scroll.scrollTop += previous - height
    })
    return () => { setBottomPin(null) }
  }, [])

  const earlier = state.transcript.hasMore === true
  const loadingEarlier = state.transcript.loadingEarlier === true

  return (
    <div
      id="q-thread-scroll"
      className="q-thread-scroll"
      ref={scrollRef}
      onScroll={event => {
        const el = event.currentTarget
        const atBottom = el.scrollTop + el.clientHeight >= el.scrollHeight - 4
        follow.current = atBottom
        // Only on a change: a scroll event per keystroke would otherwise re-render the thread.
        setFollowing(current => (current === atBottom ? current : atBottom))
        // A scroll this component did not write is the reader moving. While a page is on its
        // way that is not a reason to drop the retained line — the page will land after the
        // move, and it would undo it — so the anchor moves with them; with nothing in flight
        // it is released, because holding it would fight the reader.
        const mine = written.current
        if (anchor.current !== null && (mine === null || Math.abs(el.scrollTop - mine) > 1)) {
          if (loadingEarlier && !atBottom) captureAnchor()
          else anchor.current = null
        }
      }}
    >
      {/* The older page's control, arranged like `回到底部`: a zero-height sticky slot, so it
          floats over the first lines without being part of the content the retained offset is
          measured against. A control that took up height would move the reader by its own
          height every time it appeared or left. */}
      {earlier && (
        <div className="q-earlier">
          <button
            type="button"
            disabled={loadingEarlier}
            aria-label={loadingEarlier ? t('thread.loadingEarlier') : t('thread.loadEarlier')}
            onClick={() => {
              // Asking for older content is the reader leaving the tail — the Harness's own
              // `loadEarlier` pauses following for exactly this reason. Without it, the tail
              // pin in the effect above would put them back at the bottom on the next commit
              // and the page they just loaded would arrive off-screen.
              follow.current = false
              setFollowing(false)
              captureAnchor()
              void api.loadEarlier()
            }}
          >
            {loadingEarlier ? t('thread.loadingEarlier') : t('thread.loadEarlier')}
          </button>
        </div>
      )}
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
      {/* The way back down — the harness's own arrangement (`ChatView`): a sticky, zero-height
          slot inside the scroller, so the control floats over the last lines instead of
          scrolling with them, and it exists only while the reader is looking at something
          other than the end. Pressing it re-enters the follow state, which is what makes the
          rest of the panel keep them there. */}
      {!following && (
        <JumpToBottom
          onPress={() => {
            const scroll = scrollRef.current
            follow.current = true
            setFollowing(true)
            if (scroll === null) return
            scroll.scrollTo({
              top: scroll.scrollHeight,
              // The panel's own motion preference, like every other transition it draws.
              behavior: state.settings.reduceMotion ? 'auto' : 'smooth',
            })
          }}
        />
      )}
    </div>
  )
}

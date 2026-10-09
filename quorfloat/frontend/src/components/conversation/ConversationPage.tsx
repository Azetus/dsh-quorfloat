// The conversation page: composer, cards, thread, and the runtime row.
//
// It is `#q-main`, and the settings page replaces it rather than floating over it, so the
// whole body (and the runtime row with it) belongs to the conversation.

import type { RefObject } from 'react'
import type { Snapshot } from '../../lib/state'
import { isBusy } from '../../lib/view-text'
import { Cards } from '../cards/Cards'
import { Composer } from './Composer'
import { RuntimeRow } from './RuntimeRow'
import { Thread } from '../thread/Thread'

/** Props for {@link ConversationPage}. */
export interface ConversationPageProps {
  /** The snapshot. */
  readonly state: Snapshot
  /** Whether the settings page is up (this page is then hidden, not unmounted). */
  readonly hidden: boolean
  /** Which popover is open, if any. */
  readonly openMenu: string | null
  /** Toggle one popover by trigger id. */
  readonly onToggleMenu: (id: string) => void
  /** Close whichever popover is open. */
  readonly onCloseMenu: () => void
  /** The composer's own draft, mirrored here so a rebuild never loses it. */
  readonly draft: string
  /** Record the draft as the user types. */
  readonly onDraft: (text: string) => void
  /** Send, or stop. */
  readonly onSend: () => void
  /** The composer grew or shrank: the panel wants a fresh measurement. */
  readonly onGrow: () => void
  /** The textarea, owned by the app for focus and restore. */
  readonly inputRef: RefObject<HTMLTextAreaElement | null>
  /** The fold state, keyed by turn identity. */
  readonly folds: ReadonlyMap<string, boolean>
  /** Toggle one turn's fold. */
  readonly onToggleFold: (key: string) => void
}

/**
 * Render the conversation page.
 *
 * @param props - snapshot, view state, and handlers.
 * @returns the page.
 */
export function ConversationPage({
  state, hidden, openMenu, onToggleMenu, onCloseMenu, draft, onDraft, onSend, onGrow, inputRef,
  folds, onToggleFold,
}: ConversationPageProps) {
  return (
    <div id="q-main" hidden={hidden}>
      <Composer
        inputRef={inputRef}
        busy={isBusy(state)}
        hasEntries={state.transcript.entries.length > 0}
        draft={draft}
        onDraft={onDraft}
        onSend={onSend}
        onGrow={onGrow}
      />
      <Cards state={state} />
      <Thread state={state} folds={folds} onToggleFold={onToggleFold} />
      <RuntimeRow
        state={state}
        openMenu={openMenu}
        onToggleMenu={onToggleMenu}
        onCloseMenu={onCloseMenu}
      />
    </div>
  )
}

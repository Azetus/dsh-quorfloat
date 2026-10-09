// The panel app: the shell's snapshot in, the whole tree out, and the panel's own view
// state (which page, which popover, the draft, the folds).
//
// Nothing here edits the snapshot: it is the host's, and every decision the user makes
// leaves through `api` so the host stays the authority.

import { useCallback, useEffect, useRef, useState } from 'react'
import { api } from '../api'
import { useDocumentPreferences } from '../hooks/useDocumentPreferences'
import { useFlight } from '../hooks/useFlight'
import { useGlobalKeys } from '../hooks/useGlobalKeys'
import { useViewportHeight } from '../hooks/useViewportHeight'
import { pinToBottom } from '../lib/bottom-pin'
import { usePanelDynamics } from '../hooks/usePanelDynamics'
import { SHADOW_ROOM, type Painted } from '../lib/panel'
import type { Snapshot } from '../lib/state'
import { isBusy, targetWorkspace } from '../lib/view-text'
import { ConversationPage } from './conversation/ConversationPage'
import { Footer } from './footer/Footer'
import { Panel } from './Panel'
import { SettingsPage } from './settings/SettingsPage'
import { TopBar } from './chrome/TopBar'

/** Props for {@link App}. */
export interface AppProps {
  /** The latest snapshot from the shell. */
  readonly state: Snapshot
}

/**
 * Render the panel.
 *
 * @param props - the snapshot.
 * @returns the panel.
 */
export function App({ state }: AppProps) {
  const panelRef = useRef<HTMLElement>(null)
  const inputRef = useRef<HTMLTextAreaElement>(null)
  /** The composer's draft: the DOM's while the user types, read back on send. */
  const draft = useRef('')
  /** The last submitted text, for the "refused → put the words back" rule. */
  const lastSent = useRef<{ text: string; restored: boolean } | null>(null)

  const [page, setPage] = useState<'main' | 'settings'>('main')
  const [openMenu, setOpenMenu] = useState<string | null>(null)
  const [recording, setRecording] = useState(false)
  const [transientError, setTransientError] = useState<string | null>(null)
  const [folds, setFolds] = useState<ReadonlyMap<string, boolean>>(() => new Map())
  // The workspace the user picked for this visit, if they picked one. It is never seeded from
  // the shell's pin: `state.session.createWorkspace` already carries the ladder's answer (P0
  // is that pin, validated), so seeding would only add a second copy that can go stale when
  // the pin moves.
  const [workspaceChoice, setWorkspaceChoice] = useState<string | null>(null)

  useDocumentPreferences(state.settings)

  // The room the native window gives the panel: the product cap is a ceiling, not a
  // promise. It is part of the markup (below) so that everything inside the panel —
  // including the windowed thread, which measures its own viewport — lays out against the
  // real box before any effect runs.
  const viewport = useViewportHeight()
  const available = Math.max(80, viewport - SHADOW_ROOM)
  const maxHeight = Math.min(state.settings.maxHeight, available)

  // What the top bar names: the user's pick, or where the shell would create the conversation.
  const target = targetWorkspace(state, workspaceChoice)

  const arrived: Painted = { page, session: state.transcript.sessionId }
  const { remeasure } = usePanelDynamics(panelRef, state, arrived, available)

  // What a summon puts on screen. The panel is a thing you call for, not a place you
  // resume: the settings page and any open list belonged to the *last* visit, and coming
  // back to them (or to a half-finished chord capture, which would eat the next keystroke)
  // answers a question the user did not ask. The composer is focused so the summon ends
  // where the user's attention is.
  const onPanelShown = useCallback((): void => {
    setPage('main')
    setOpenMenu(null)
    setRecording(false)
    setTransientError(null)
    inputRef.current?.focus()
  }, [])
  const flight = useFlight(panelRef, state, onPanelShown)

  // The refusal recovery rule: the composer clears on send, and when the shell later
  // refuses, the words come back — only into an empty input, and only when it is not the
  // one being typed into.
  useEffect(() => {
    const outcome = state.delivery.prompt.state
    const sent = lastSent.current
    if (outcome === 'failed' && sent !== null && !sent.restored) {
      const input = inputRef.current
      if (draft.current === '' && (input?.value ?? '') === '') {
        draft.current = sent.text
        sent.restored = true
        if (input !== null && document.activeElement !== input) input.value = sent.text
        remeasure()
      }
    }
    if (outcome !== 'failed' && sent !== null && sent.restored) {
      // A new send resets the guard for its own refusal.
      if (outcome === 'sending' || outcome === 'accepted') lastSent.current = null
    }
  })

  const submit = useCallback((): void => {
    const input = inputRef.current
    const text = (input?.value ?? draft.current).trim()
    if (text === '') {
      input?.focus()
      return
    }
    if (isBusy(state)) {
      void api.cancel()
      return
    }
    void api.submit(text, workspaceChoice)
    lastSent.current = { text, restored: false }
    draft.current = ''
    if (input !== null) input.value = ''
    onComposerGrow()
  }, [state, workspaceChoice, remeasure])

  const toggleMenu = useCallback((id: string): void => {
    setOpenMenu(current => (current === id ? null : id))
  }, [])
  const closeMenu = useCallback((): void => { setOpenMenu(null) }, [])
  const toggleSettings = useCallback((): void => {
    setOpenMenu(null)
    setPage(current => (current === 'settings' ? 'main' : 'settings'))
  }, [])
  const backFromSettings = useCallback((): void => {
    setPage('main')
    inputRef.current?.focus()
  }, [])
  const chooseWorkspace = useCallback((workspaceId: string): void => {
    // Choosing a workspace while following a conversation is the new-conversation action.
    if (state.session.following !== null) void api.startNew()
    setWorkspaceChoice(workspaceId)
  }, [state.session.following])
  const toggleFold = useCallback((key: string): void => {
    setFolds(current => {
      const next = new Map(current)
      next.set(key, !(current.get(key) ?? false))
      return next
    })
  }, [])
  const onDraft = useCallback((text: string): void => { draft.current = text }, [])

  // The composer grew or shrank. Re-measure first (the probe moves the panel's box once more),
  // then hold the reader at the bottom: the composer's growth comes out of the thread's box
  // while the panel is capped, and that happens outside React's render cycle — without this the
  // bottom slipped a line away per keystroke and snapped back on the next render.
  const onComposerGrow = useCallback((): void => {
    remeasure()
    pinToBottom()
  }, [remeasure])

  useGlobalKeys({
    recording,
    setRecording,
    setTransientError,
    menuOpen: openMenu !== null,
    closeMenu,
    settingsOpen: page === 'settings',
    backFromSettings,
    hide: flight.requestHide,
    inputRef,
    submit,
  })

  return (
    <Panel panelRef={panelRef} hiding={flight.hiding} maxHeight={maxHeight}>
      <TopBar
        state={state}
        targetWorkspace={target}
        openMenu={openMenu}
        settingsOpen={page === 'settings'}
        onToggleMenu={toggleMenu}
        onCloseMenu={closeMenu}
        onChooseWorkspace={chooseWorkspace}
        onToggleSettings={toggleSettings}
        onHide={flight.requestHide}
      />
      <SettingsPage
        state={state}
        hidden={page !== 'settings'}
        recording={recording}
        onRecord={() => { setRecording(true) }}
        onBack={backFromSettings}
      />
      <ConversationPage
        state={state}
        hidden={page === 'settings'}
        openMenu={openMenu}
        onToggleMenu={toggleMenu}
        onCloseMenu={closeMenu}
        draft={draft.current}
        onDraft={onDraft}
        onSend={submit}
        onGrow={onComposerGrow}
        inputRef={inputRef}
        folds={folds}
        onToggleFold={toggleFold}
      />
      <Footer state={state} transientError={transientError} />
    </Panel>
  )
}

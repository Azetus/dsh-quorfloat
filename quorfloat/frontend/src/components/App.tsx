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
import { usePanelDynamics } from '../hooks/usePanelDynamics'
import { SHADOW_ROOM, type Painted } from '../lib/panel'
import type { Snapshot } from '../lib/state'
import { isBusy } from '../lib/view-text'
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
  // The workspace a new conversation will be created in. Seeded once from the shell's pin
  // — the P0 default — and never written back to the host: the pin is the host's, this is
  // the frontend's own pending choice.
  const [workspaceChoice, setWorkspaceChoice] = useState<string | null>(() => {
    const pinned = state.session.pinnedWorkspace
    return pinned !== null && state.session.workspaces.some(w => w.workspaceId === pinned)
      ? pinned
      : null
  })

  useDocumentPreferences(state.settings)

  // The room the native window gives the panel: the product cap is a ceiling, not a
  // promise. It is part of the markup (below) so that everything inside the panel —
  // including the windowed thread, which measures its own viewport — lays out against the
  // real box before any effect runs.
  const viewport = useViewportHeight()
  const available = Math.max(80, viewport - SHADOW_ROOM)
  const maxHeight = Math.min(state.settings.maxHeight, available)

  const arrived: Painted = { page, session: state.transcript.sessionId }
  const { remeasure } = usePanelDynamics(panelRef, state, arrived, available)

  const focusComposer = useCallback((): void => { inputRef.current?.focus() }, [])
  const flight = useFlight(panelRef, state, focusComposer)

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
    remeasure()
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
    focusComposer()
  }, [focusComposer])
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
        workspaceChoice={workspaceChoice}
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
        onGrow={remeasure}
        inputRef={inputRef}
        folds={folds}
        onToggleFold={toggleFold}
      />
      <Footer state={state} transientError={transientError} />
    </Panel>
  )
}

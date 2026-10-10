// The panel's keyboard rules: a chord while recording, Escape to close or hide, Enter to
// send.

import { useEffect, type RefObject } from 'react'
import { api } from '../api'
import { chordFromEvent, validateChord } from '../lib/hotkey'
import { useT } from './useI18n'

/** Everything the keyboard handler needs from the app. */
export interface GlobalKeyOptions {
  /** Whether the shortcut field is waiting for a chord. */
  readonly recording: boolean
  /** Stop recording (a chord was accepted or rejected). */
  readonly setRecording: (recording: boolean) => void
  /** Show a locally produced message, e.g. why a chord was rejected. */
  readonly setTransientError: (message: string | null) => void
  /** Whether a popover is open. */
  readonly menuOpen: boolean
  /** Close the open popover. */
  readonly closeMenu: () => void
  /** Whether the settings page is up. */
  readonly settingsOpen: boolean
  /** Return to the conversation. */
  readonly backFromSettings: () => void
  /** Hide the panel. */
  readonly hide: () => void
  /** The composer, so Enter only sends from there. */
  readonly inputRef: RefObject<HTMLTextAreaElement | null>
  /** Send the draft (or stop a running turn). */
  readonly submit: () => void
}

/**
 * Install the document-level key handler.
 *
 * @param options - the state and actions the rules need.
 */
export function useGlobalKeys(options: GlobalKeyOptions): void {
  const {
    recording, setRecording, setTransientError, menuOpen, closeMenu, settingsOpen,
    backFromSettings, hide, inputRef, submit,
  } = options
  // The rejected chord's reason is a dictionary key; the status line takes the words.
  const t = useT()

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      // IME composition is the user's, not the panel's: Enter during composition confirms
      // a candidate, it does not send.
      if (event.isComposing || event.keyCode === 229) return

      if (recording) {
        event.preventDefault()
        const chord = chordFromEvent(event)
        if (chord === null) return
        const verdict = validateChord(chord)
        if (!verdict.ok) {
          setTransientError(t(verdict.reason))
          return
        }
        setTransientError(null)
        setRecording(false)
        void api.setPreferences({ hotkey: chord })
        return
      }

      if (event.key === 'Escape') {
        event.preventDefault()
        if (menuOpen) {
          closeMenu()
          return
        }
        if (settingsOpen) {
          backFromSettings()
          return
        }
        hide()
        return
      }

      if (event.key === 'Enter' && !event.shiftKey && event.target === inputRef.current) {
        event.preventDefault()
        submit()
      }
    }
    document.addEventListener('keydown', onKeyDown)
    return () => { document.removeEventListener('keydown', onKeyDown) }
  }, [
    t, recording, setRecording, setTransientError, menuOpen, closeMenu, settingsOpen,
    backFromSettings, hide, inputRef, submit,
  ])
}

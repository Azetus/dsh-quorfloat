// The input row: search mark, the auto-growing textarea, and the send/stop button.
//
// The textarea is deliberately uncontrolled: while the user types, the DOM is the
// source of truth for the draft (the host snapshots arrive on every stream event, and a
// controlled value would fight the caret). The draft is pushed back in only when the box
// is not focused — the same rule the old renderer used.

import { useLayoutEffect, type RefObject } from 'react'
import { Icon } from '../common/Icon'

/** Props for {@link Composer}. */
export interface ComposerProps {
  /** The textarea, owned by the app so the draft survives re-renders. */
  readonly inputRef: RefObject<HTMLTextAreaElement | null>
  /** Whether a turn is in flight (the button becomes stop). */
  readonly busy: boolean
  /** Whether the conversation already has entries (changes the placeholder). */
  readonly hasEntries: boolean
  /** The draft to show while the textarea is not focused. */
  readonly draft: string
  /** Record the draft the user is typing. */
  readonly onDraft: (text: string) => void
  /** Send, or stop while a turn runs. */
  readonly onSend: () => void
  /** The box changed size: the panel has to remeasure (the shell grows the window). */
  readonly onGrow: () => void
}

/**
 * Render the composer.
 *
 * @param props - busy state, placeholder input, and handlers.
 * @returns the composer row.
 */
export function Composer({ inputRef, busy, hasEntries, draft, onDraft, onSend, onGrow }: ComposerProps) {
  // The box's single-line height, matching the CSS (`.q-compose textarea`).
  const base = 34
  const cap = 120

  const resize = (): void => {
    const input = inputRef.current
    if (input === null) return
    input.style.height = `${base}px`
    input.style.height = `${Math.min(input.scrollHeight || base, cap)}px`
    input.style.overflowY = input.scrollHeight > cap ? 'auto' : 'hidden'
  }

  useLayoutEffect(() => {
    const input = inputRef.current
    if (input === null) return
    if (document.activeElement !== input) input.value = draft
    resize()
  })

  return (
    <div className="q-compose">
      <span className="q-search" id="q-search-icon"><Icon name="search" /></span>
      <textarea
        id="q-input"
        ref={inputRef}
        rows={1}
        aria-label="输入问题"
        placeholder={hasEntries ? '继续追问…' : '问点什么…'}
        onInput={event => {
          onDraft(event.currentTarget.value)
          resize()
          onGrow()
        }}
      />
      <button
        type="button"
        id="q-send"
        className="q-submit"
        aria-label={busy ? '停止生成' : '发送问题'}
        onClick={onSend}
      >
        <Icon name={busy ? 'square' : 'arrow-up'} />
      </button>
    </div>
  )
}

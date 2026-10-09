// The footer's left half: the key hints, and the status line when there is something to say.

import { KeyChip } from './KeyChip'

/** Props for {@link StatusLine}. */
export interface StatusLineProps {
  /** The message to show, or null. */
  readonly status: string | null
}

/**
 * Render the key hints and the status.
 *
 * The design's markup is `<kbd>↵</kbd> 发送　<kbd>⇧ ↵</kbd> 换行　<kbd>esc</kbd> 关闭`, with the
 * status appended after a small gap. The status is right-truncated by the CSS, so any
 * message here has to put its action first.
 *
 * @param props - the status message.
 * @returns the status element.
 */
export function StatusLine({ status }: StatusLineProps) {
  return (
    <span id="q-status">
      <KeyChip kind="enter" />
      {' 发送\u3000'}
      <KeyChip kind="shift-enter" />
      {' 换行\u3000'}
      <KeyChip kind="esc" />
      {' 关闭'}
      {status !== null && status !== '' && <span style={{ marginLeft: '10px' }}>{status}</span>}
    </span>
  )
}

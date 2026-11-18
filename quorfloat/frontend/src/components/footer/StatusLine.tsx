// The footer's left half: the key hints, and the status line when there is something to say.

import { useT } from '../../hooks/useI18n'
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
 * status appended after a small gap. The CSS lets a long status wrap rather than truncate
 * it; keep the action first so the visible part is still useful.
 *
 * @param props - the status message.
 * @returns the status element.
 */
export function StatusLine({ status }: StatusLineProps) {
  const t = useT()
  return (
    <span id="q-status">
      <KeyChip kind="enter" />
      {` ${t('status.send')}\u3000`}
      <KeyChip kind="shift-enter" />
      {` ${t('status.newline')}\u3000`}
      <KeyChip kind="esc" />
      {` ${t('status.close')}`}
      {status !== null && status !== '' && <span className="q-status-message">{status}</span>}
    </span>
  )
}

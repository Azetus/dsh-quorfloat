// The way back down: a round control that floats over the last lines while the reader is
// looking at something else, styled after the harness's own (`ChatView`'s `toBottom`: a
// chevron-down in a circular floating button labelled 回到底部).

import { Icon } from '../common/Icon'

/** Props for {@link JumpToBottom}. */
export interface JumpToBottomProps {
  /** Return to the end of the conversation, and stay there. */
  readonly onPress: () => void
}

/**
 * Render the control.
 *
 * @param props - the press handler.
 * @returns the sticky slot and its button.
 */
export function JumpToBottom({ onPress }: JumpToBottomProps) {
  return (
    <div className="q-jump">
      <button type="button" aria-label="回到底部" onClick={onPress}>
        <Icon name="chevron-down" />
      </button>
    </div>
  )
}

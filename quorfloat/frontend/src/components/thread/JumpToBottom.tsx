// The way back down: a round control that floats over the last lines while the reader is
// looking at something else, styled after the harness's own (`ChatView`'s `toBottom`: a
// chevron-down in a circular floating button labelled 回到底部).

import { useT } from '../../hooks/useI18n'
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
  const t = useT()
  return (
    <div className="q-jump">
      <button type="button" aria-label={t('thread.backToBottom')} onClick={onPress}>
        <Icon name="chevron-down" />
      </button>
    </div>
  )
}

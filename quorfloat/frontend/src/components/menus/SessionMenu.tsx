// The conversation picker's menu.

import { api } from '../../api'
import { useT } from '../../hooks/useI18n'
import type { Snapshot } from '../../lib/state'
import { conversationTitle, relativeTime, workspaceTitle } from '../../lib/view-text'
import { MenuHeading } from './MenuHeading'
import { MenuHint } from './MenuHint'
import { MenuOption } from './MenuOption'
import { MenuRow } from './MenuRow'
import { MenuSeparator } from './MenuSeparator'
import { PinButton } from './PinButton'

/** Props for {@link SessionMenu}. */
export interface SessionMenuProps {
  /** The snapshot. */
  readonly state: Snapshot
  /** The workspace the next conversation would use, for the heading's context. */
  readonly targetWorkspace: string | null
  /** Close the menu after an action that should close it. */
  readonly onClose: () => void
}

/**
 * Render the conversation list.
 *
 * Blank conversations are not offered (a fresh session with no turn is not something to
 * switch *to*), and the pin is the only persistent choice: it decides which conversation
 * the next summon opens.
 *
 * @param props - snapshot and the close handler.
 * @returns the menu contents.
 */
export function SessionMenu({ state, targetWorkspace, onClose }: SessionMenuProps) {
  const t = useT()
  const pinned = state.session.pinned
  const conversations = state.session.conversations.filter(c => !c.blank)
  return (
    <>
      <MenuHeading title={t('session.heading')} detail={t(workspaceTitle(state, targetWorkspace))} />
      <MenuOption
        label={t('session.new')}
        selected={state.session.following === null}
        symbol="plus"
        onPick={() => {
          onClose()
          void api.startNew()
        }}
      />
      <MenuSeparator />
      {conversations.map(conversation => {
        const title = conversationTitle(conversation)
        const isPinned = pinned === conversation.sessionId
        return (
          <MenuRow key={conversation.sessionId}>
            <MenuOption
              label={title}
              selected={state.session.following?.sessionId === conversation.sessionId}
              description={t(relativeTime(conversation.updatedAt))}
              symbol="message-square"
              onPick={() => {
                onClose()
                void api.selectSession(conversation.sessionId)
              }}
            />
            <PinButton
              name={t('session.pinName', { name: title })}
              pinned={isPinned}
              onPick={() => {
                onClose()
                void api.pin(isPinned ? null : conversation.sessionId)
              }}
            />
          </MenuRow>
        )
      })}
      {conversations.length === 0 && <MenuHint text={t('session.none')} />}
      <MenuSeparator />
      <MenuHint text={pinned !== null ? t('session.pinnedHint') : t('session.unpinnedHint')} />
    </>
  )
}

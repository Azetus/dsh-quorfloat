// The text a component shows, derived from one snapshot. Components call these
// directly instead of drilling props.
//
// Nothing here writes a sentence: the helpers return a `Text` (host data to show verbatim,
// or a dictionary key plus its data), and the component resolves it with `t`. That is what
// keeps the wording in `logic/i18n/zh.ts` / `en.ts` and both languages honest.

import { msg, raw, type Text } from './i18n'
import { workspaceLabel } from './labels'
import type { Conversation, Snapshot } from './state'

/**
 * The workspace the *next* conversation would start in.
 *
 * The user's own pick for this visit wins; otherwise the shell's answer — the ladder it
 * applies when the panel has no choice of its own (P0 the window's last choice, then P3 the
 * most recently active workspace). `null` means nothing is known and only the user can
 * decide, which is what the field's `workspace.choose` fallback is for.
 *
 * @param state - the snapshot.
 * @param chosen - the workspace the user picked in this visit, if any.
 * @returns the workspace id to show, or null.
 */
export function targetWorkspace(state: Snapshot, chosen: string | null): string | null {
  return chosen ?? state.session.createWorkspace
}

/**
 * What the panel calls the current workspace.
 *
 * @param state - the snapshot.
 * @param workspaceChoice - the frontend's own preselect for a new conversation.
 * @returns the workspace title or path tail as host data, or a fallback key.
 */
export function workspaceTitle(state: Snapshot, workspaceChoice: string | null): Text {
  const following = state.session.following
  if (following !== null) {
    const byCwd = state.session.conversations.find(c => c.sessionId === following.sessionId)?.cwd
    if (byCwd !== undefined && byCwd !== null) {
      const match = state.session.workspaces.find(w => w.path === byCwd)
      if (match !== undefined) return raw(workspaceLabel(match.title, match.path))
    }
    return following.label !== null ? raw(following.label) : msg('workspace.heading')
  }
  const chosen = state.session.workspaces.find(w => w.workspaceId === workspaceChoice)
  return chosen !== undefined ? raw(workspaceLabel(chosen.title, chosen.path)) : msg('workspace.choose')
}

/**
 * What the panel calls the current conversation.
 *
 * @param state - the snapshot.
 * @returns the host's title or the id's short form as host data, or the new-conversation key.
 */
export function sessionTitle(state: Snapshot): Text {
  const following = state.session.following
  if (following === null) return msg('session.new')
  const conversation = state.session.conversations.find(c => c.sessionId === following.sessionId)
  return raw(conversation?.title ?? following.sessionId.slice(0, 13))
}

/**
 * The status line's message: the first thing the user needs to know, if any.
 *
 * Order is deliberate — a delivery failure outranks a setting failure, which outranks a
 * hotkey problem.
 *
 * @param state - the snapshot.
 * @param transientError - a locally produced message (a rejected chord).
 * @returns the message, or null when there is nothing to say.
 */
export function statusText(state: Snapshot, transientError: string | null): string | null {
  const prompt = state.delivery.prompt.status
  const cancel = state.delivery.cancel.status
  if (prompt !== null && prompt !== undefined) return prompt
  if (cancel !== null && cancel !== undefined) return cancel
  if (state.session.settingFailure !== null) return state.session.settingFailure
  if (state.session.createFailure !== null) return state.session.createFailure
  if (state.hotkey.reason !== null) return state.hotkey.reason
  return transientError
}

/**
 * Whether a turn is in flight.
 *
 * @param state - the snapshot.
 * @returns true while streaming or while a prompt is being admitted.
 */
export function isBusy(state: Snapshot): boolean {
  return (
    state.transcript.turnActive ||
    state.transcript.streaming ||
    state.delivery.prompt.state === 'sending'
  )
}

/**
 * A conversation's age, as data the dictionary can phrase.
 *
 * The clock reading is host data (`14:05` reads the same in both languages), so it comes
 * back raw; the other three cases are keys with their number.
 *
 * @param updatedAt - the host's timestamp.
 * @returns a short relative label token.
 */
export function relativeTime(updatedAt: number): Text {
  const now = Date.now()
  const diff = now - updatedAt
  if (diff < 60_000) return msg('time.now')
  if (diff < 3_600_000) return msg('time.minutesAgo', { minutes: Math.floor(diff / 60_000) })
  const date = new Date(updatedAt)
  const today = new Date()
  if (date.toDateString() === today.toDateString()) {
    return raw(`${String(date.getHours()).padStart(2, '0')}:${String(date.getMinutes()).padStart(2, '0')}`)
  }
  return msg('time.date', { month: date.getMonth() + 1, day: date.getDate() })
}

/**
 * What the session picker calls one conversation.
 *
 * @param conversation - the host's summary.
 * @returns its title, or the start of its id when the host has none.
 */
export function conversationTitle(conversation: Conversation): string {
  return conversation.title ?? conversation.sessionId.slice(0, 13)
}

/**
 * Whether the top bar marks the current workspace choice as pinned.
 *
 * @param state - the snapshot.
 * @param workspaceChoice - the frontend's own preselect.
 * @returns true when the pin belongs to the workspace now shown.
 */
export function pinnedWorkspaceMark(state: Snapshot, workspaceChoice: string | null): boolean {
  return (
    state.session.pinnedWorkspace !== null &&
    workspaceChoice === state.session.pinnedWorkspace
  )
}

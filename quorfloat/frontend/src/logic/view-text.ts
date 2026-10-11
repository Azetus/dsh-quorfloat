// The text a component shows, derived from one snapshot. Components call these
// directly instead of drilling props.
//
// Nothing here writes a sentence: the helpers return a `Text` (host data to show verbatim,
// or a dictionary key plus its data), and the component resolves it with `t`. That is what
// keeps the wording in `logic/i18n/zh.ts` / `en.ts` and both languages honest.

import { msg, raw, type Text } from './i18n'
import { workspaceLabel } from './labels'
import { isUngrouped, workspaceOfSession } from './scope'
import type { Conversation, Snapshot } from './state'

/**
 * The workspace the *next* conversation would start in.
 *
 * The user's own pick for this visit wins; otherwise the shell's answer — the ladder it
 * applies when the panel has no choice of its own (P0 the workspace pin, then P3 the
 * most recently active workspace, then the host's first-use default). `null` means nothing is
 * known and only the user can decide, which is what the field's fallbacks are for.
 *
 * @param state - the snapshot.
 * @param chosen - the workspace the user picked in this visit, if any.
 * @returns the workspace id to show, or null.
 */
export function targetWorkspace(state: Snapshot, chosen: string | null): string | null {
  return chosen ?? state.session.createWorkspace
}

/**
 * What the panel calls the workspace it is about.
 *
 * Two different questions, and the snapshot decides which one is being asked:
 *
 * - **a conversation is on screen** — the field is the *fact*. The workspace is the one the
 *   registry's membership account places that conversation in (`logic/scope.ts`); a
 *   conversation no workspace accounts for is **Ungrouped**, which is a label and not a target;
 *   a conversation the list has not caught up with yet keeps its own directory name, because
 *   "Ungrouped" is a claim nobody has made.
 * - **no conversation is on screen** — the field is the *decision*: the user's pick for this
 *   visit, or the shell's ladder, and when neither can answer, the host's refusal to provide a
 *   default is what sends the user to the Harness.
 *
 * @param state - the snapshot.
 * @param target - the resolved workspace id for a new conversation (the user's pick, or the shell's).
 * @returns the workspace title or path tail as host data, or a fallback key.
 */
export function workspaceTitle(state: Snapshot, target: string | null): Text {
  const following = state.session.following
  if (following !== null) {
    const scope = workspaceOfSession(state, following.sessionId)
    if (scope !== null) return raw(workspaceLabel(scope.title, scope.path))
    if (isUngrouped(state, following.sessionId)) return msg('workspace.ungrouped')
    // Visible in neither list yet: a conversation that was just created. Its own directory
    // name is the honest thing to show until the registry's account arrives.
    return following.label !== null ? raw(following.label) : msg('workspace.heading')
  }
  const chosen = state.session.workspaces.find(workspace => workspace.workspaceId === target)
  if (chosen !== undefined) return raw(workspaceLabel(chosen.title, chosen.path))
  if (state.session.defaultWorkspace.state === 'failed') return msg('workspace.needHost')
  return msg('workspace.choose')
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
 * creation failure, which outranks the explanation of a lost pin, which outranks a hotkey
 * problem. The pin notice sits below the failures because those describe something the user
 * just asked for and did not get, while the notice explains something the shell did *for* them.
 *
 * @param state - the snapshot.
 * @param transientError - a locally produced message (a rejected chord).
 * @returns the message token, or null when there is nothing to say.
 */
export function statusText(state: Snapshot, transientError: string | null): Text | null {
  const prompt = state.delivery.prompt.status
  const cancel = state.delivery.cancel.status
  if (prompt !== null && prompt !== undefined) return raw(prompt)
  if (cancel !== null && cancel !== undefined) return raw(cancel)
  if (state.session.settingFailure !== null) return raw(state.session.settingFailure)
  if (state.session.createFailure !== null) return raw(state.session.createFailure)
  const notice = pinNoticeText(state)
  if (notice !== null) return notice
  if (state.hotkey.reason !== null) return raw(state.hotkey.reason)
  return transientError === null ? null : raw(transientError)
}

/**
 * The sentence a `session.pinNotice` reason stands for.
 *
 * The wire carries a *reason* (`sessionGone` / `workspaceGone`) and never a sentence: the
 * panel's words live in its dictionaries, and both languages get them from the same place. An
 * unknown reason — a newer shell — says nothing rather than inventing a claim about the user's
 * pin.
 *
 * @param state - the snapshot.
 * @returns the dictionary key with its data, or null when there is no notice.
 */
export function pinNoticeText(state: Snapshot): Text | null {
  const notice = state.session.pinNotice
  if (notice === null || notice === undefined) return null
  switch (notice.kind) {
    case 'sessionGone':
      return msg('session.pinNotice.sessionGone')
    case 'workspaceGone':
      return msg('session.pinNotice.workspaceGone')
    default:
      return null
  }
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

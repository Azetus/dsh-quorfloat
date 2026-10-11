// Which workspace a conversation belongs to, and which conversations a workspace holds.
//
// **Membership, not the directory.** The Harness's workspace registry keeps its own account of
// which conversations belong to a workspace (`sessionIds` on each row), and it is the only
// thing that knows: a conversation can be moved out of a workspace without its `cwd` changing,
// so comparing paths is silently wrong in exactly the case the account exists to describe.
//
// The account is *intersected with the visible session list*, for two reasons:
// - it can name a conversation the list no longer shows, and offering a scope for a row that is
//   not there is not a fact anybody can check;
// - a subagent conversation is dropped from the list at the host boundary (`listSessions`), so
//   it must not create a scope here either.
//
// Deliberately pure and free of React: the "which workspace is this" question is asked by the
// top bar and by the pickers, and the answer must not depend on which of them asked.

import type { Snapshot, Workspace } from './state'

/**
 * Whether the panel is drawing this conversation at all.
 *
 * @param state - the snapshot.
 * @param sessionId - the conversation to look for.
 * @returns true when the host's session list carries it.
 */
export function isVisibleSession(state: Snapshot, sessionId: string): boolean {
  return state.session.conversations.some(conversation => conversation.sessionId === sessionId)
}

/**
 * The workspace whose account names one conversation.
 *
 * A conversation that is not in the visible list has no scope, even when some workspace's
 * account still names it: the first half of the intersection.
 *
 * @param state - the snapshot.
 * @param sessionId - the conversation to place.
 * @returns the owning workspace, or `null` when no visible workspace's account names it.
 */
export function workspaceOfSession(state: Snapshot, sessionId: string): Workspace | null {
  if (!isVisibleSession(state, sessionId)) return null
  return state.session.workspaces.find(workspace => workspace.sessionIds.includes(sessionId)) ?? null
}

/**
 * Whether a conversation is shown but belongs to no workspace.
 *
 * The third state, beside "in a workspace" and "not visible at all": the panel draws such a row
 * under nothing, and the workspace field names that fact (Ungrouped) rather than guessing a
 * directory. It is a *label*, never a target: "Ungrouped" is not a workspace to create in, and
 * it does not appear in the picker.
 *
 * @param state - the snapshot.
 * @param sessionId - the conversation to place.
 * @returns true only when the conversation is visible and unaccounted for.
 */
export function isUngrouped(state: Snapshot, sessionId: string): boolean {
  return isVisibleSession(state, sessionId) && workspaceOfSession(state, sessionId) === null
}

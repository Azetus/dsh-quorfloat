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
// The same derivation answers a second question — *what is the conversation list listing?* —
// and the automatic answer is `fieldScope`: the workspace the workspace field is showing, which is the
// fact when a conversation is pinned open and the choice (the ladder's answer) when none is.
// A view-only browsing choice can override this answer for both the field and the list.
//
// Deliberately pure and free of React: the "which workspace is this" question is asked by the
// top bar and by the pickers, and the answer must not depend on which of them asked. The one
// piece of state around the scope — remembering the last resolved value across the frames in
// which the field cannot answer — lives in `hooks/useSessionScope.ts`, not here.

import type { Conversation, Snapshot, Workspace } from './state'

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
 * the picker's synthetic entry only browses this scope.
 *
 * @param state - the snapshot.
 * @param sessionId - the conversation to place.
 * @returns true only when the conversation is visible and unaccounted for.
 */
export function isUngrouped(state: Snapshot, sessionId: string): boolean {
  return isVisibleSession(state, sessionId) && workspaceOfSession(state, sessionId) === null
}

/**
 * What a conversation list is showing: one workspace's conversations, or the ungrouped ones.
 *
 * `ungrouped` is a state, not an entity: it names the absence of an account, and it is never a
 * creation target (see `isUngrouped`).
 */
export type Scope =
  | { readonly kind: 'workspace'; readonly workspaceId: string }
  | { readonly kind: 'ungrouped' }

/** The ungrouped scope. One value, so scope comparisons can be by identity or by {@link sameScope}. */
const UNGROUPED: Scope = { kind: 'ungrouped' }

/**
 * Whether two scopes name the same thing. `null` is its own answer — no scope yet.
 *
 * @param a - one scope.
 * @param b - the other.
 * @returns true when both name the same workspace, both are ungrouped, or both are null.
 */
export function sameScope(a: Scope | null, b: Scope | null): boolean {
  if (a === null || b === null) return a === b
  if (a.kind === 'ungrouped' || b.kind === 'ungrouped') return a.kind === b.kind
  return a.workspaceId === b.workspaceId
}

/**
 * The scope one conversation sits in.
 *
 * Three answers, all derived from the membership account above: the workspace that accounts
 * for it, `ungrouped` when none does, or `null` when the visible list has not caught up with
 * the conversation yet. The last one is real — a conversation just created is in neither list,
 * and claiming either state for it would be a guess.
 *
 * @param state - the snapshot.
 * @param sessionId - the conversation to place.
 * @returns its scope, or null while only its directory name is known.
 */
export function sessionScope(state: Snapshot, sessionId: string): Scope | null {
  const owner = workspaceOfSession(state, sessionId)
  if (owner !== null) return { kind: 'workspace', workspaceId: owner.workspaceId }
  return isUngrouped(state, sessionId) ? UNGROUPED : null
}

/**
 * The scope a workspace id names, when the snapshot knows that workspace.
 *
 * @param state - the snapshot.
 * @param workspaceId - the id to look up, or null for "nothing chosen".
 * @returns the workspace scope, or null when the id names nothing in this snapshot
 *   (the workspace list has not arrived, or the workspace is gone).
 */
export function workspaceScope(state: Snapshot, workspaceId: string | null): Scope | null {
  if (workspaceId === null) return null
  const known = state.session.workspaces.some(workspace => workspace.workspaceId === workspaceId)
  return known ? { kind: 'workspace', workspaceId } : null
}

/**
 * The scope the **workspace field** is showing — the panel's one answer to "which workspace is
 * this?".
 *
 * It mirrors the field's own two cases exactly (`view-text.ts`'s `workspaceTitle`, which draws
 * its label from this same call):
 *
 * - **a conversation is open** (the session pin is armed): the field is the *fact*, so the scope
 *   is that conversation's own scope — its workspace, or ungrouped;
 * - **no conversation is open**: the field is the *decision*, so the scope is the workspace the
 *   field names (the user's choice for this visit, or the shell's ladder — the caller passes
 *   whichever `target` it shows).
 *
 * `null` means the field has not resolved to a concrete workspace: a conversation the visible
 * list has not caught up with, a workspace id the snapshot does not know yet, or no decision at
 * all (the field then shows the "choose a workspace" / "no workspace yet" prompt). The list must
 * not be narrowed to nothing in that case; see `hooks/useSessionScope.ts` for what it does.
 *
 * @param state - the snapshot.
 * @param target - the resolved workspace id for a new conversation (the user's pick, or the shell's).
 * @returns the scope the field is showing, or null while it is unresolved.
 */
export function fieldScope(state: Snapshot, target: string | null): Scope | null {
  const following = state.session.following
  if (following !== null) return sessionScope(state, following.sessionId)
  return workspaceScope(state, target)
}

/**
 * The conversations a scope lists, in the snapshot's order.
 *
 * The visible, non-blank conversations, narrowed to the scope's members. Membership is the
 * registry's account (`sessionIds`), never the directory: a conversation moved out of a
 * workspace keeps its `cwd`, and listing it under the old workspace would be the silently wrong
 * case that account exists to describe. A `null` scope lists everything — the field has not
 * resolved, so there is nothing to narrow to yet.
 *
 * @param state - the snapshot.
 * @param scope - the scope to list, or null for every conversation.
 * @returns the conversations to offer.
 */
export function sessionsInScope(state: Snapshot, scope: Scope | null): Conversation[] {
  const shown = state.session.conversations.filter(conversation => !conversation.blank)
  if (scope === null) return shown
  if (scope.kind === 'ungrouped') {
    return shown.filter(conversation => isUngrouped(state, conversation.sessionId))
  }
  return shown.filter(
    conversation => workspaceOfSession(state, conversation.sessionId)?.workspaceId === scope.workspaceId,
  )
}

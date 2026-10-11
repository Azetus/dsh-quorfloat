//! Which workspace a new conversation is created in.
//!
//! The rule is a priority ladder, and the priorities came from the design rather than from
//! what was easy to reach:
//!
//! | rung | value | where it comes from |
//! |---|---|---|
//! | P0 | the directory this window last chose | our own pinned workspace, **validated** against the host's list |
//! | P1 | the running conversation's directory | the session list's `running` flag |
//! | P2 | the first-use default | left to the host, by asking for no particular workspace |
//! | P3 | the most recently active workspace | the list, aggregated by its own members' newest conversation |
//! | P4 | ask the user | the picker, when there is nothing to go on at all |
//!
//! P0 is validated because a pinned workspace can stop existing — the Harness can forget a
//! directory between two runs — and a creation aimed at a workspace that is gone is a failure
//! the user cannot act on. P2 is not a value at all: it is the absence of one, which the host
//! answers by creating or choosing its default.
//!
//! **Membership, not paths.** Which conversations belong to a workspace is the registry's own
//! `sessionIds` account, carried on every `workspaces/list` row, and no path comparison remains
//! on any hot path. (A path-equality fallback for a conversation the registry did not account
//! for had no callers and was deleted on 2026-10-11. If a path comparison is ever unavoidable:
//! normalise `\` and `/`, trim trailing separators, and fold case under `cfg!(windows)`.)

use crate::app::session::follow::{SessionSummary, Workspace};

/// Which workspace to create a conversation in.
///
/// @param pinned - the workspace this window last chose, if any.
/// @param current - the workspace of the conversation on screen, if there is one.
/// @param running - whether that conversation has a turn in flight.
/// @param workspaces - the host's list.
/// @param sessions - the host's conversations, which is what P3 is computed from.
/// @returns the workspace to ask for, or `None` to let the host use its default.
#[must_use]
pub fn to_create_in(
    pinned: Option<&str>,
    current: Option<&str>,
    running: bool,
    workspaces: &[Workspace],
    sessions: &[SessionSummary],
) -> Option<String> {
    // P0: what this window last chose, if it still exists.
    if let Some(pinned) = pinned.filter(|id| workspaces.iter().any(|workspace| workspace.workspace_id == *id)) {
        return Some(pinned.to_owned());
    }
    // P1: where the conversation on screen is, while it is still working. A turn in flight is
    // the strongest evidence of what the user is doing right now.
    if running {
        if let Some(current) = current.filter(|id| workspaces.iter().any(|workspace| workspace.workspace_id == *id)) {
            return Some(current.to_owned());
        }
    }
    // P3: the workspace with the newest conversation in it, which is what the Harness window
    // itself would call the recent one.
    recent(workspaces, sessions)
}

/// The workspace whose newest conversation is the newest overall.
///
/// The score is the newest `updatedAt` among the workspace's own members — the `sessionIds` the
/// registry accounts to it, **not** every conversation whose path happens to be under the same
/// directory: a conversation can be moved out of a workspace without its directory changing, and
/// the registry is what knows. A workspace with no members scores its own `createdAt`, so a
/// directory that was just added is still a candidate. Replacement is strictly greater, so two
/// workspaces with the same score keep the first the host listed.
///
/// @param workspaces - the host's list.
/// @param sessions - the host's conversations.
/// @returns that workspace's id, or `None` when nothing can be told from the lists.
#[must_use]
pub fn recent(workspaces: &[Workspace], sessions: &[SessionSummary]) -> Option<String> {
    let mut best: Option<(String, i64)> = None;
    for workspace in workspaces {
        let newest = sessions
            .iter()
            .filter(|session| workspace.session_ids.iter().any(|id| id == &session.session_id))
            .map(|session| session.updated_at)
            .max()
            .unwrap_or(workspace.created_at);
        if best.as_ref().is_none_or(|(_, score)| newest > *score) {
            best = Some((workspace.workspace_id.clone(), newest));
        }
    }
    best.map(|(workspace_id, _)| workspace_id)
}

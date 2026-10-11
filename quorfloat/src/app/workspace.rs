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
//! | P3 | the most recently active workspace | the list, aggregated by newest conversation |
//! | P4 | ask the user | the picker, when there is nothing to go on at all |
//!
//! P0 is validated because a pinned workspace can stop existing — the Harness can forget a
//! directory between two runs — and a creation aimed at a workspace that is gone is a failure
//! the user cannot act on. P2 is not a value at all: it is the absence of one, which the host
//! answers by creating or choosing its default.

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
/// @param workspaces - the host's list.
/// @param sessions - the host's conversations.
/// @returns that workspace's id, or `None` when nothing can be told from the lists.
#[must_use]
pub fn recent(workspaces: &[Workspace], sessions: &[SessionSummary]) -> Option<String> {
    workspaces
        .iter()
        .filter_map(|workspace| {
            let newest = sessions
                .iter()
                .filter(|session| session.cwd.as_deref().is_some_and(|cwd| same_directory(&workspace.path, cwd)))
                .map(|session| session.updated_at)
                .max()?;
            Some((workspace.workspace_id.clone(), newest))
        })
        .max_by_key(|(_, newest)| *newest)
        .map(|(workspace_id, _)| workspace_id)
}

/// Whether two paths name the same directory.
///
/// @param left - one path, as the host wrote it.
/// @param right - the other.
/// @returns whether they are the same directory, trailing separators aside.
#[must_use]
pub fn same_directory(left: &str, right: &str) -> bool {
    let trim = |path: &str| path.trim_end_matches(['/', '\\']).to_owned();
    !left.is_empty() && trim(left) == trim(right)
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Two workspaces and three conversations, one of them newer in the second workspace.
    fn lists() -> (Vec<Workspace>, Vec<SessionSummary>) {
        let workspaces = vec![
            Workspace {
                workspace_id: "ws-old".to_owned(),
                title: "old".to_owned(),
                path: "/work/old".to_owned(),
            },
            Workspace {
                workspace_id: "ws-new".to_owned(),
                title: "new".to_owned(),
                path: "/work/new".to_owned(),
            },
        ];
        let sessions = vec![
            summary("/work/old", 10),
            summary("/work/old", 20),
            summary("/work/new", 30),
        ];
        (workspaces, sessions)
    }

    /// One conversation, built the way the follow layer builds them.
    fn summary(cwd: &str, updated_at: i64) -> SessionSummary {
        let value = json!({"sessionId": format!("session-{updated_at}"), "updatedAt": updated_at, "cwd": cwd});
        let result = json!({"items": [value]});
        // The parser is the follow layer's, so this test cannot drift from what it produces.
        crate::app::session::follow::parse_sessions(&result).remove(0)
    }

    #[test]
    fn the_workspace_this_window_last_chose_wins() {
        let (workspaces, sessions) = lists();
        assert_eq!(
            to_create_in(Some("ws-old"), Some("ws-new"), true, &workspaces, &sessions).as_deref(),
            Some("ws-old"),
            "P0 outranks everything, including the conversation on screen",
        );
    }

    #[test]
    fn a_pinned_workspace_that_no_longer_exists_is_not_used() {
        let (workspaces, sessions) = lists();
        // The harness can forget a directory between two runs; aiming at it would be a failure
        // the user cannot act on.
        assert_eq!(
            to_create_in(Some("ws-gone"), None, false, &workspaces, &sessions).as_deref(),
            Some("ws-new"),
            "so the ladder falls through to P3",
        );
    }

    #[test]
    fn a_running_conversation_speaks_for_its_workspace() {
        let (workspaces, sessions) = lists();
        assert_eq!(
            to_create_in(None, Some("ws-old"), true, &workspaces, &sessions).as_deref(),
            Some("ws-old"),
            "P1 beats P3 while a turn is in flight",
        );
        // Idle, the same conversation does not: the newest one is a better guess about what
        // the user is doing next.
        assert_eq!(
            to_create_in(None, Some("ws-old"), false, &workspaces, &sessions).as_deref(),
            Some("ws-new"),
        );
    }

    #[test]
    fn with_nothing_to_go_on_the_host_decides() {
        // P2 and P4 are the host's: asking for no particular workspace is how the panel says
        // "you choose", and an empty registry is the host's cue to make its default one.
        assert_eq!(to_create_in(None, None, false, &[], &[]), None);
        assert_eq!(to_create_in(None, None, false, &lists().0, &[]), None, "no conversations, no evidence");
        assert_eq!(to_create_in(Some("ws-gone"), None, false, &[], &[]), None);
    }

    #[test]
    fn the_most_recent_workspace_is_the_one_with_the_newest_conversation() {
        let (workspaces, sessions) = lists();
        assert_eq!(recent(&workspaces, &sessions).as_deref(), Some("ws-new"));
        // A conversation in a subdirectory belongs to no workspace: guessing by prefix would
        // pick a workspace the conversation is not in.
        let nested = vec![summary("/work/new/src", 99)];
        assert_eq!(recent(&workspaces, &nested), None);
    }
}

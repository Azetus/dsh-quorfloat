//! The optional raw-frame dump: every conversation notification, verbatim.
//!
//! Separate from [`crate::marker`] on purpose. The marker records *what this process
//! decided* — short, human-readable breadcrumbs, safe to leave on. This records what
//! the host *sent*, including whatever the conversation happened to contain, and it
//! exists for one reason: the shapes the renderer must handle cannot be guessed from a
//! schema. `session/event`'s `type` is a free-form string in the contract, so the only
//! authoritative list of kinds and payloads is the traffic itself.
//!
//! Its rules are the marker's rules, plus one:
//!
//! - **Off unless asked for**, by `DSH_QUORFLOAT_DUMP`.
//! - **Never load-bearing**: every failure is dropped.
//! - **It writes conversation text to disk**, which is why it is a development switch
//!   and not something the host ever sets for a user.
//!
//! Lines are NDJSON — one object per notification — so a capture can be read with
//! `jq`, replayed in a test, or diffed between two harness versions.

use std::path::PathBuf;

use serde_json::Value;

/// A raw-frame sink, or nothing when the environment does not name one.
#[derive(Debug, Clone, Default)]
pub struct Dump {
    path: Option<PathBuf>,
}

impl Dump {
    /// A dump writing to an explicit path.
    ///
    /// @param path - where lines are appended.
    #[must_use]
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: Some(path.into()) }
    }

    /// Read the dump path from `DSH_QUORFLOAT_DUMP`.
    ///
    /// @returns a dump that writes nothing when the variable is unset or blank.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            path: std::env::var("DSH_QUORFLOAT_DUMP")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .map(PathBuf::from),
        }
    }

    /// Whether anything would be written.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.path.is_some()
    }

    /// Append one notification, if a dump was requested.
    ///
    /// Opened, written and closed per line for the same reason the marker does it: the
    /// interesting captures end with the process being killed.
    ///
    /// @param method - the notification method, kept outside the payload so a capture
    ///   can be filtered by kind without parsing the body.
    /// @param params - the notification parameters, verbatim.
    pub fn record(&self, method: &str, params: Option<&Value>) {
        let Some(path) = &self.path else { return };
        let line = serde_json::json!({
            "at": crate::rpc::now_millis(),
            "method": method,
            "params": params.cloned().unwrap_or(Value::Null),
        });
        use std::io::Write as _;
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(file, "{line}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_disabled_dump_writes_nothing() {
        let dump = Dump::default();
        assert!(!dump.is_enabled());
        dump.record("session/event", Some(&serde_json::json!({"seq": 1})));
    }

    #[test]
    fn a_dump_writes_one_json_object_per_notification() {
        let path = std::env::temp_dir().join(format!("quorfloat-dump-test-{}.ndjson", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let dump = Dump { path: Some(path.clone()) };
        dump.record("session/snapshot", Some(&serde_json::json!({"cursor": 4})));
        dump.record("session/stream", None);
        let written = std::fs::read_to_string(&path).expect("the dump file exists");
        let _ = std::fs::remove_file(&path);

        let lines: Vec<Value> = written.lines().map(|line| serde_json::from_str(line).expect("valid JSON")).collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["method"], "session/snapshot");
        assert_eq!(lines[0]["params"]["cursor"], 4);
        // A notification with no parameters is still a fact worth recording, and it is
        // recorded as `null` rather than skipped.
        assert_eq!(lines[1]["params"], Value::Null);
        assert!(lines[0]["at"].as_i64().is_some(), "each line carries a time");
    }

    #[test]
    fn an_unwritable_path_is_survivable() {
        let dump = Dump { path: Some(PathBuf::from("/definitely/not/a/real/directory/dump.ndjson")) };
        dump.record("session/event", Some(&serde_json::json!({"seq": 1})));
    }
}

//! The optional raw-frame dump: every conversation notification, verbatim.
//!
//! Separate from [`crate::runtime::diag::marker`] on purpose. The marker records *what
//! this process decided* — short, human-readable breadcrumbs, safe to leave on. This
//! records what the host *sent*, including whatever the conversation happened to
//! contain, and it exists for one reason: the shapes the renderer must handle cannot be
//! guessed from a schema. `session/event`'s `type` is a free-form string in the
//! contract, so the only authoritative list of kinds and payloads is the traffic itself.
//!
//! The file is [`Append`]; this type is its line format, which is NDJSON — one object
//! per notification — so a capture can be read with `jq`, replayed in a test, or diffed
//! between two harness versions.
//!
//! **It writes conversation text to disk**, which is why it is a development switch and
//! not something the host ever sets for a user.

use serde_json::Value;

use crate::runtime::diag::append::Append;

/// The environment variable that names the dump file.
pub const DUMP_ENV: &str = "DSH_QUORFLOAT_DUMP";

/// A raw-frame sink, or nothing when the environment does not name one.
#[derive(Debug, Clone, Default)]
pub struct Dump {
    sink: Append,
}

impl Dump {
    /// Read the dump path from [`DUMP_ENV`].
    ///
    /// @returns a dump that writes nothing when the variable is unset or blank.
    #[must_use]
    pub fn from_env() -> Self {
        Self { sink: Append::from_env(DUMP_ENV) }
    }

    /// Whether anything would be written.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.sink.is_enabled()
    }

    /// Append one notification, if a dump was requested.
    ///
    /// @param method - the notification method, kept outside the payload so a capture
    ///   can be filtered by kind without parsing the body.
    /// @param params - the notification parameters, verbatim.
    pub fn record(&self, method: &str, params: Option<&Value>) {
        if !self.sink.is_enabled() {
            // The JSON is only built when it is going to be written: a dump that is off
            // must not cost a serialisation per notification.
            return;
        }
        let line = serde_json::json!({
            "at": crate::ipc::rpc::now_millis(),
            "method": method,
            "params": params.cloned().unwrap_or(Value::Null),
        });
        self.sink.line(&line.to_string());
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
        let dump = Dump { sink: Append::at(&path) };
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
}

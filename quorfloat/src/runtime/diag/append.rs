//! The file this process appends lines to, when the environment names one.
//!
//! Both diagnostics in this crate are this mechanism with a different line format, so
//! the mechanism lives here and its rules are stated once:
//!
//! - **Off unless asked for.** Nothing is written without an environment variable, so a
//!   shipped build touches no disk.
//! - **Never load-bearing.** Every failure is dropped: a diagnostic that can fail a
//!   session is worse than no diagnostic.
//! - **Opened, written, and closed per line.** This process is killed by signal in the
//!   failure cases that matter, and a buffered line lost to a kill would remove exactly
//!   the evidence someone came for.

use std::path::PathBuf;

/// An append-only line sink that writes nothing unless it was given a path.
#[derive(Debug, Clone, Default)]
pub struct Append {
    path: Option<PathBuf>,
}

impl Append {
    /// A sink writing to the path named by an environment variable.
    ///
    /// Read once, where the process is assembled, rather than on every write: the
    /// environment is a startup input, and a diagnostic whose destination could change
    /// mid-run would make the file it wrote impossible to reason about.
    ///
    /// @param variable - the variable to read. Unset or blank means a silent sink.
    #[must_use]
    pub fn from_env(variable: &str) -> Self {
        Self::at_opt(
            std::env::var(variable)
                .ok()
                .filter(|value| !value.trim().is_empty())
                .map(PathBuf::from),
        )
    }

    /// A sink writing to an explicit path.
    ///
    /// @param path - where lines are appended.
    #[must_use]
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self::at_opt(Some(path.into()))
    }

    /// A sink writing to a path, or to nowhere when there is none.
    #[must_use]
    pub fn at_opt(path: Option<PathBuf>) -> Self {
        Self { path }
    }

    /// Whether anything would be written.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.path.is_some()
    }

    /// Append one line, if a path was named.
    ///
    /// @param line - text to append, without a trailing newline.
    pub fn line(&self, line: &str) {
        let Some(path) = &self.path else { return };
        use std::io::Write as _;
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(file, "{line}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch path that does not exist yet.
    fn scratch(tag: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("quorfloat-append-{tag}-{}.log", std::process::id()));
        let _ = std::fs::remove_file(&path);
        path
    }

    #[test]
    fn a_disabled_sink_writes_nothing() {
        let sink = Append::default();
        assert!(!sink.is_enabled());
        sink.line("this line goes nowhere");
    }

    #[test]
    fn a_sink_appends_lines_in_order() {
        let path = scratch("order");
        let sink = Append::at(&path);
        assert!(sink.is_enabled());
        sink.line("first");
        sink.line("second");

        let written = std::fs::read_to_string(&path).expect("the file exists");
        let _ = std::fs::remove_file(&path);
        assert_eq!(written, "first\nsecond\n");
    }

    #[test]
    fn a_second_sink_appends_rather_than_replaces() {
        // The whole point of these files is reconstructing a sequence, including across
        // restarts, so a fresh process must not truncate what the last one wrote.
        let path = scratch("append");
        Append::at(&path).line("before the restart");
        Append::at(&path).line("after the restart");

        let written = std::fs::read_to_string(&path).expect("the file exists");
        let _ = std::fs::remove_file(&path);
        assert_eq!(written.lines().count(), 2);
    }

    #[test]
    fn an_unwritable_path_is_survivable() {
        // A diagnostic must never be the reason a session fails.
        let sink = Append::at("/definitely/not/a/real/directory/diag.log");
        sink.line("dropped on the floor");
    }
}

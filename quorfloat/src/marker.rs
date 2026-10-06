//! The optional lifecycle marker: a file this process appends breadcrumbs to.
//!
//! It exists because the host captures this process's stderr and does not surface
//! it anywhere a developer or a test can read. That leaves questions like "did the
//! binary even start", "did the approval reach the panel", and "did the panel
//! answer it" unanswerable from outside — and every one of those has already been
//! asked for real during this project.
//!
//! Three properties keep it honest:
//!
//! - **Off unless asked for.** Nothing is written without
//!   `DSH_QUORFLOAT_RUST_MARKER`, so a shipped build touches no disk.
//! - **Never load-bearing.** Every failure is dropped: a diagnostic that can fail a
//!   session is worse than no diagnostic.
//! - **Append-only and timestamped.** The file is read to answer "what happened, in
//!   what order, and was there a restart", so a line without a time answers less
//!   than it appears to.

use std::path::PathBuf;

/// A breadcrumb file, or nothing when the environment does not name one.
#[derive(Debug, Clone, Default)]
pub struct Marker {
    path: Option<PathBuf>,
}

impl Marker {
    /// Read the marker path from `DSH_QUORFLOAT_RUST_MARKER`.
    ///
    /// @returns a marker that writes nothing when the variable is unset or blank.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            path: std::env::var("DSH_QUORFLOAT_RUST_MARKER")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .map(PathBuf::from),
        }
    }

    /// A marker that writes nothing.
    #[must_use]
    pub fn disabled() -> Self {
        Self { path: None }
    }

    /// Whether anything would be written.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.path.is_some()
    }

    /// Append one line, if a marker was requested.
    ///
    /// Opened, written, and closed per line rather than held open: this process is
    /// killed by signal in the failure cases that matter, and a buffered line lost
    /// to a kill would remove exactly the evidence someone came for.
    ///
    /// @param line - text to append, without a trailing newline.
    pub fn write(&self, line: &str) {
        let Some(path) = &self.path else { return };
        use std::io::Write as _;
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(file, "[{}] {line}", crate::rpc::now_millis());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_disabled_marker_writes_nothing_and_says_so() {
        let marker = Marker::disabled();
        assert!(!marker.is_enabled());
        marker.write("this line goes nowhere");
    }

    #[test]
    fn a_marker_appends_timestamped_lines_in_order() {
        let path = std::env::temp_dir().join(format!("quorfloat-marker-test-{}.log", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let marker = Marker { path: Some(path.clone()) };
        marker.write("start");
        marker.write("ready host=0.2.0-rc.2 session=quorfloat-1-abc");
        let written = std::fs::read_to_string(&path).expect("the marker file exists");
        let _ = std::fs::remove_file(&path);
        let lines: Vec<&str> = written.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with('['), "each line carries a timestamp: {}", lines[0]);
        assert!(lines[0].ends_with("] start"));
        assert!(lines[1].ends_with("] ready host=0.2.0-rc.2 session=quorfloat-1-abc"));
        // Two markers over one path append rather than replace: the whole point is
        // reconstructing a sequence, including across restarts.
        assert!(written.ends_with('\n'));
    }

    #[test]
    fn an_unwritable_path_is_survivable() {
        // A diagnostic must never be the reason a session fails.
        let marker = Marker { path: Some(PathBuf::from("/definitely/not/a/real/directory/marker.log")) };
        marker.write("start");
    }
}

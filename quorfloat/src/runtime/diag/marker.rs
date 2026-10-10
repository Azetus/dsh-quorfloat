//! The optional lifecycle marker: a file this process appends breadcrumbs to.
//!
//! It exists because the host captures this process's stderr and does not surface it
//! anywhere a developer or a test can read. That leaves questions like "did the binary
//! even start", "did the approval reach the panel", and "did the panel answer it"
//! unanswerable from outside.
//!
//! The file itself is [`Append`]; this type is its line format, which is
//! `[<milliseconds since the epoch>] <text>`: a line without a time answers less than it
//! appears to when the question is "what happened, in what order".

use crate::runtime::diag::append::Append;

/// The environment variable that names the marker file.
pub const MARKER_ENV: &str = "DSH_QUORFLOAT_RUST_MARKER";

/// A breadcrumb file, or nothing when the environment does not name one.
#[derive(Debug, Clone, Default)]
pub struct Marker {
    sink: Append,
}

impl Marker {
    /// Read the marker path from [`MARKER_ENV`].
    ///
    /// @returns a marker that writes nothing when the variable is unset or blank.
    #[must_use]
    pub fn from_env() -> Self {
        Self { sink: Append::from_env(MARKER_ENV) }
    }

    /// A marker that writes nothing.
    #[must_use]
    pub fn disabled() -> Self {
        Self { sink: Append::default() }
    }

    /// Whether anything would be written.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.sink.is_enabled()
    }

    /// Append one line, if a marker was requested.
    ///
    /// @param line - text to append, without a trailing newline.
    pub fn write(&self, line: &str) {
        self.sink.line(&format!("[{}] {line}", crate::ipc::rpc::now_millis()));
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
        let marker = Marker { sink: Append::at(&path) };
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
}

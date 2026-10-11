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

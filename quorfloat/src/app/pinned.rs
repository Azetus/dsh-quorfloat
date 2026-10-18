//! The conversation and workspace the user pinned, remembered across runs.
//!
//! A pin is a preference, so it belongs in a file — but it is also the *only* thing that
//! decides which conversation the panel opens, and a file that can break the panel is worse
//! than no file. The rules are therefore the ones `geometry.rs` uses for the window's
//! position, for the same reasons:
//!
//! - **Nothing is load-bearing.** A missing, unreadable or unparseable file means "nothing
//!   is pinned", not an error.
//! - **Nothing is written without a place to write it.**
//! - **A pin that cannot be honoured is dropped rather than kept.** A pinned conversation
//!   the host no longer lists must not leave the panel attached to nothing: the follow
//!   layer validates the pin against the list it receives and forgets an unlisted one,
//!   which puts the panel back into "new conversation" mode. The workspace half is validated
//!   the same way, against the workspace list, and forgotten on its own.
//!
//! **The two fields are two independent facts.** The session field is *the* pin — "which
//! conversation opens next", the panel's home. The workspace field is a pin of its own —
//! "where a new conversation is created" — and it is never derived from the conversation, nor
//! the other way around. Only a user gesture sets or clears either half, the program clears a
//! half only when that half itself has been shown to be invalid, and a gesture on one half
//! leaves the other exactly as it was. A file written by an older build (which did treat the
//! workspace field as the session pin's projection) is read leniently: it is simply a file
//! with two independent pins, and each is validated on its own.

use std::path::PathBuf;

/// What the user pinned, if anything.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pinned {
    /// The conversation to open, if one is pinned.
    pub session: Option<String>,
    /// The workspace new conversations are created in, if one is pinned.
    ///
    /// Independent of [`Pinned::session`]: either half may be remembered without the other.
    pub workspace: Option<String>,
}

impl Pinned {
    /// Whether anything is pinned at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.session.is_none() && self.workspace.is_none()
    }

    /// Read the pinned pair.
    ///
    /// @param path - where it would be. Missing or unreadable means nothing is pinned.
    /// @returns what was read, with empty strings treated as absent.
    #[must_use]
    pub fn load(path: &std::path::Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            return Self::default();
        };
        let string = |key: &str| {
            value
                .get(key)
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        };
        Self { session: string("session"), workspace: string("workspace") }
    }

    /// Write the pinned pair atomically.
    ///
    /// Failure is returned rather than swallowed: the caller keeps the in-memory pins and
    /// records a marker. An empty pin removes the file rather than writing `{}`: "nothing is
    /// pinned" and "the file says nothing is pinned" should not be two different states to
    /// reason about.
    ///
    /// @param path - where to write.
    /// @returns `Ok(())` when the file now holds the pins, or when the empty case removed it;
    ///   an error with the path in its message otherwise.
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        if self.is_empty() {
            return crate::app::persist::remove_if_present(path);
        }
        let body = serde_json::json!({ "session": self.session, "workspace": self.workspace });
        crate::app::persist::write_atomic(path, body.to_string().as_bytes())
    }
}

/// Where the pin lives when the environment does not name a place.
///
/// Beside the window's remembered position, for the same reason: it is this process's own
/// small piece of state about how the user likes the panel. The settings system will own it
/// in P3.
///
/// @returns the path, or `None` when there is no home to put it in.
#[must_use]
pub fn default_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|value| !value.is_empty())?;
    Some(PathBuf::from(home).join(".dsh-quorfloat").join("pinned.json"))
}

/// The path this process remembers its pins in.
///
/// @returns the environment's choice when it names one, otherwise the default.
#[must_use]
pub fn path_from_env() -> Option<PathBuf> {
    let named = std::env::var_os("DSH_QUORFLOAT_PINNED")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty());
    named.or_else(default_path)
}

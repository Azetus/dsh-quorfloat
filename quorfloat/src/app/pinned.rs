//! The conversation and workspace the user pinned, remembered across runs.
//!
//! A pin is a preference, so it belongs in a file — but it is also the *only* thing that
//! decides which conversation the panel opens, and a file that can break the panel is worse
//! than no file. The rules are therefore the ones `ui/geometry.rs` uses for the window's
//! position, for the same reasons:
//!
//! - **Nothing is load-bearing.** A missing, unreadable or unparseable file means "nothing
//!   is pinned", which is exactly the behaviour the panel had before pins existed.
//! - **Nothing is written without a place to write it.**
//! - **A pin that cannot be honoured is dropped rather than kept.** A pinned conversation
//!   the host no longer lists must not leave the panel attached to nothing: the follow layer
//!   validates the pin against the list it receives, and falls back to the newest
//!   conversation ([`crate::app::session::follow`]).

use std::path::PathBuf;

/// What the user pinned, if anything.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pinned {
    /// The conversation to open, if one is pinned.
    pub session: Option<String>,
    /// The workspace new conversations are created in, if one is pinned.
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

    /// Write the pinned pair. Failure is ignored: not being able to remember a preference is
    /// a smaller problem than refusing to run because of it.
    ///
    /// An empty pin removes the file rather than writing `{}`: "nothing is pinned" and "the
    /// file says nothing is pinned" should not be two different states to reason about.
    ///
    /// @param path - where to write.
    pub fn save(&self, path: &std::path::Path) {
        if self.is_empty() {
            let _ = std::fs::remove_file(path);
            return;
        }
        if let Some(directory) = path.parent() {
            let _ = std::fs::create_dir_all(directory);
        }
        let body = serde_json::json!({ "session": self.session, "workspace": self.workspace });
        let _ = std::fs::write(path, body.to_string());
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// A scratch path that does not exist yet.
    fn scratch(tag: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("quorfloat-pinned-{tag}-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        path
    }

    #[test]
    fn a_pin_survives_a_round_trip() {
        let path = scratch("round");
        let pinned = Pinned { session: Some("session-1".to_owned()), workspace: Some("ws-1".to_owned()) };
        pinned.save(&path);
        assert_eq!(Pinned::load(&path), pinned);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn nothing_pinned_is_no_file_rather_than_an_empty_one() {
        let path = scratch("empty");
        Pinned { session: Some("s".to_owned()), workspace: None }.save(&path);
        assert!(path.exists());

        Pinned::default().save(&path);
        assert!(!path.exists(), "unpinning removes the file");
        assert_eq!(Pinned::load(&path), Pinned::default(), "and reads back as nothing pinned");
    }

    #[test]
    fn a_missing_or_corrupt_file_means_nothing_is_pinned() {
        assert_eq!(Pinned::load(&scratch("missing")), Pinned::default());

        let path = scratch("corrupt");
        std::fs::write(&path, "{ not json").expect("write");
        assert_eq!(Pinned::load(&path), Pinned::default());

        // Empty strings are what a half-written or hand-edited file looks like, and an empty
        // pin is not a pin: it would attach the panel to a session id that cannot exist.
        std::fs::write(&path, r#"{"session": "  ", "workspace": ""}"#).expect("write");
        assert_eq!(Pinned::load(&path), Pinned::default());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_unwritable_path_is_survivable() {
        Pinned { session: Some("s".to_owned()), workspace: None }
            .save(Path::new("/definitely/not/a/real/directory/pinned.json"));
    }
}

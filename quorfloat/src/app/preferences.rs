//! What the user chose inside the panel, remembered across runs.
//!
//! Until the settings view existed, everything the panel drew was decided by the host's
//! configuration: the theme arrived in the `ready` payload and the panel had no opinion. A setting
//! the user changes *in the panel* is a different kind of fact — it has to take effect on the next
//! frame, and it has to survive a restart — so it needs somewhere to live, and the rules are the
//! ones `pinned.rs` and `geometry.rs` already use for the same problem:
//!
//! - **Nothing is load-bearing.** A missing, unreadable or unparseable file means "no local
//!   preference", which leaves the host's configuration in charge — exactly the behaviour the
//!   panel had before this file existed.
//! - **Nothing is written without a place to write it.**
//! - **An unrecognised value is not a value.** A theme this build does not know falls back to the
//!   host's setting rather than guessing, on the same principle as an unrecognised record shape in
//!   the transcript: showing the wrong thing is worse than showing the default.
//!
//! **Precedence, in one place:** the host's configuration is the baseline and these values sit on
//! top of it. That is why the fields are optional — `None` is the honest encoding of "the user has
//! not overridden this", and it is what keeps the host's config authoritative until it is.

use std::path::PathBuf;

use crate::app::theme::Preference;

/// What the user set in the settings view.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Preferences {
    /// The theme, when the user has chosen one here.
    ///
    /// Absent means the host's configuration decides, which is what the platform's own setting
    /// should do until someone says otherwise.
    pub theme: Option<Preference>,
    /// Whether the panel stays open when the user moves to another window.
    ///
    /// The design's `keepOpen`, and the inverse of the window setting the host sends: the design
    /// labels the switch by what it gives the user ("失焦时保持展开"), while the config names the
    /// behaviour the process implements (`hideOnBlur`). Both spellings are correct for their side;
    /// the conversion happens once, in [`WindowSettings::with_preferences`](crate::app::window_settings::WindowSettings).
    pub keep_open: Option<bool>,
    /// The accelerator the panel should register, when the user has changed it here.
    ///
    /// `None` means the host's configuration decides, which is what the hotkey the host started the
    /// process with should do until someone says otherwise.
    ///
    /// **The spec rather than a key.** What gets remembered has to be the same string the host
    /// writes and this process registers — `Alt+Space`, `Cmd+Shift+K` — because that value is what
    /// the settings page shows and what `hello` reports. Remembering a parsed key instead would mean
    /// two spellings of the same fact, and they would eventually disagree.
    pub hotkey: Option<String>,
}

impl Preferences {
    /// Whether the user has set nothing at all here.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.theme.is_none() && self.keep_open.is_none() && self.hotkey.is_none()
    }

    /// Read the preferences file.
    ///
    /// @param path - where it would be. Missing or unreadable means no local preference.
    /// @returns what was read, with unrecognised values treated as absent.
    #[must_use]
    pub fn load(path: &std::path::Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            return Self::default();
        };
        // `from_name` maps everything it does not recognise to `System`, which would turn a
        // hand-edited `"theme": "purple"` into a *decision* to follow the platform. Only the three
        // names this build knows are accepted here, so a typo leaves the host in charge.
        let theme = value
            .get("theme")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|name| {
                ["system", "light", "dark"]
                    .iter()
                    .any(|known| name.eq_ignore_ascii_case(known))
            })
            .map(|name| Preference::from_name(Some(name)));
        let keep_open = value.get("keepOpen").and_then(serde_json::Value::as_bool);
        // An accelerator is only remembered if this build can parse it. A hand-edited file naming a
        // key this build does not know must not become a registration attempt that fails on every
        // start: unrecognised means absent, and the host's configuration stays in charge.
        let hotkey = value
            .get("hotkey")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|spec| !spec.is_empty())
            .filter(|spec| crate::runtime::hotkey::parse_accelerator(spec).is_some())
            .map(str::to_owned);
        Self { theme, keep_open, hotkey }
    }

    /// Write the preferences. Failure is ignored: not being able to remember a preference is a
    /// smaller problem than refusing to run because of it.
    ///
    /// A file with nothing in it is removed rather than written as `{}`, so that "the user has
    /// chosen nothing" is one state and not two.
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
        let theme = self.theme.map(|preference| match preference {
            Preference::System => "system",
            Preference::Light => "light",
            Preference::Dark => "dark",
        });
        let body = serde_json::json!({
            "theme": theme,
            "keepOpen": self.keep_open,
            "hotkey": self.hotkey,
        });
        let _ = std::fs::write(path, body.to_string());
    }
}

/// Which accelerator a run should try to hold.
///
/// **The user's own choice wins over the host's configuration**, which is the precedence the theme
/// and the keep-open switch already follow: the host's value is the baseline, and a shortcut changed
/// *in the panel* is an override. Without this rule the settings page would appear to work and then
/// quietly forget itself, because the host starts every run with its own configured hotkey in the
/// spawn environment.
///
/// A pure function of two values on purpose: this decision is the one thing about the hotkey that
/// cannot be re-tested by hand once it is wrong, because the symptom is "the panel does not open".
///
/// @param preferences - what the user has chosen in the panel, if anything.
/// @param configured - the accelerator the host asked for at spawn time.
/// @returns the accelerator to attempt.
#[must_use]
pub fn hotkey_to_attempt(preferences: &Preferences, configured: &str) -> String {
    preferences
        .hotkey
        .clone()
        .filter(|spec| !spec.trim().is_empty())
        .unwrap_or_else(|| configured.to_owned())
}

/// Where the preferences live when the environment does not name a place.
///
/// Beside the pinned pair and the window's remembered position, for the same reason: it is this
/// process's own small piece of state about how the user likes the panel.
///
/// @returns the path, or `None` when there is no home to put it in.
#[must_use]
pub fn default_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|value| !value.is_empty())?;
    Some(PathBuf::from(home).join(".dsh-quorfloat").join("preferences.json"))
}

/// The path this process remembers its preferences in.
///
/// @returns the environment's choice when it names one, otherwise the default.
#[must_use]
pub fn path_from_env() -> Option<PathBuf> {
    let named = std::env::var_os("DSH_QUORFLOAT_PREFERENCES")
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
        let path = std::env::temp_dir()
            .join(format!("quorfloat-preferences-{tag}-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        path
    }

    #[test]
    fn a_preference_survives_a_round_trip() {
        let path = scratch("round");
        let preferences = Preferences {
            theme: Some(Preference::Dark),
            keep_open: Some(true),
            hotkey: Some("Cmd+Shift+K".to_owned()),
        };
        preferences.save(&path);
        assert_eq!(Preferences::load(&path), preferences);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn nothing_chosen_is_no_file_rather_than_an_empty_one() {
        let path = scratch("empty");
        Preferences { theme: Some(Preference::Light), keep_open: None, hotkey: None }.save(&path);
        assert!(path.exists());

        Preferences::default().save(&path);
        assert!(!path.exists(), "clearing every setting removes the file");
        assert_eq!(Preferences::load(&path), Preferences::default());
    }

    #[test]
    fn an_unreadable_file_leaves_the_host_in_charge() {
        assert_eq!(Preferences::load(&scratch("missing")), Preferences::default());

        let path = scratch("corrupt");
        std::fs::write(&path, "{ not json").expect("write");
        assert_eq!(Preferences::load(&path), Preferences::default());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_theme_this_build_does_not_know_is_not_a_theme() {
        // The trap this guards: `Preference::from_name` maps anything unrecognised to `System`, so
        // reading the file through it would turn a typo into a deliberate "follow the platform" —
        // a decision the user never made.
        let path = scratch("unknown");
        std::fs::write(&path, r#"{"theme": "purple", "keepOpen": true}"#).expect("write");
        let loaded = Preferences::load(&path);
        assert_eq!(loaded.theme, None, "an unknown name leaves the host's theme in charge");
        assert_eq!(loaded.keep_open, Some(true), "the rest of the file still counts");

        // And the names this build does know are read, whatever their case.
        std::fs::write(&path, r#"{"theme": "Dark"}"#).expect("write");
        assert_eq!(Preferences::load(&path).theme, Some(Preference::Dark));

        // A field of the wrong type is absent, not a remembered `false`.
        std::fs::write(&path, r#"{"keepOpen": "yes"}"#).expect("write");
        assert_eq!(Preferences::load(&path).keep_open, None);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_users_own_shortcut_outranks_the_hosts_configuration() {
        // The precedence rule, on its own. Its failure mode is the worst one this project has: the
        // settings page changes the shortcut, the next launch quietly re-registers the host's, and
        // the user's only symptom is a key that does not open the panel any more.
        let configured = "Alt+Space";
        let chosen = Preferences { hotkey: Some("Cmd+Shift+K".to_owned()), ..Preferences::default() };
        assert_eq!(hotkey_to_attempt(&chosen, configured), "Cmd+Shift+K");

        // With nothing chosen, the host decides — including the host's deliberate "no hotkey", which
        // every development profile relies on so two profiles do not fight over one accelerator.
        assert_eq!(hotkey_to_attempt(&Preferences::default(), configured), configured);
        assert_eq!(hotkey_to_attempt(&Preferences::default(), ""), "");

        // A blank choice is not a choice: it would mean "no shortcut", which is a different decision
        // from the one an empty field appears to make.
        let blank = Preferences { hotkey: Some("   ".to_owned()), ..Preferences::default() };
        assert_eq!(hotkey_to_attempt(&blank, configured), configured);
    }

    #[test]
    fn an_accelerator_this_build_cannot_read_is_not_remembered() {
        // A hand-edited file naming a key this build does not know must not become a registration
        // attempt that fails on every start: absent is the honest reading, and the host stays in
        // charge. The same rule the theme follows for a name it does not recognise.
        let path = scratch("bad-hotkey");
        std::fs::write(&path, r#"{"hotkey": "Frobnicate+Nope"}"#).expect("write");
        assert_eq!(Preferences::load(&path).hotkey, None);

        std::fs::write(&path, r#"{"hotkey": "  "}"#).expect("write");
        assert_eq!(Preferences::load(&path).hotkey, None, "blank is not an accelerator either");

        std::fs::write(&path, r#"{"hotkey": "Cmd+Shift+K"}"#).expect("write");
        assert_eq!(Preferences::load(&path).hotkey.as_deref(), Some("Cmd+Shift+K"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_unwritable_path_is_survivable() {
        Preferences { theme: Some(Preference::Dark), keep_open: None, hotkey: Some("Alt+Space".to_owned()) }
            .save(Path::new("/definitely/not/a/real/directory/preferences.json"));
    }
}

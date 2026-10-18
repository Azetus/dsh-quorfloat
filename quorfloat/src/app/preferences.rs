//! What the user chose inside the panel, remembered across runs.
//!
//! The host's configuration is a baseline: the theme arrives in the `ready` payload. A setting
//! the user changes *in the panel* is a different kind of fact — it has to take effect on the
//! next frame, and it has to survive a restart — so it needs somewhere to live, and the rules
//! are the ones `pinned.rs` and `geometry.rs` use for the same problem:
//!
//! - **Nothing is load-bearing.** A missing, unreadable or unparseable file means "no local
//!   preference", which leaves the host's configuration in charge.
//! - **Nothing is written without a place to write it.**
//! - **An unrecognised value is not a value.** A theme this build does not know falls back to the
//!   host's setting rather than guessing, on the same principle as an unrecognised record shape in
//!   the transcript: showing the wrong thing is worse than showing the default.
//!
//! **Precedence, in one place:** the host's configuration is the baseline and these values sit on
//! top of it. That is why the fields are optional — `None` is the honest encoding of "the user has
//! not overridden this", and it is what keeps the host's config authoritative until it is.

use std::path::PathBuf;

use crate::app::language::Language;
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
    /// The language the user chose in the panel, when they have chosen one here.
    ///
    /// This is the panel's own setting, and it is the one that is *persisted on the first
    /// launch*: the host resolves a language before the process starts (an explicit panel
    /// setting, or the Harness's own locale preference when there is none), and
    /// [`seed_language`] adopts that answer as a remembered choice when this field is
    /// still empty. Every later launch therefore reads an explicit value and never asks
    /// the Harness again — there is no follow mode and no live following.
    pub language: Option<Language>,
}

impl Preferences {
    /// Whether the user has set nothing at all here.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.theme.is_none() && self.keep_open.is_none() && self.hotkey.is_none() && self.language.is_none()
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
        // A language this build does not ship is absent, not a remembered choice: the
        // same rule the theme follows, and the reason the seed gets another chance
        // rather than the panel freezing on a typo.
        let language = Language::known(value.get("language").and_then(serde_json::Value::as_str));
        Self { theme, keep_open, hotkey, language }
    }

    /// Write the preferences atomically.
    ///
    /// Failure is returned rather than swallowed: the caller keeps the in-memory value and
    /// records a marker. A file with nothing in it is removed rather than written as `{}`, so
    /// that "the user has chosen nothing" is one state and not two.
    ///
    /// @param path - where to write.
    /// @returns `Ok(())` when the file now holds the preferences, or when the empty case
    ///   removed it; an error with the path in its message otherwise.
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        if self.is_empty() {
            return crate::app::persist::remove_if_present(path);
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
            "language": self.language.map(Language::as_str),
        });
        crate::app::persist::write_atomic(path, body.to_string().as_bytes())
    }
}

/// The panel's own language after the first-launch seed.
///
/// **The one rule about the seed, in one place.** The panel's remembered choice is the
/// authority once it exists; otherwise the language this launch resolved before the
/// process started decides, but only if it really *is* a decision:
///
/// - a `decided` baseline is the explicit plugin configuration or the Harness's stored
///   locale, and it is adopted and persisted here, exactly as the seed always did;
/// - a baseline that is only the host's `en` fallback is used for the first frame but
///   **not written down**: the panel's own webview has not reported yet, and its answer
///   outranks a fallback (see [`seed_from_reported`]).
///
/// There is no follow mode: whichever way this ends, it happens once per install.
///
/// Pure on purpose. The failure mode is a panel that is stuck in the wrong language for
/// every future launch, which is exactly the kind of decision that has to be asserted
/// without a window or a settings file.
///
/// @param chosen - the language the panel had remembered, when it had one.
/// @param baseline - the language this launch resolved before the process started.
/// @param decided - whether that baseline is a decision rather than the `en` fallback.
/// @returns the language to use, and whether it was newly adopted (and so must be saved).
#[must_use]
pub fn seed_language(chosen: Option<Language>, baseline: Language, decided: bool) -> (Language, bool) {
    match (chosen, decided) {
        (Some(language), _) => (language, false),
        (None, true) => (baseline, true),
        (None, false) => (baseline, false),
    }
}

/// What the panel adopts once its webview reports the languages it can see.
///
/// This is the last question of the resolution order, and it is only reached while
/// nothing has been chosen: a remembered preference — or a baseline that was a decision,
/// and so was already written down at startup — means the report has nothing to say.
/// Otherwise the raw list (`navigator.languages`) is walked in order and the first entry
/// whose primary subtag this build ships wins; an empty, absent, or unrecognised list is
/// `en`, the same fallback the host would have used.
///
/// @param chosen - the language the panel remembers *now*, after the startup seed.
/// @param reported - the tags the webview reported, verbatim and in its own order.
/// @returns the language to use, and whether it was newly adopted (and so must be saved).
#[must_use]
pub fn seed_from_reported(chosen: Option<Language>, reported: &[String]) -> (Language, bool) {
    match chosen {
        Some(language) => (language, false),
        None => (Language::from_reported(reported).unwrap_or(Language::En), true),
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

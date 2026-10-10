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
            "language": self.language.map(Language::as_str),
        });
        let _ = std::fs::write(path, body.to_string());
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
            language: Some(Language::En),
        };
        preferences.save(&path);
        assert_eq!(Preferences::load(&path), preferences);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn nothing_chosen_is_no_file_rather_than_an_empty_one() {
        let path = scratch("empty");
        Preferences { theme: Some(Preference::Light), keep_open: None, hotkey: None, language: None }
            .save(&path);
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
        Preferences {
            theme: Some(Preference::Dark),
            keep_open: None,
            hotkey: Some("Alt+Space".to_owned()),
            language: Some(Language::En),
        }
        .save(Path::new("/definitely/not/a/real/directory/preferences.json"));
    }

    #[test]
    fn a_language_preference_survives_a_round_trip() {
        // The panel's own setting is what later launches read *instead of* asking the
        // Harness again, so losing it would silently re-seed a language the user changed.
        let path = scratch("language-round");
        for language in [Language::Zh, Language::En] {
            let preferences = Preferences { language: Some(language), ..Preferences::default() };
            preferences.save(&path);
            assert_eq!(Preferences::load(&path), preferences);
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_language_this_build_does_not_know_is_not_a_preference() {
        // A hand-edited `"language": "fr"` must not become a decision: absent is the
        // honest reading, and the first-launch seed gets to answer again.
        let path = scratch("bad-language");
        std::fs::write(&path, r#"{"language": "fr"}"#).expect("write");
        assert_eq!(Preferences::load(&path).language, None);

        std::fs::write(&path, r#"{"language": "EN"}"#).expect("write");
        assert_eq!(Preferences::load(&path).language, Some(Language::En), "the ids are read case-insensitively");

        std::fs::write(&path, r#"{"language": 2}"#).expect("write");
        assert_eq!(Preferences::load(&path).language, None);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_first_launch_adopts_a_decided_language_and_later_launches_keep_it() {
        // The seed, in one place. On a launch with nothing remembered, a *decided* baseline
        // (the explicit plugin configuration, or the Harness's own locale) is adopted and
        // must be written down — otherwise every future launch would ask the Harness again,
        // which is the "follow mode" this design deliberately does not have.
        assert_eq!(seed_language(None, Language::Zh, true), (Language::Zh, true));
        assert_eq!(seed_language(None, Language::En, true), (Language::En, true));

        // With a remembered choice, the remembered one wins and nothing is written:
        // changing the Harness's language later must not move the panel.
        assert_eq!(seed_language(Some(Language::En), Language::Zh, true), (Language::En, false));
        assert_eq!(seed_language(Some(Language::Zh), Language::En, true), (Language::Zh, false));
    }

    #[test]
    fn a_fallback_baseline_is_not_written_down_before_the_webview_answers() {
        // `en` is also what the host answers when nobody chose anything, so it must not be
        // persisted as if it were a choice: doing so would freeze every future launch in
        // English before the panel's own webview ever gets to answer.
        assert_eq!(seed_language(None, Language::En, false), (Language::En, false));
        // The value is still what the first frame draws with, so nothing flickers.
        assert_eq!(seed_language(None, Language::Zh, false), (Language::Zh, false));
    }

    #[test]
    fn the_panels_own_preference_beats_the_webview() {
        // A language the user chose *in the panel* is the one thing the webview can never
        // overrule: the report is the answer to the first launch's question, not a vote on
        // every launch.
        let reported = vec!["en-US".to_owned(), "zh-CN".to_owned()];
        assert_eq!(seed_from_reported(Some(Language::Zh), &reported), (Language::Zh, false));
        assert_eq!(seed_from_reported(Some(Language::En), &reported), (Language::En, false));
    }

    #[test]
    fn a_decided_harness_language_beats_the_webview() {
        // Composed end to end, because the two halves live in different places: the host
        // resolves an explicit Harness `zh` and says it is a decision, so the startup seed
        // writes it down; the report of an English webview then finds a remembered value.
        let (baseline, written_down) = seed_language(None, Language::Zh, true);
        assert!(written_down, "a decision is persisted at startup");
        let remembered = written_down.then_some(baseline);
        let reported = vec!["en-US".to_owned()];
        assert_eq!(seed_from_reported(remembered, &reported), (Language::Zh, false));
    }

    #[test]
    fn the_seed_happens_once_and_a_later_launch_ignores_the_report() {
        // Launch one: nobody chose anything, so the baseline is only the fallback and is
        // not written down — until the webview reports a Chinese list, which is adopted
        // and persisted.
        let (provisional, written_down) = seed_language(None, Language::En, false);
        assert_eq!((provisional, written_down), (Language::En, false));
        let reported = vec!["zh-Hans-CN".to_owned(), "en-US".to_owned()];
        let (adopted, seeded) = seed_from_reported(None, &reported);
        assert_eq!((adopted, seeded), (Language::Zh, true), "the webview seeds the panel");

        // Launch two: the file now remembers it, so neither the host's fallback nor a new
        // report can move it — there is no follow mode.
        let remembered = seeded.then_some(adopted);
        assert_eq!(seed_language(remembered, Language::En, false), (Language::Zh, false));
        assert_eq!(
            seed_from_reported(remembered, &["en-US".to_owned()]),
            (Language::Zh, false),
        );
    }

    #[test]
    fn a_stored_language_we_do_not_ship_still_leaves_the_webview_the_answer() {
        // A stored `zh-Hant` is not one of the two ids, so the host resolves `en` and says
        // it is *not* a decision; with no webview answer either, the panel is English —
        // strictly what the user decided, and the fallback the report must not skip.
        let (provisional, written_down) = seed_language(None, Language::En, false);
        assert_eq!((provisional, written_down), (Language::En, false));
        assert_eq!(seed_from_reported(None, &[]), (Language::En, true));
        assert_eq!(seed_from_reported(None, &["fr-FR".to_owned()]), (Language::En, true));
    }
}

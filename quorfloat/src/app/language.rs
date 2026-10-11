//! Which language the panel draws and labels itself in.
//!
//! Exactly two ids, and they are the Harness's own spelling (`zh`, `en`), so the value
//! that crosses the wire from `src/config.ts` and the value the frontend reads out of
//! the snapshot are the same string — there is no third vocabulary to keep in step.
//!
//! The host resolves the language (a panel setting, seeded from the Harness's locale on
//! the first launch) and sends it in the spawn environment and in the `ready` payload.
//! What this module owns is the reading of that value: an unrecognised or absent name
//! keeps the built-in default, which is Chinese. A typo in an environment variable must
//! not decide what language the panel speaks, the same rule `theme::Preference` follows
//! for a palette it does not know.

/// A language the panel ships.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Language {
    /// Chinese, the language the panel's own strings were written in first.
    #[default]
    Zh,
    /// English.
    En,
}

impl Language {
    /// Read the language from the environment.
    ///
    /// @returns the configured language, or the default when the variable is absent
    ///   or names a language this build does not know.
    #[must_use]
    pub fn from_env() -> Self {
        Self::from_name(std::env::var(ENV_LANGUAGE).ok().as_deref())
    }

    /// Read a language from its id, which is also the spelling the host sends.
    ///
    /// @param name - `zh`, `en`, or anything else.
    /// @returns the matching language, defaulting to [`Language::Zh`].
    #[must_use]
    pub fn from_name(name: Option<&str>) -> Self {
        Self::known(name).unwrap_or_default()
    }

    /// Recognise a language id, without inventing one for an unrecognised name.
    ///
    /// The distinction matters where a value already in force exists: `from_name` answers
    /// "which language should a typo mean", while this answers "did the other side
    /// actually name one of ours". A pushed configuration uses the second reading, so a
    /// name this build does not know leaves the current language alone.
    ///
    /// @param name - candidate id.
    /// @returns the language, or `None` when the name is not one of ours.
    #[must_use]
    pub fn known(name: Option<&str>) -> Option<Self> {
        match name.map(str::trim) {
            Some(value) if value.eq_ignore_ascii_case("zh") => Some(Self::Zh),
            Some(value) if value.eq_ignore_ascii_case("en") => Some(Self::En),
            _ => None,
        }
    }

    /// The language a webview's own preference list asks for.
    ///
    /// The list is the browser's answer in the user's own order (`navigator.languages`),
    /// and the rule here is the one the Harness itself follows while nothing is stored:
    /// walk the list and take the **first entry this build supports**, by primary subtag.
    /// `zh`, `zh-CN`, `zh-Hans` and `zh-Hant` all name the Chinese this build ships, and
    /// `en-US` names English; anything else is skipped and the next entry is considered.
    /// Case does not matter — the tags cross a webview, not this project's own wire.
    ///
    /// @param languages - tags in the order the webview reported them.
    /// @returns the first supported language, or `None` when none of them is supported.
    #[must_use]
    pub fn from_reported<S: AsRef<str>>(languages: &[S]) -> Option<Self> {
        languages.iter().find_map(|tag| {
            // The primary subtag is everything before the first `-` of a BCP 47 tag. An
            // entry with no usable primary subtag is skipped rather than guessed at.
            let primary = tag.as_ref().trim().split('-').next().unwrap_or("");
            match primary.to_ascii_lowercase().as_str() {
                "zh" => Some(Self::Zh),
                "en" => Some(Self::En),
                _ => None,
            }
        })
    }

    /// The id this language is written and sent as.
    ///
    /// @returns `zh` or `en`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Zh => "zh",
            Self::En => "en",
        }
    }

    /// The BCP 47 tag for the document element.
    ///
    /// A separate spelling from [`Self::as_str`] because the two answer different questions:
    /// the id is this project's wire value, while `lang` on `<html>` is read by the platform
    /// for font selection, hyphenation and assistive technology. The shipped document says
    /// `zh-CN`, so Chinese keeps that exact tag — an English switch is the only thing that
    /// changes, and it changes to `en`.
    ///
    /// @returns the document tag.
    #[must_use]
    pub fn as_document_tag(self) -> &'static str {
        match self {
            Self::Zh => "zh-CN",
            Self::En => "en",
        }
    }
}

/// The script that puts a language on the document element.
///
/// `frontend/index.html` ships `lang="zh-CN"` because the panel's copy defaults to Chinese, and
/// the value is a document-level fact the frontend cannot derive from its own state alone.
/// The half that resolves the language is the half that keeps this honest: the script is
/// applied as an initialization script (so the very first paint carries it) and re-applied with
/// `WebviewWindow::eval` when the setting changes at runtime.
///
/// The template takes a [`Language`] rather than a string on purpose: there is no path by which
/// a configuration value could reach the JavaScript as text of its own.
///
/// @param language - the language in force.
/// @returns one JavaScript statement.
#[must_use]
pub fn document_language_script(language: Language) -> String {
    format!("document.documentElement.lang = '{}';", language.as_document_tag())
}

/// The environment variable the host passes the resolved language in.
pub const ENV_LANGUAGE: &str = "DSH_QUORFLOAT_WINDOW_LANGUAGE";

/// The environment variable that says whether that language is a decision.
///
/// The host always resolves *some* language before the process starts, and `en` is also
/// what it answers when nobody chose anything. The sidecar has to tell those two apart:
/// a decision is written down as the panel's own setting, while a fallback must wait for
/// the panel's webview to report what it can see (see [`crate::app::preferences::seed_from_reported`]).
pub const ENV_LANGUAGE_DECIDED: &str = "DSH_QUORFLOAT_WINDOW_LANGUAGE_DECIDED";

/// Whether a host-supplied "decided" value means the language is a decision.
///
/// **Absent is `true`.** A process nobody told (a manual run, an older host build) treats
/// the value in the environment — or the built-in default — as the baseline the first
/// launch writes down, rather than waiting for a report that may never come. Only an
/// explicit negative says "this is the fallback".
///
/// @param value - the raw environment value, when the variable is set.
/// @returns whether the language in play is a decision.
#[must_use]
pub fn baseline_decided(value: Option<&str>) -> bool {
    match value.map(str::trim) {
        None => true,
        Some(text) => !matches!(text.to_ascii_lowercase().as_str(), "0" | "false" | "no" | "off"),
    }
}

/// Read [`baseline_decided`] from [`ENV_LANGUAGE_DECIDED`].
///
/// @returns whether the language in the environment is a decision.
#[must_use]
pub fn baseline_decided_from_env() -> bool {
    baseline_decided(std::env::var(ENV_LANGUAGE_DECIDED).ok().as_deref())
}

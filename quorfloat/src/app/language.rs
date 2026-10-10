//! Which language the panel draws and labels itself in.
//!
//! Exactly two ids, and they are the Harness's own spelling (`zh`, `en`), so the value
//! that crosses the wire from `src/config.ts` and the value the frontend reads out of
//! the snapshot are the same string — there is no third vocabulary to keep in step.
//!
//! The host resolves the language (a panel setting, seeded from the Harness's locale on
//! the first launch) and sends it in the spawn environment and in the `ready` payload.
//! What this module owns is the reading of that value: an unrecognised or absent name
//! keeps the built-in default, which is the Chinese the tray shipped with before the
//! setting existed. A typo in an environment variable must not decide what language the
//! panel speaks, the same rule `theme::Preference` follows for a palette it does not know.

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
/// `frontend/index.html` ships `lang="zh-CN"` because the panel's copy was Chinese first, and
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ids_the_host_and_the_settings_file_write_are_read_back() {
        assert_eq!(Language::from_name(Some("zh")), Language::Zh);
        assert_eq!(Language::from_name(Some(" EN ")), Language::En);
        assert_eq!(Language::Zh.as_str(), "zh");
        assert_eq!(Language::En.as_str(), "en");
    }

    #[test]
    fn an_unknown_value_is_not_a_decision() {
        // The tray shipped in Chinese before this setting existed; a typo must keep it
        // there rather than silently switching the panel's language.
        assert_eq!(Language::from_name(Some("fr")), Language::Zh);
        assert_eq!(Language::from_name(Some("")), Language::Zh);
        assert_eq!(Language::from_name(None), Language::Zh);
        assert_eq!(Language::default(), Language::Zh);
    }

    #[test]
    fn an_unrecognised_pushed_id_is_reported_as_unknown() {
        // `known` exists so a pushed configuration can tell "the host named English"
        // from "the host named something this build does not know": the second must not
        // become a decision.
        assert_eq!(Language::known(Some("en")), Some(Language::En));
        assert_eq!(Language::known(Some(" zh ")), Some(Language::Zh));
        assert_eq!(Language::known(Some("fr")), None);
        assert_eq!(Language::known(Some("")), None);
        assert_eq!(Language::known(None), None);
    }

    #[test]
    fn the_document_tag_keeps_the_shipped_chinese_and_is_plain_english() {
        // `frontend/index.html` ships `lang="zh-CN"`: switching to English has to replace it,
        // and staying in Chinese has to leave it exactly as it was.
        assert_eq!(Language::Zh.as_document_tag(), "zh-CN");
        assert_eq!(Language::En.as_document_tag(), "en");
        assert_eq!(
            document_language_script(Language::En),
            "document.documentElement.lang = 'en';"
        );
        assert_eq!(
            document_language_script(Language::Zh),
            "document.documentElement.lang = 'zh-CN';"
        );
    }
}

//! The window: how big the panel is, and how it sits among other windows.
//!
//! Settings arrive from the environment at startup and can be overridden by the host's
//! `ready` payload; everything here is the translation of those numbers into what the
//! shell asks of the platform. Nothing in this file decides *whether* the panel is
//! visible — that is state, and it lives in the shell's dispatcher (`main.rs`).
//!
//! This module is the configuration contract without the viewport builder.

/// Window geometry and appearance, from the host's effective configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowSettings {
    /// Width in logical pixels.
    pub width: f32,
    /// Maximum expanded height in logical pixels.
    pub max_height: f32,
    /// Keep the panel above other windows.
    pub always_on_top: bool,
    /// Suppress animations. The host exposes this because motion sensitivity is a
    /// user preference, not a per-app choice.
    pub reduce_motion: bool,
    /// Which of the design's two palettes to draw in.
    pub theme: crate::app::theme::Preference,
    /// Which language the panel's own interface and menu bar speak.
    ///
    /// The panel's setting, not the conversation's: the strings this process owns (the tray
    /// menu) and the copy the frontend owns are drawn from it. The host resolves it before
    /// the process starts — an explicit panel setting, or the Harness's own locale on the
    /// very first launch — and the panel's remembered choice sits on top of it like every
    /// other setting here (see [`Self::with_preferences`]).
    pub language: crate::app::language::Language,
    /// Put the panel away when the user moves to another window.
    ///
    /// The design's own semantics: the panel is a thing you summon, use, and leave — and a
    /// panel still sitting over someone else's application after they have gone back to it
    /// is in the way. Off is a legitimate choice too, so it is a setting, not a rule.
    pub hide_on_blur: bool,
    /// Open with the panel already showing.
    ///
    /// Off by default and meant for development: it is the difference between "look at the
    /// panel" and "press the hotkey and then describe what you saw", which is what makes a
    /// scriptable run possible.
    pub start_visible: bool,
    /// Where the panel opens when there is no remembered position to restore.
    ///
    /// A startup fact like [`Self::start_visible`], not something the host's `ready` payload can
    /// change: the anchor decides where the window is *created*, and a configuration push arrives
    /// after that, when moving a panel the user may already be looking at would be the wrong
    /// thing to do. The default is the centre of the primary display (see
    /// [`crate::app::geometry::Anchor::Center`]), which is also what the panel falls back to when
    /// a remembered position turns out to be on no attached display.
    pub anchor: crate::app::geometry::Anchor,
}

impl Default for WindowSettings {
    fn default() -> Self {
        // Matches `DEFAULT_CONFIG.window` on the host side. Duplicated rather than
        // requested so the panel can be shown before the handshake completes; the
        // host's `ready` payload is the authority once it arrives.
        Self {
            // The design's own `max-width:640px`.
            width: 640.0,
            max_height: 560.0,
            always_on_top: true,
            reduce_motion: false,
            // A panel that floats over other applications should look like it belongs to
            // the desktop it is floating over, so the platform decides until told otherwise.
            theme: crate::app::theme::Preference::System,
            // The language the tray's own strings are written in; the host overrides this
            // from the resolved setting, and a standalone run keeps the historical Chinese.
            language: crate::app::language::Language::default(),
            hide_on_blur: true,
            start_visible: false,
            // The host's schema default, restated here for the same reason as the numbers above:
            // the very first frame is placed before `ready` arrives.
            anchor: crate::app::geometry::Anchor::default(),
        }
    }
}

impl WindowSettings {
    /// Read overrides from the environment.
    ///
    /// The host passes `DSH_QUORFLOAT_WINDOW_*` at spawn time, which is what makes
    /// the panel the right size on its very first frame instead of resizing once
    /// the handshake returns.
    ///
    /// @returns the settings, falling back to defaults per field.
    #[must_use]
    pub fn from_env() -> Self {
        let defaults = Self::default();
        Self {
            width: float_env("DSH_QUORFLOAT_WINDOW_WIDTH", defaults.width),
            max_height: float_env("DSH_QUORFLOAT_WINDOW_MAX_HEIGHT", defaults.max_height),
            always_on_top: bool_env("DSH_QUORFLOAT_WINDOW_ALWAYS_ON_TOP", defaults.always_on_top),
            theme: crate::app::theme::Preference::from_env(),
            language: crate::app::language::Language::from_env(),
            hide_on_blur: bool_env("DSH_QUORFLOAT_WINDOW_HIDE_ON_BLUR", defaults.hide_on_blur),
            start_visible: bool_env("DSH_QUORFLOAT_WINDOW_START_VISIBLE", defaults.start_visible),
            reduce_motion: bool_env("DSH_QUORFLOAT_WINDOW_REDUCE_MOTION", defaults.reduce_motion),
            anchor: anchor_env("DSH_QUORFLOAT_WINDOW_ANCHOR", defaults.anchor),
        }
    }

    /// Apply the window section of the host's `ready` payload.
    ///
    /// @param window - the `config.window` object, if present.
    pub fn apply_host(&mut self, window: Option<&serde_json::Value>) {
        let Some(window) = window.and_then(serde_json::Value::as_object) else {
            return;
        };
        if let Some(width) = window.get("width").and_then(serde_json::Value::as_f64) {
            self.width = width as f32;
        }
        if let Some(height) = window.get("maxHeight").and_then(serde_json::Value::as_f64) {
            self.max_height = height as f32;
        }
        if let Some(hide) = window.get("hideOnBlur").and_then(serde_json::Value::as_bool) {
            self.hide_on_blur = hide;
        }
        if let Some(theme) = window.get("theme").and_then(serde_json::Value::as_str) {
            self.theme = crate::app::theme::Preference::from_name(Some(theme));
        }
        // An unrecognised id keeps the language already in force: this value crosses a
        // language boundary (`LanguagePreference` in `src/config.ts`), and a host build
        // that learns a third language must not make an older panel guess at it.
        if let Some(named) = crate::app::language::Language::known(
            window.get("language").and_then(serde_json::Value::as_str),
        ) {
            self.language = named;
        }
        if let Some(always) = window.get("alwaysOnTop").and_then(serde_json::Value::as_bool) {
            self.always_on_top = always;
        }
        if let Some(reduce) = window.get("reduceMotion").and_then(serde_json::Value::as_bool) {
            self.reduce_motion = reduce;
        }
    }

    /// Put what the user chose in the panel's own settings on top of the configuration.
    ///
    /// The host's configuration is the baseline and a local choice is an override — never the
    /// other way round, or the settings view would appear to do nothing. Only the fields the user
    /// actually set are touched, so the host stays in charge of everything else (including the
    /// theme, until someone picks one).
    ///
    /// **The two spellings meet here, once.** The design labels the switch by what it gives the
    /// user — "失焦时保持展开" — while the configuration names the behaviour the process implements,
    /// `hideOnBlur`. Converting between them in the settings view would spread the inversion across
    /// every place that reads either; doing it at the boundary keeps one meaning per name.
    ///
    /// @param self - the settings to overlay onto.
    /// @param preferences - what the user chose here, if anything.
    /// @returns the effective settings.
    #[must_use]
    pub fn with_preferences(&self, preferences: &crate::app::preferences::Preferences) -> Self {
        let mut effective = self.clone();
        if let Some(theme) = preferences.theme {
            effective.theme = theme;
        }
        if let Some(keep_open) = preferences.keep_open {
            effective.hide_on_blur = !keep_open;
        }
        if let Some(language) = preferences.language {
            effective.language = language;
        }
        effective
    }
}

/// Read a float-valued environment variable.
fn float_env(key: &str, fallback: f32) -> f32 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<f32>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(fallback)
}

/// Read an anchor-valued environment variable.
///
/// An unknown name keeps the default rather than moving the panel to a guess: the value crosses a
/// language boundary (`WindowAnchor` in `src/config.ts`), and a build that predates a new spelling
/// must not open the panel somewhere arbitrary.
fn anchor_env(key: &str, fallback: crate::app::geometry::Anchor) -> crate::app::geometry::Anchor {
    std::env::var(key)
        .ok()
        .and_then(|value| crate::app::geometry::Anchor::from_name(&value))
        .unwrap_or(fallback)
}

/// Read a boolean-valued environment variable, accepting the spellings the host
/// and a human both produce.
fn bool_env(key: &str, fallback: bool) -> bool {
    match std::env::var(key) {
        Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" => false,
            _ => fallback,
        },
        Err(_) => fallback,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_settings_default_to_the_host_defaults() {
        // Both sides carry the same numbers, so a first frame drawn before `ready`
        // arrives is not visibly resized a moment later.
        let settings = WindowSettings::default();
        assert_eq!(settings.width, 640.0);
        assert_eq!(settings.max_height, 560.0);
        assert!(settings.always_on_top);
        assert!(!settings.reduce_motion);
    }

    #[test]
    fn the_panel_anchors_at_the_centre_unless_told_otherwise() {
        // The half that has to agree with `src/config.ts` and `cordis.patch.yml` is this one.
        use crate::app::geometry::Anchor;
        assert_eq!(WindowSettings::default().anchor, Anchor::Center);
        assert_eq!(anchor_env("DSH_QUORFLOAT_TEST_ANCHOR_ABSENT", Anchor::Center), Anchor::Center);
    }

    #[test]
    fn the_host_ready_payload_overrides_geometry() {
        let mut settings = WindowSettings::default();
        settings.apply_host(Some(&serde_json::json!({
            "width": 480, "maxHeight": 300, "alwaysOnTop": false, "reduceMotion": true,
        })));
        assert_eq!(settings.width, 480.0);
        assert_eq!(settings.max_height, 300.0);
        assert!(!settings.always_on_top);
        assert!(settings.reduce_motion);
    }

    #[test]
    fn a_partial_or_absent_host_payload_leaves_defaults_alone() {
        let mut settings = WindowSettings::default();
        settings.apply_host(Some(&serde_json::json!({ "width": 500 })));
        assert_eq!(settings.width, 500.0);
        assert_eq!(settings.max_height, 560.0, "the missing field kept its default");
        settings.apply_host(None);
        assert_eq!(settings.width, 500.0, "an absent payload changes nothing");
    }

    #[test]
    fn the_host_resolves_the_language_on_top_of_the_built_in_default() {
        use crate::app::language::Language;
        // The host sends the value it resolved (a panel setting, or the Harness's own
        // locale on the first launch), so the panel never has to read the Harness itself.
        let mut settings = WindowSettings::default();
        assert_eq!(settings.language, Language::Zh, "a standalone run keeps the historical Chinese");
        settings.apply_host(Some(&serde_json::json!({ "language": "en" })));
        assert_eq!(settings.language, Language::En);
        // An id this build does not ship is not a decision: the current language stands.
        settings.apply_host(Some(&serde_json::json!({ "language": "fr" })));
        assert_eq!(settings.language, Language::En, "an unknown id leaves the language alone");
        settings.apply_host(Some(&serde_json::json!({ "language": "" })));
        assert_eq!(settings.language, Language::En, "the schema's unset is not a language either");
    }

    #[test]
    fn the_panels_own_language_outranks_the_hosts_baseline() {
        use crate::app::language::Language;
        use crate::app::preferences::Preferences;
        // The same precedence every other panel setting follows: the host's configuration
        // is the baseline, and a choice made *in the panel* is an override. Without this
        // the settings page would appear to work and then lose to the seed.
        let baseline = WindowSettings { language: Language::Zh, ..WindowSettings::default() };
        let chosen = Preferences { language: Some(Language::En), ..Preferences::default() };
        assert_eq!(baseline.with_preferences(&chosen).language, Language::En);
        // With nothing chosen here, the host's resolved value stands.
        assert_eq!(baseline.with_preferences(&Preferences::default()).language, Language::Zh);
    }
}

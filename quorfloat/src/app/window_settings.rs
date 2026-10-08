//! The window: how big the panel is, and how it sits among other windows.
//!
//! Settings arrive from the environment at startup and can be overridden by the host's
//! `ready` payload; everything here is the translation of those numbers into what the
//! shell asks of the platform. Nothing in this file decides *whether* the panel is
//! visible — that is state, and it lives in the shell's dispatcher (`main.rs`).
//!
//! This is the pure half of the old `ui::window`: the viewport builder left with egui,
//! the configuration contract did not.

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
            hide_on_blur: true,
            start_visible: false,
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
            hide_on_blur: bool_env("DSH_QUORFLOAT_WINDOW_HIDE_ON_BLUR", defaults.hide_on_blur),
            start_visible: bool_env("DSH_QUORFLOAT_WINDOW_START_VISIBLE", defaults.start_visible),
            reduce_motion: bool_env("DSH_QUORFLOAT_WINDOW_REDUCE_MOTION", defaults.reduce_motion),
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
}

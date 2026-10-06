//! The viewport: how big the panel is, and how it sits among other windows.
//!
//! Settings arrive from the environment at startup and can be overridden by the host's
//! `ready` payload; everything here is the translation of those numbers into the
//! viewport egui creates. Nothing in this file decides *whether* the panel is visible —
//! that is state, and it lives in `app`.

use eframe::egui;


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
}

impl Default for WindowSettings {
    fn default() -> Self {
        // Matches `DEFAULT_CONFIG.window` on the host side. Duplicated rather than
        // requested so the panel can be shown before the handshake completes; the
        // host's `ready` payload is the authority once it arrives.
        Self { width: 640.0, max_height: 560.0, always_on_top: true, reduce_motion: false }
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
        if let Some(always) = window.get("alwaysOnTop").and_then(serde_json::Value::as_bool) {
            self.always_on_top = always;
        }
        if let Some(reduce) = window.get("reduceMotion").and_then(serde_json::Value::as_bool) {
            self.reduce_motion = reduce;
        }
    }
}

/// Build the viewport the panel is shown in.
///
/// @param settings - geometry and appearance.
/// @returns the builder to hand to `NativeOptions`.
#[must_use]
pub fn viewport(settings: &WindowSettings) -> egui::ViewportBuilder {
    let builder = egui::ViewportBuilder::default()
        // Undecorated: this is a panel that appears over the user's work, not a
        // document window competing for space in the window list.
        .with_decorations(false)
        // `always_on_top` is a level, not a flag: there is no "not on top"
        // variant to pass, so the default level is simply left alone.
        .with_window_level(if settings.always_on_top {
            egui::WindowLevel::AlwaysOnTop
        } else {
            egui::WindowLevel::Normal
        })
        .with_resizable(false)
        // Starts hidden. The hotkey is what reveals it, and starting visible would
        // flash a panel on every launch — including every automatic restart.
        .with_visible(false)
        .with_inner_size([settings.width, settings.max_height])
        .with_min_inner_size([settings.width.min(320.0), 80.0])
        .with_title("quorfloat");
    builder
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

    #[test]
    fn the_viewport_starts_hidden_and_undecorated() {
        // Both are load-bearing: visible would flash a panel on every launch and
        // every automatic restart, and decorations would make it a document window
        // that takes a slot in the window list and the taskbar.
        let settings = WindowSettings::default();
        let viewport = viewport(&settings);
        assert_eq!(viewport.visible, Some(false));
        assert_eq!(viewport.decorations, Some(false));
        assert_eq!(viewport.window_level, Some(egui::WindowLevel::AlwaysOnTop));
        assert_eq!(viewport.resizable, Some(false));
        assert_eq!(viewport.inner_size, Some([640.0, 560.0].into()));
    }

    #[test]
    fn turning_always_on_top_off_selects_the_normal_level() {
        // There is no "off" flag to clear, so the configured value has to pick the
        // level — otherwise the setting would look accepted and do nothing.
        let settings = WindowSettings { always_on_top: false, ..WindowSettings::default() };
        assert_eq!(viewport(&settings).window_level, Some(egui::WindowLevel::Normal));
    }
}

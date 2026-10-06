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
    /// Which of the design's two palettes to draw in.
    pub theme: crate::ui::theme::Preference,
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
            theme: crate::ui::theme::Preference::System,
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
            theme: crate::ui::theme::Preference::from_env(),
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
        if let Some(theme) = window.get("theme").and_then(serde_json::Value::as_str) {
            self.theme = crate::ui::theme::Preference::from_name(Some(theme));
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
///
/// @param settings - size and layering, from the environment and the host's `ready`.
/// @param remembered - where the window was last time, if anywhere.
pub fn viewport(settings: &WindowSettings, remembered: Option<(f32, f32)>) -> egui::ViewportBuilder {
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
        // The panel draws its own rounded corners and its own shadow, which is only possible
        // if the window behind them is not painted: an opaque window would show its square
        // corners around the rounded panel.
        .with_transparent(true)
        // Starts hidden. The hotkey is what reveals it, and starting visible would
        // flash a panel on every launch — including every automatic restart.
        .with_visible(false)
        // The window is the panel *plus* the room the panel's shadow needs, so that the
        // configured width and height keep meaning "how big the panel is".
        .with_inner_size([
            settings.width + f32::from(crate::ui::theme::SHADOW_ROOM_SIDE) * 2.0,
            settings.max_height
                + f32::from(crate::ui::theme::SHADOW_ROOM_TOP)
                + f32::from(crate::ui::theme::SHADOW_ROOM_BOTTOM),
        ])
        .with_min_inner_size([settings.width.min(320.0), 80.0])
        .with_title("quorfloat");
    // Where the user left it — or nowhere, which is a real difference rather than a
    // default: `ViewportBuilder` has to be *told* a position to place the window, so a
    // panel that always opened at the origin would be one the user moves every launch.
    // Unset means the platform chooses, which is what a first run should look like.
    match remembered {
        Some((x, y)) => builder.with_position(egui::Pos2::new(x, y)),
        None => builder,
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

    #[test]
    fn the_viewport_starts_hidden_and_undecorated() {
        // Both are load-bearing: visible would flash a panel on every launch and
        // every automatic restart, and decorations would make it a document window
        // that takes a slot in the window list and the taskbar.
        let settings = WindowSettings::default();
        let viewport = viewport(&settings, None);
        assert_eq!(viewport.visible, Some(false));
        assert_eq!(viewport.decorations, Some(false));
        assert_eq!(viewport.window_level, Some(egui::WindowLevel::AlwaysOnTop));
        assert_eq!(viewport.resizable, Some(false));
    }

    #[test]
    fn the_window_is_the_panel_plus_the_room_its_shadow_needs() {
        // The panel has rounded corners and a shadow of its own, which needs a transparent
        // window and a margin to draw them in — so the window is bigger than the panel, and
        // the configured numbers keep meaning "how big the panel is".
        let settings = WindowSettings::default();
        let viewport = viewport(&settings, None);
        assert_eq!(viewport.transparent, Some(true), "the panel draws its own corners");
        let expected = [
            settings.width + f32::from(crate::ui::theme::SHADOW_ROOM_SIDE) * 2.0,
            settings.max_height
                + f32::from(crate::ui::theme::SHADOW_ROOM_TOP)
                + f32::from(crate::ui::theme::SHADOW_ROOM_BOTTOM),
        ];
        assert_eq!(viewport.inner_size, Some(expected.into()));
    }

    #[test]
    fn a_remembered_position_is_where_the_window_opens() {
        let settings = WindowSettings::default();
        assert_eq!(
            viewport(&settings, Some((120.0, 64.0))).position,
            Some(egui::Pos2::new(120.0, 64.0)),
        );
    }

    #[test]
    fn with_nothing_remembered_the_platform_chooses_where_to_open() {
        // Not the origin: a panel that always appears in the top-left corner is a panel the
        // user has to move every single time.
        assert_eq!(viewport(&WindowSettings::default(), None).position, None);
    }

    #[test]
    fn turning_always_on_top_off_selects_the_normal_level() {
        // There is no "off" flag to clear, so the configured value has to pick the
        // level — otherwise the setting would look accepted and do nothing.
        let settings = WindowSettings { always_on_top: false, ..WindowSettings::default() };
        assert_eq!(viewport(&settings, None).window_level, Some(egui::WindowLevel::Normal));
    }
}

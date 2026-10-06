//! Native window, always-on-top behaviour, and the global hotkey.
//!
//! Three jobs, in the order they matter:
//!
//! 1. **Summon the panel from anywhere.** The hotkey is registered through
//!    `global-hotkey`, not as a window-local binding, because the whole point of
//!    this window is that Harness is not the focused application when it appears.
//! 2. **Behave like a system panel, not a document window.** Undecorated,
//!    always-on-top, and hidden rather than closed — closing it must not end the
//!    process, because the process is what keeps the host's channel alive.
//! 3. **Report its own visibility.** The host decides who answers an approval
//!    from `window/visibility`; a panel that appears without telling the host
//!    would leave approvals being routed to a window the user is not looking at.
//!
//! The hotkey may legitimately fail to register (something else owns the
//! accelerator). That is reported, not fatal: the panel is still reachable from
//! the host's own settings surface, and a hard failure here would take the whole
//! session down over a key binding.

use eframe::egui;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

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

/// A registered hotkey, or the reason there is none.
pub enum Hotkey {
    /// Registered and receiving events.
    Active {
        /// Receives press events. The manager is held so the registration lives.
        _manager: Box<GlobalHotKeyManager>,
        /// The accelerator that was registered, in the host's spelling.
        spec: String,
    },
    /// Not registered; the panel is still reachable another way.
    Unavailable {
        /// Why registration failed, for the log and the host's diagnostics.
        reason: String,
        /// The accelerator that was attempted.
        spec: String,
    },
}

impl Hotkey {
    /// Register one accelerator.
    ///
    /// @param spec - the accelerator in the host's spelling, e.g. `Alt+Space`.
    /// @returns the outcome; failure is reported rather than propagated because a
    ///   taken accelerator must not cost the session.
    #[must_use]
    pub fn register(spec: &str) -> Self {
        let Some(hotkey) = parse_accelerator(spec) else {
            return Self::Unavailable {
                reason: format!("unrecognised accelerator {spec:?}"),
                spec: spec.to_owned(),
            };
        };
        match GlobalHotKeyManager::new() {
            Ok(manager) => match manager.register(hotkey) {
                Ok(()) => Self::Active { _manager: Box::new(manager), spec: spec.to_owned() },
                Err(error) => Self::Unavailable {
                    reason: format!("the accelerator is already taken or unsupported: {error}"),
                    spec: spec.to_owned(),
                },
            },
            Err(error) => Self::Unavailable {
                reason: format!("no global hotkey support on this system: {error}"),
                spec: spec.to_owned(),
            },
        }
    }

    /// Whether registration succeeded. This is what `hello` reports.
    #[must_use]
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Active { .. })
    }

    /// Why registration failed, if it did.
    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Active { .. } => None,
            Self::Unavailable { reason, .. } => Some(reason),
        }
    }

    /// The accelerator this hotkey stands for.
    #[must_use]
    pub fn spec(&self) -> &str {
        match self {
            Self::Active { spec, .. } | Self::Unavailable { spec, .. } => spec,
        }
    }

}

/// Watch the global hotkey on its own thread.
///
/// Polling is not an option here, and this is the subtlest part of the window
/// layer. eframe repaints **on demand** — its own task module says it "only
/// repaints when there are events or `request_repaint` is called" — so an idle
/// panel performs no pass at all, and a `try_recv` inside the render callback
/// would never run while the user is doing nothing. The hotkey would then appear
/// to do nothing at all, which is exactly what it did before this existed.
///
/// Blocking on the hotkey channel from a dedicated thread fixes the direction of
/// the wait: the thread sleeps until the user presses the key, then wakes the
/// render loop. `request_repaint` is safe to call from any thread.
///
/// @param wake - the app's wake channel.
/// @param egui - the render context slot, filled once the window exists.
/// @param log - where to report a watcher that ends unexpectedly.
pub fn watch_hotkey(
    wake: std::sync::mpsc::Sender<crate::app::sink::Wake>,
    egui: std::sync::Arc<std::sync::Mutex<Option<egui::Context>>>,
    log: std::sync::Arc<crate::app::sink::SharedSink>,
) {
    let _ = std::thread::Builder::new()
        .name("quorfloat-hotkey".to_owned())
        .spawn(move || {
            let receiver = GlobalHotKeyEvent::receiver();
            loop {
                // `recv` blocks until the next event; a send error means egui has
                // shut down and there is nothing left to wake.
                let Ok(event) = receiver.recv() else {
                    log.log("hotkey watcher stopped: the event channel closed");
                    return;
                };
                if event.state != HotKeyState::Pressed {
                    // The release of the previous press. Ignoring it keeps one
                    // physical press from toggling the panel twice.
                    continue;
                }
                if wake.send(crate::app::sink::Wake::Hotkey).is_err() {
                    return;
                }
                let context = match egui.lock() {
                    Ok(slot) => slot.clone(),
                    Err(poisoned) => poisoned.into_inner().clone(),
                };
                if let Some(ctx) = context {
                    ctx.request_repaint();
                }
            }
        });
}

/// Parse an accelerator in the host's spelling.
///
/// The host writes accelerators the way a user reads them (`Alt+Space`,
/// `Cmd+Shift+K`), while the hotkey library wants a modifier set and a key code.
/// This is the one place that translation happens; an unrecognised name yields
/// `None` so the caller can report it rather than registering something wrong.
///
/// @param spec - the accelerator text.
/// @returns the hotkey, or `None` if any part is unrecognised.
#[must_use]
pub fn parse_accelerator(spec: &str) -> Option<HotKey> {
    let mut modifiers = Modifiers::empty();
    let mut code = None;
    for part in spec.split('+') {
        let part = part.trim();
        if part.is_empty() {
            return None;
        }
        match part.to_ascii_lowercase().as_str() {
            "alt" | "option" | "opt" => modifiers |= Modifiers::ALT,
            "ctrl" | "control" => modifiers |= Modifiers::CONTROL,
            "shift" => modifiers |= Modifiers::SHIFT,
            // `Cmd` on macOS and `Super`/`Win` elsewhere are the same modifier to
            // the platform layer, but a user writing either means the same key.
            "cmd" | "command" | "super" | "meta" | "win" => modifiers |= Modifiers::SUPER,
            name => {
                // Only one non-modifier part is meaningful; a second one is a typo
                // and silently keeping the last would register an unexpected key.
                if code.is_some() {
                    return None;
                }
                code = Some(key_code(name)?);
            }
        }
    }
    Some(HotKey::new(Some(modifiers), code?))
}

/// Map one key name to a key code.
///
/// @param name - lower-case key name.
/// @returns the code, or `None` when the name is not one we translate.
#[must_use]
fn key_code(name: &str) -> Option<Code> {
    Some(match name {
        "space" => Code::Space,
        "enter" | "return" => Code::Enter,
        "escape" | "esc" => Code::Escape,
        "tab" => Code::Tab,
        "backspace" => Code::Backspace,
        "delete" | "del" => Code::Delete,
        "insert" => Code::Insert,
        "home" => Code::Home,
        "end" => Code::End,
        "pageup" => Code::PageUp,
        "pagedown" => Code::PageDown,
        "arrowup" | "up" => Code::ArrowUp,
        "arrowdown" | "down" => Code::ArrowDown,
        "arrowleft" | "left" => Code::ArrowLeft,
        "arrowright" | "right" => Code::ArrowRight,
        "`" | "backquote" => Code::Backquote,
        "-" | "minus" => Code::Minus,
        "=" | "equal" => Code::Equal,
        "[" | "bracketleft" => Code::BracketLeft,
        "]" | "bracketright" => Code::BracketRight,
        "\\" | "backslash" => Code::Backslash,
        ";" | "semicolon" => Code::Semicolon,
        "'" | "quote" => Code::Quote,
        "," | "comma" => Code::Comma,
        "." | "period" => Code::Period,
        "/" | "slash" => Code::Slash,
        single if single.len() == 1 && single.chars().all(|c| c.is_ascii_alphabetic()) => {
            letter_code(single.chars().next()?)?
        }
        digit if digit.len() == 1 && digit.chars().all(|c| c.is_ascii_digit()) => {
            digit_code(digit.chars().next()?)?
        }
        function if function.starts_with('f') => {
            let number: u8 = function.get(1..)?.parse().ok()?;
            function_code(number)?
        }
        _ => return None,
    })
}

/// Map `a`-`z` to its key code.
fn letter_code(letter: char) -> Option<Code> {
    Some(match letter {
        'a' => Code::KeyA, 'b' => Code::KeyB, 'c' => Code::KeyC, 'd' => Code::KeyD,
        'e' => Code::KeyE, 'f' => Code::KeyF, 'g' => Code::KeyG, 'h' => Code::KeyH,
        'i' => Code::KeyI, 'j' => Code::KeyJ, 'k' => Code::KeyK, 'l' => Code::KeyL,
        'm' => Code::KeyM, 'n' => Code::KeyN, 'o' => Code::KeyO, 'p' => Code::KeyP,
        'q' => Code::KeyQ, 'r' => Code::KeyR, 's' => Code::KeyS, 't' => Code::KeyT,
        'u' => Code::KeyU, 'v' => Code::KeyV, 'w' => Code::KeyW, 'x' => Code::KeyX,
        'y' => Code::KeyY, 'z' => Code::KeyZ,
        _ => return None,
    })
}

/// Map `0`-`9` to its key code.
fn digit_code(digit: char) -> Option<Code> {
    Some(match digit {
        '0' => Code::Digit0, '1' => Code::Digit1, '2' => Code::Digit2, '3' => Code::Digit3,
        '4' => Code::Digit4, '5' => Code::Digit5, '6' => Code::Digit6, '7' => Code::Digit7,
        '8' => Code::Digit8, '9' => Code::Digit9,
        _ => return None,
    })
}

/// Map `1`-`24` to a function key code.
fn function_code(number: u8) -> Option<Code> {
    Some(match number {
        1 => Code::F1, 2 => Code::F2, 3 => Code::F3, 4 => Code::F4, 5 => Code::F5,
        6 => Code::F6, 7 => Code::F7, 8 => Code::F8, 9 => Code::F9, 10 => Code::F10,
        11 => Code::F11, 12 => Code::F12, 13 => Code::F13, 14 => Code::F14, 15 => Code::F15,
        16 => Code::F16, 17 => Code::F17, 18 => Code::F18, 19 => Code::F19, 20 => Code::F20,
        21 => Code::F21, 22 => Code::F22, 23 => Code::F23, 24 => Code::F24,
        _ => return None,
    })
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
    fn a_simple_accelerator_parses() {
        let hotkey = parse_accelerator("Alt+Space").expect("Alt+Space is the default");
        assert_eq!(hotkey.mods, Modifiers::ALT);
        assert_eq!(hotkey.key, Code::Space);
    }

    #[test]
    fn cmd_and_super_mean_the_same_modifier() {
        // A user writing either means the platform's command key, and the host may
        // write either depending on where the setting came from.
        let from_cmd = parse_accelerator("Cmd+K").expect("Cmd+K parses");
        let from_super = parse_accelerator("Super+K").expect("Super+K parses");
        assert_eq!(from_cmd, from_super);
    }

    #[test]
    fn multiple_modifiers_combine() {
        let hotkey = parse_accelerator("Ctrl+Shift+F5").expect("parses");
        assert_eq!(hotkey.mods, Modifiers::CONTROL | Modifiers::SHIFT);
        assert_eq!(hotkey.key, Code::F5);
    }

    #[test]
    fn an_unknown_key_is_refused_rather_than_guessed() {
        // Registering the wrong key silently would leave the user pressing a
        // combination that does nothing, with no error anywhere.
        assert!(parse_accelerator("Alt+Banana").is_none());
        assert!(parse_accelerator("Alt+").is_none());
        assert!(parse_accelerator("").is_none());
        assert!(parse_accelerator("Alt+F99").is_none());
    }

    #[test]
    fn two_non_modifier_keys_are_refused() {
        assert!(parse_accelerator("Alt+K+J").is_none());
    }

    #[test]
    fn a_modifier_only_accelerator_is_refused() {
        assert!(parse_accelerator("Alt").is_none());
    }

    #[test]
    fn letters_digits_and_punctuation_are_all_translated() {
        assert_eq!(parse_accelerator("Alt+A").unwrap().key, Code::KeyA);
        assert_eq!(parse_accelerator("Alt+7").unwrap().key, Code::Digit7);
        assert_eq!(parse_accelerator("Alt+Backquote").unwrap().key, Code::Backquote);
        assert_eq!(parse_accelerator("Alt+ArrowUp").unwrap().key, Code::ArrowUp);
        assert_eq!(parse_accelerator("Alt+Semicolon").unwrap().key, Code::Semicolon);
    }

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

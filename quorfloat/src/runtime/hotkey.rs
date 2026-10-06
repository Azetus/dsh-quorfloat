//! The global hotkey: registering it, and turning a spec into a key.
//!
//! A global hotkey is the one input this panel has that the operating system owns, and
//! it is subject to rules nothing else here shares: another application may already hold
//! the combination, macOS requires registration from the main thread, and the key must
//! keep working while this process owns no focused window. All three are why the failure
//! is a *state this type reports* rather than an error it returns — a panel without a
//! hotkey still has to draw itself and say so.
//!
//! Parsing is deliberately strict. Guessing at a spec the user mistyped would register a
//! key they did not ask for, and a hotkey that steals a combination it was not given is
//! worse than one that refuses to start.

use eframe::egui;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};


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
}

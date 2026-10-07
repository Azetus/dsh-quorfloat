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


/// The accelerator this process holds, and the grab that keeps it.
///
/// **`attempted` and `held` are two fields because they are two facts.** They agree almost always, and
/// the case where they do not is the one that matters: a change the desktop refused leaves the user
/// holding their *previous* shortcut while having asked for a new one. One field cannot carry both —
/// and the first version of this tried, with the result that the reported key and the key actually
/// grabbed disagreed, so releasing "the current accelerator" released a key this process did not
/// hold. Two fields make that mistake unrepresentable.
pub struct Hotkey {
    /// The accelerator the user is asking for: what the settings page shows, and what `hello` reports.
    attempted: Option<String>,
    /// The accelerator actually grabbed, and the manager keeping it alive.
    held: Option<Held>,
    /// Why the last attempt did not become the held one, when it did not.
    reason: Option<String>,
}

/// A live grab: the accelerator, and the manager that owns the registration.
struct Held {
    /// What is really registered.
    spec: String,
    /// Real registration, or a test's claim of one.
    holder: Holder,
}

/// What is holding the grab.
///
/// The distinction exists for tests: [`Holder::Claimed`] is a registration this process really made,
/// and [`Holder::ForTest`] is one a unit test asserts about without touching the operating system.
/// Both are "held" to every reader outside this file.
/// See [`Hotkey::active_for_test`] for the second one.
enum Holder {
    /// A real registration. Dropping it releases the accelerator.
    Claimed(Box<GlobalHotKeyManager>),
    /// A registration a test claims, with no manager behind it.
    ForTest,
}

/// What this registration amounts to, for a log line or a test failure.
///
/// Hand-written because the manager behind a real grab is not `Debug`, and because the manager is the
/// least interesting part of it: "which accelerator, and is it held" is the whole question anyone
/// asks of this type, and a derived `Debug` could not answer it anyway.
impl std::fmt::Debug for Hotkey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = formatter.debug_struct("Hotkey");
        debug.field("attempted", &self.attempted);
        debug.field("held", &self.held.as_ref().map(|held| held.spec.as_str()));
        if let Some(reason) = &self.reason {
            debug.field("reason", reason);
        }
        debug.finish()
    }
}

impl Hotkey {
    /// Register one accelerator.
    ///
    /// @param spec - the accelerator in the host's spelling, e.g. `Alt+Space`.
    /// @returns the outcome; failure is reported rather than propagated because a
    ///   taken accelerator must not cost the session.
    #[must_use]
    pub fn register(spec: &str) -> Self {
        // **An empty spec is no spec.** The host's documented way of asking for no hotkey is an empty
        // string, and the difference is visible: `Some("")` makes the settings box show an empty chip —
        // a control that looks broken rather than one that says "none". The parser would refuse it
        // anyway, so the only thing an empty string could add is a misleading reason.
        let spec = spec.trim();
        let mut hotkey = Self {
            attempted: (!spec.is_empty()).then(|| spec.to_owned()),
            held: None,
            reason: None,
        };
        if !spec.is_empty() {
            hotkey.claim(spec);
        }
        hotkey
    }

    /// Register a different accelerator, releasing the current one first.
    ///
    /// **The release is explicit, and it is the whole reason this is not just another
    /// [`Hotkey::register`].** A global hotkey manager is process-wide, so registering a new
    /// accelerator while the old one is still held leaves *both* grabbed: the panel would keep
    /// answering the key the user has just replaced, and a desktop where the new choice is taken
    /// would inherit a grab nobody can see. Unregister, then register.
    ///
    /// **A refused change never costs the user their shortcut.** If the new accelerator is taken or
    /// unreadable, the old one is claimed again — so the worst case of trying a bad shortcut is a
    /// panel that says why and still opens the way it did before. In that state
    /// [`Hotkey::spec`] is the new accelerator (what the user asked for, and what the chip shows)
    /// while [`Hotkey::held_spec`] is the old one (what actually works, and what the reason names).
    ///
    /// @param self - the current registration.
    /// @param spec - the accelerator to hold instead, in the host's spelling.
    /// @returns the resulting registration; ask [`Hotkey::is_active`] and [`Hotkey::reason`].
    #[must_use]
    pub fn rebind(mut self, spec: &str) -> Self {
        let previous = self.held.take().map(|held| held.spec);
        // Release by the accelerator that is really registered, which is why `previous` is read from
        // `held` and not from `attempted`: after an earlier refusal those two differ, and releasing
        // the attempted one would leave the real grab alive.
        if let Some(previous) = &previous {
            self.release(previous);
        }
        self.attempted = Some(spec.to_owned());
        self.reason = None;
        self.claim(spec);
        if self.held.is_none() {
            if let Some(previous) = previous.filter(|spec| !spec.is_empty()) {
                let why = self.reason.take();
                self.claim(&previous);
                let restored = self.held.is_some();
                self.reason = match (restored, why) {
                    // The old shortcut works again, so the failure is about the new one. Saying
                    // "no hotkey" would be a lie: the panel is reachable exactly as before.
                    (true, Some(why)) => Some(format!("{why}（仍使用 {previous}）")),
                    (true, None) => None,
                    (false, why) => why,
                };
            }
        }
        self
    }

    /// Take the grab for one accelerator, recording why if it cannot be had.
    ///
    /// @param self - the registration being filled in; nothing may be held already.
    /// @param spec - the accelerator to claim.
    fn claim(&mut self, spec: &str) {
        debug_assert!(self.held.is_none(), "claim without releasing what is held");
        let Some(parsed) = parse_accelerator(spec) else {
            self.reason = Some(format!("读不懂这个快捷键：{spec}"));
            return;
        };
        match GlobalHotKeyManager::new() {
            Ok(manager) => match manager.register(parsed) {
                Ok(()) => {
                    self.held = Some(Held {
                        spec: spec.to_owned(),
                        holder: Holder::Claimed(Box::new(manager)),
                    });
                }
                Err(error) => {
                    self.reason = Some(format!("这个快捷键已被占用或不受支持：{error}"));
                }
            },
            Err(error) => {
                self.reason = Some(format!("本系统不支持全局快捷键：{error}"));
            }
        }
    }

    /// Let go of one accelerator.
    ///
    /// Unregistering by name *and* dropping the manager: the drop alone would release it, but the
    /// explicit call is what makes "the key is given back before the next one is asked for" true by
    /// reading, rather than true only to someone who knows how the library releases things.
    ///
    /// A [`Holder::ForTest`] is dropped, not unregistered — there is nothing behind it to release.
    ///
    /// @param self - the registration to empty.
    /// @param spec - the accelerator to give back, which must be the held one.
    fn release(&mut self, spec: &str) {
        match self.held.take() {
            Some(Held { holder: Holder::Claimed(manager), .. }) => {
                if let Some(current) = parse_accelerator(spec) {
                    let _ = manager.unregister(current);
                }
                drop(manager);
            }
            // A test's claim: nothing to give back, but it must still stop being held — otherwise a
            // test would be exercising a registration that the code under test believes is gone.
            Some(Held { holder: Holder::ForTest, .. }) | None => {}
        }
    }

    /// Whether a grab is held. This is what `hello` reports.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.held.is_some()
    }

    /// Why the last attempt did not become the held accelerator, if it did not.
    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }

    /// The accelerator the user asked for: what the settings page shows.
    ///
    /// `None` when the host asked for none at all, which is the documented escape hatch for a
    /// development profile that must not fight the desktop one for the accelerator.
    #[must_use]
    pub fn spec(&self) -> Option<&str> {
        self.attempted.as_deref()
    }

    /// The accelerator actually in use, which after a refused change is not the reported one.
    ///
    /// The settings page needs both facts: the chip shows what was asked for, so the user can see
    /// what was refused, and this is what tells them (with [`Hotkey::reason`]) what still works.
    ///
    /// @returns the accelerator in use, when one is.
    #[must_use]
    pub fn held_spec(&self) -> Option<&str> {
        self.held.as_ref().map(|held| held.spec.as_str())
    }

    /// A hotkey that *says* it is held, for tests, without touching the operating system.
    ///
    /// The success shape is the one worth testing and the one a unit test cannot otherwise reach:
    /// [`Hotkey::register`] needs a real global-hotkey manager, and the machine a test runs on is
    /// exactly the case this has to survive — an accelerator some other application already holds.
    ///
    /// @param spec - the accelerator to report as held.
    /// @returns a hotkey that reports it holds `spec`.
    #[doc(hidden)]
    #[must_use]
    pub fn active_for_test(spec: &str) -> Self {
        Self {
            attempted: Some(spec.to_owned()),
            held: Some(Held { spec: spec.to_owned(), holder: Holder::ForTest }),
            reason: None,
        }
    }

    /// A hotkey that holds nothing, for tests, with a reason a test can assert on.
    ///
    /// @param spec - the accelerator that was attempted.
    /// @param reason - why it is not held.
    /// @returns an unheld hotkey.
    #[doc(hidden)]
    #[must_use]
    pub fn unavailable_for_test(spec: &str, reason: &str) -> Self {
        Self {
            attempted: Some(spec.to_owned()),
            held: None,
            reason: Some(reason.to_owned()),
        }
    }
}

/// Serialises every test that drives the real global-hotkey backend.
///
/// **The backend is process-wide and not safe to drive from several managers at once.** On macOS,
/// creating two of them concurrently makes registration fail with `os error 0`, which reads exactly
/// like "the accelerator is taken" and is not — so a suite that does this fails against itself, and
/// the failure looks like a bug in the code under test. Cargo runs tests in threads, and more than
/// one test here needs a real grab (the panel's own tests included, because a rebind reaches the
/// operating system), so they all take this one lock.
///
/// Production is unaffected: the panel holds exactly one registration, on one thread, for the life of
/// the process.
///
/// @returns a guard to hold for as long as the test drives the backend.
#[cfg(test)]
#[must_use]
pub fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    use std::sync::{Mutex, OnceLock};
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
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

/// The accelerator a key press stands for, in the host's spelling.
///
/// **The reverse of [`parse_accelerator`], and the half that makes a shortcut recorder possible.**
/// egui reports every key press with the modifiers held at that moment — including the function keys
/// and the punctuation keys — so "press the combination you want" needs nothing from the platform
/// layer. What it needs is this: a mapping from a key and its modifiers to the same string the host
/// writes in its configuration and [`parse_accelerator`] reads back.
///
/// The names are chosen to round-trip: `parse_accelerator(accelerator_from_press(k, m))` must give the
/// same key back, which `a_recorded_chord_is_the_accelerator_the_parser_reads` checks for every key in
/// the table below.
///
/// Modifier keys pressed **alone** are their own accelerator (`Ctrl+Shift` is a legal global hotkey),
/// so they are not filtered out here — but a recorder is free to wait for a non-modifier key instead,
/// which `ui::settings` chooses to do because a chord is what a user means by "a shortcut".
///
/// @param key - the key egui reported.
/// @param modifiers - the modifiers held when it was pressed.
/// @returns the accelerator, or `None` for a key that cannot be one.
#[must_use]
pub fn accelerator_from_press(key: egui::Key, modifiers: egui::Modifiers) -> Option<String> {
    let name = key_name(key)?;
    let mut parts: Vec<&str> = Vec::new();
    // A fixed order, so two spellings of one chord cannot exist. It is also the order the host's own
    // configuration uses, which is what makes a recorded accelerator read like a written one.
    if modifiers.ctrl || modifiers.command {
        // The primary modifier: `Cmd` on macOS, `Ctrl` elsewhere. **They are different modifiers, not
        // two spellings of one** — on macOS `Ctrl` and `Cmd` are separate keys, and the parser maps
        // them to `CONTROL` and `SUPER`. So a chord recorded as `Cmd+K` is not the same chord as one
        // written `Ctrl+K`, and the recorder must not pretend otherwise.
        parts.push(if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" });
    }
    if modifiers.alt {
        parts.push("Alt");
    }
    if modifiers.shift {
        parts.push("Shift");
    }
    // On macOS `command` is the primary modifier and is already spent above; on other platforms a
    // real Super/Meta key is a modifier of its own.
    if !cfg!(target_os = "macos") && modifiers.mac_cmd {
        parts.push("Super");
    }
    parts.push(name);
    Some(parts.join("+"))
}

/// The name [`parse_accelerator`] knows for one key.
///
/// @param key - the key egui reported.
/// @returns the name, or `None` for a key that is not an accelerator.
#[must_use]
pub fn key_name(key: egui::Key) -> Option<&'static str> {
    use egui::Key as K;
    Some(match key {
        K::Space => "Space",
        K::Enter => "Enter",
        K::Tab => "Tab",
        K::Backspace => "Backspace",
        K::Delete => "Delete",
        K::Insert => "Insert",
        K::Home => "Home",
        K::End => "End",
        K::PageUp => "PageUp",
        K::PageDown => "PageDown",
        K::ArrowUp => "Up",
        K::ArrowDown => "Down",
        K::ArrowLeft => "Left",
        K::ArrowRight => "Right",
        K::Backtick => "`",
        K::Minus => "-",
        K::Equals => "=",
        K::OpenBracket => "[",
        K::CloseBracket => "]",
        K::Backslash => "\\",
        K::Semicolon => ";",
        K::Quote => "'",
        K::Comma => ",",
        K::Period => ".",
        K::Slash => "/",
        K::A => "A",
        K::B => "B",
        K::C => "C",
        K::D => "D",
        K::E => "E",
        K::F => "F",
        K::G => "G",
        K::H => "H",
        K::I => "I",
        K::J => "J",
        K::K => "K",
        K::L => "L",
        K::M => "M",
        K::N => "N",
        K::O => "O",
        K::P => "P",
        K::Q => "Q",
        K::R => "R",
        K::S => "S",
        K::T => "T",
        K::U => "U",
        K::V => "V",
        K::W => "W",
        K::X => "X",
        K::Y => "Y",
        K::Z => "Z",
        K::Num0 => "0",
        K::Num1 => "1",
        K::Num2 => "2",
        K::Num3 => "3",
        K::Num4 => "4",
        K::Num5 => "5",
        K::Num6 => "6",
        K::Num7 => "7",
        K::Num8 => "8",
        K::Num9 => "9",
        K::F1 => "F1",
        K::F2 => "F2",
        K::F3 => "F3",
        K::F4 => "F4",
        K::F5 => "F5",
        K::F6 => "F6",
        K::F7 => "F7",
        K::F8 => "F8",
        K::F9 => "F9",
        K::F10 => "F10",
        K::F11 => "F11",
        K::F12 => "F12",
        K::F13 => "F13",
        K::F14 => "F14",
        K::F15 => "F15",
        K::F16 => "F16",
        K::F17 => "F17",
        K::F18 => "F18",
        K::F19 => "F19",
        K::F20 => "F20",
        K::F21 => "F21",
        K::F22 => "F22",
        K::F23 => "F23",
        K::F24 => "F24",
        // Deliberately absent: `Escape` is how a recorder is cancelled, and the command keys
        // (`Copy`/`Cut`/`Paste`) are not keys anybody can hold down as a shortcut.
        _ => return None,
    })
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

    /// An accelerator no test process is likely to be holding, so a real grab can be taken.
    ///
    /// `rebind` needs a manager to grab with — a test double has none — so the success path below is
    /// exercised against the operating system. That is deliberate: this is the one mechanism in the
    /// panel whose failure a user cannot work around, and a fake that always succeeded would prove
    /// nothing about it. All three modifiers plus a function key makes a collision with a real desktop
    /// vanishingly unlikely, and CI machines hold nothing at all.
    const FREE_ACCELERATOR: &str = "Ctrl+Alt+Shift+F13";

    #[test]
    fn rebinding_takes_the_new_accelerator_and_lets_the_old_one_go() {
        let _serial = test_lock();
        // What the settings page does, end to end, against the real global-hotkey manager: the
        // reported accelerator changes **and** a real grab is held afterwards. The second half is the
        // one worth the trouble — a rebind that reported the new key while quietly holding nothing
        // would leave the user with a panel they cannot summon and a settings page saying all is well.
        let held = Hotkey::active_for_test("Alt+Space");
        assert!(held.is_active());
        assert_eq!(held.held_spec(), Some("Alt+Space"));

        let rebound = held.rebind(FREE_ACCELERATOR);
        assert_eq!(rebound.spec(), Some(FREE_ACCELERATOR), "the chip reports the new accelerator");
        assert!(rebound.is_active(), "and a real grab was taken: {rebound:?}");
        assert_eq!(rebound.held_spec(), Some(FREE_ACCELERATOR));
        assert_eq!(rebound.reason(), None, "with nothing to report");

        // Rebinding away from it must give it back: if the old grab were leaked, this second change
        // would fail on an accelerator that nothing else in the process is holding.
        let again = rebound.rebind("Ctrl+Alt+Shift+F14");
        assert!(again.is_active(), "the previous grab was released first: {again:?}");
    }

    #[test]
    fn a_refused_change_says_what_is_still_in_use() {
        let _serial = test_lock();
        // The message a user reads when their new shortcut did not take. It has to answer both
        // questions at once — "what did I do wrong" and "can I still open the panel" — because a
        // failed hotkey change leaves the panel reachable only by a key they are no longer sure of.
        //
        // An unreadable accelerator is the refusal this test can produce deterministically; a *taken*
        // one depends on the machine, and would make the suite fail on the desktop of whoever happens
        // to hold it. Both take the same path: report, then claim the old accelerator again.
        let refused = Hotkey::active_for_test("Alt+Space").rebind("Frobnicate+Nope");
        assert_eq!(
            refused.spec(),
            Some("Frobnicate+Nope"),
            "the chip shows what was refused, not the key it fell back to",
        );
        let reason = refused.reason().expect("the refusal is reported");
        assert!(reason.contains("Frobnicate+Nope"), "it names what was refused: {reason}");
        // The fallback ran: `Alt+Space` is a real accelerator, so it was grabbed again and the reason
        // says so. This is the half that makes the failure survivable.
        assert_eq!(refused.held_spec(), Some("Alt+Space"), "the old shortcut is live again");
        assert!(refused.is_active(), "{refused:?}");
        assert!(
            reason.contains("仍使用 Alt+Space"),
            "and the message says which key still works: {reason}",
        );
    }

    #[test]
    fn a_hotkey_that_never_registered_can_still_be_replaced() {
        let _serial = test_lock();
        // The path a real user hits when the host's configured accelerator was taken by another
        // application: the panel came up with no hotkey, and the settings page is how they get one.
        // There is nothing to release, so this must not be treated as an error.
        let none = Hotkey::unavailable_for_test("Alt+Space", "taken");
        assert!(!none.is_active());
        let rebound = none.rebind(FREE_ACCELERATOR);
        assert_eq!(rebound.spec(), Some(FREE_ACCELERATOR), "the attempt is what is reported");
        assert!(rebound.is_active(), "and this time it was had: {rebound:?}");
        // Not the stale reason: the user has moved on from it, and an explanation of a previous
        // failure sitting under a working shortcut is worse than none.
        assert_eq!(rebound.reason(), None);
    }

    #[test]
    fn every_supported_key_has_a_name_the_parser_reads_back() {
        // **The property that makes a shortcut recorder work.** Recording produces a string from a
        // key press; registration parses that string back into a key code. If the two tables disagree
        // by so much as a spelling, the panel would accept a chord the user pressed and then fail to
        // register it — the worst kind of failure, because the input looked fine.
        //
        // Every key the mapping claims to support is listed here, so a key added to the table without
        // a passing round trip fails this test rather than shipping.
        use egui::Key as K;
        let keys = [
            K::Space, K::Enter, K::Tab, K::Backspace, K::Delete, K::Insert, K::Home, K::End,
            K::PageUp, K::PageDown, K::ArrowUp, K::ArrowDown, K::ArrowLeft, K::ArrowRight,
            K::Backtick, K::Minus, K::Equals, K::OpenBracket, K::CloseBracket, K::Backslash,
            K::Semicolon, K::Quote, K::Comma, K::Period, K::Slash,
            K::A, K::B, K::C, K::D, K::E, K::F, K::G, K::H, K::I, K::J, K::K, K::L, K::M,
            K::N, K::O, K::P, K::Q, K::R, K::S, K::T, K::U, K::V, K::W, K::X, K::Y, K::Z,
            K::Num0, K::Num1, K::Num2, K::Num3, K::Num4, K::Num5, K::Num6, K::Num7, K::Num8,
            K::Num9,
            K::F1, K::F2, K::F3, K::F4, K::F5, K::F6, K::F7, K::F8, K::F9, K::F10, K::F11,
            K::F12, K::F13, K::F14, K::F15, K::F16, K::F17, K::F18, K::F19, K::F20, K::F21,
            K::F22, K::F23, K::F24,
        ];
        let bare = egui::Modifiers::NONE;
        for key in keys {
            let accelerator = accelerator_from_press(key, bare)
                .unwrap_or_else(|| panic!("{key:?} has no accelerator"));
            assert!(
                parse_accelerator(&accelerator).is_some(),
                "{key:?} recorded as {accelerator:?}, which this build cannot register",
            );
        }

        // The keys that are deliberately not accelerators, each for a reason.
        for key in [K::Escape, K::Copy, K::Cut, K::Paste] {
            assert_eq!(key_name(key), None, "{key:?} must not be recordable");
        }
    }

    #[test]
    fn a_recorded_chord_is_spelled_the_way_the_host_writes_it() {
        let all = egui::Modifiers {
            alt: true,
            ctrl: true,
            shift: true,
            mac_cmd: false,
            command: false,
        };
        // One fixed order, so one chord has one spelling: primary, then Alt, then Shift.
        let spelled = accelerator_from_press(egui::Key::K, all).expect("K is recordable");
        let expected = if cfg!(target_os = "macos") { "Cmd+Alt+Shift+K" } else { "Ctrl+Alt+Shift+K" };
        assert_eq!(spelled, expected);
        assert!(parse_accelerator(&spelled).is_some(), "and the parser reads it");

        // The parser is order-independent on the way back in, so a hand-written chord and a recorded
        // one mean the same thing when they name the same keys. (The spelling the *recorder* emits is
        // fixed, so one chord cannot have two recorded forms; this is about what it will accept.)
        let written = if cfg!(target_os = "macos") {
            "Shift+Alt+Cmd+K"
        } else {
            "Shift+Alt+Ctrl+K"
        };
        assert_eq!(parse_accelerator(written), parse_accelerator(&spelled));

        // A modifier on its own is its own accelerator: `Ctrl+Shift` really is a global hotkey.
        let modifiers_only = egui::Modifiers { shift: true, alt: true, ..egui::Modifiers::NONE };
        assert_eq!(
            accelerator_from_press(egui::Key::F13, modifiers_only).as_deref(),
            Some("Alt+Shift+F13"),
        );
    }

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

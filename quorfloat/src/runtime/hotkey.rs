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

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};


/// The accelerator this process holds, and the grab that keeps it.
///
/// **`attempted` and `held` are two fields because they are two facts.** They agree almost always, and
/// the case where they do not is the one that matters: a change the desktop refused leaves the user
/// holding their *previous* shortcut while having asked for a new one. One field cannot carry both —
/// releasing "the current accelerator" would then release a key this process does not hold. Two
/// fields make that mistake unrepresentable.
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

/// The OS registration, and the thread it belongs to.
///
/// [`GlobalHotKeyManager`] is `Send + Sync` on macOS only. Upstream asserts it there
/// (`platform_impl/macos/mod.rs`) because the macOS manager is an event handler plus mutexes; the
/// Windows manager is a hidden window instead — `CreateWindowExW` to make it, `RegisterHotKey(hwnd,
/// ..)` for the grab, `DestroyWindow` to release it — and Win32 binds a window to the thread that
/// created it, both for the message carrying the key press and for the destruction. `!Send` on
/// Windows is therefore honest, and this type is where the invariant the platform is really asking
/// for gets written down: **the grab lives on the main thread**.
///
/// That is not a new rule, it is the one this module has documented all along — macOS requires
/// registration from the main thread too — and the call sites have always obeyed it.
/// [`Hotkey::register`] runs in `main()` before the event loop; the only later change comes from
/// `set_preferences`, a synchronous Tauri command, which Tauri runs on the main thread; and the
/// watcher thread ([`watch_hotkey`]) never touches the manager, only the global event channel.
///
/// **If a thread other than the main one ever has to change the accelerator, do not widen this
/// assertion.** Send the work to the main thread instead — `AppHandle::run_on_main_thread` plus a
/// channel the caller blocks on, which is how `tauri-plugin-global-shortcut` answers the same
/// question — and let [`Grab::assert_owner`] keep catching the mistake.
struct Grab {
    /// The registration itself. Dropping it releases the accelerator.
    manager: GlobalHotKeyManager,
    /// The thread that made `manager`, so a use or a drop from another thread is caught loudly
    /// instead of being handed to a Win32 call that cannot work from there.
    owner: std::thread::ThreadId,
}

// SAFETY: the value is created, used and dropped on the thread named by `owner`, which the call
// sites pin to the main thread (see the type's documentation above). Nothing shares it across
// threads: it lives in the `Mutex<Option<Hotkey>>` in `ShellRuntime` only because Tauri's managed
// state has to be `Send`, and `Send` is all that `Mutex` asks of it — the `Sync` half of upstream's
// macOS assertion is deliberately not claimed here, because no `&Grab` ever crosses a thread. Every
// use goes through `Deref` and every drop through `Drop`, and both re-check `owner` in debug builds.
unsafe impl Send for Grab {}

impl Grab {
    /// Take a manager that has just made a registration, recording the thread that owns it.
    ///
    /// @param manager - the registration to keep alive.
    /// @returns the grab, owned by the calling thread.
    fn owning(manager: GlobalHotKeyManager) -> Self {
        Self { manager, owner: std::thread::current().id() }
    }

    /// Check that this thread is the one the grab belongs to.
    ///
    /// A `debug_assert` rather than an `assert`: the invariant is a property of the call sites, and
    /// a release build should not re-check it on every use. What it buys is that a mistake becomes a
    /// named panic while developing, instead of a Win32 call that silently does nothing or a window
    /// destroyed from a thread that does not own it.
    fn assert_owner(&self) {
        debug_assert_eq!(
            self.owner,
            std::thread::current().id(),
            "the global hotkey manager was touched by a thread other than the one that owns it",
        );
    }
}

impl std::ops::Deref for Grab {
    type Target = GlobalHotKeyManager;

    fn deref(&self) -> &Self::Target {
        self.assert_owner();
        &self.manager
    }
}

impl Drop for Grab {
    fn drop(&mut self) {
        self.assert_owner();
    }
}

/// What is holding the grab.
///
/// The distinction exists for tests: [`Holder::Claimed`] is a registration this process really made,
/// and [`Holder::ForTest`] is one a unit test asserts about without touching the operating system.
/// Both are "held" to every reader outside this file.
/// See [`Hotkey::active_for_test`] for the second one.
enum Holder {
    /// A real registration. Dropping it releases the accelerator.
    Claimed(Box<Grab>),
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
                        holder: Holder::Claimed(Box::new(Grab::owning(manager))),
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

/// Watch the global hotkey on its own thread.
///
/// Polling is not an option here: the event loop is busy with the window, and a
/// `try_recv` buried in a per-frame path would run only while something else asks
/// for frames, so the hotkey would appear to do nothing at all.
///
/// Blocking on the hotkey channel from a dedicated thread fixes the direction of
/// the wait: the thread sleeps until the user presses the key, then wakes the
/// shell's dispatcher and nudges the frontend.
///
/// @param wake - the shell's wake channel.
/// @param nudge - how to tell the frontend the state changed.
/// @param log - where to report a watcher that ends unexpectedly.
pub fn watch_hotkey(
    wake: std::sync::mpsc::Sender<crate::app::sink::Wake>,
    nudge: std::sync::Arc<dyn Fn() + Send + Sync>,
    log: std::sync::Arc<crate::app::sink::SharedSink>,
) {
    let _ = std::thread::Builder::new()
        .name("quorfloat-hotkey".to_owned())
        .spawn(move || {
            let receiver = GlobalHotKeyEvent::receiver();
            loop {
                // `recv` blocks until the next event; a send error means the shell
                // has shut down and there is nothing left to wake.
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
                nudge();
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

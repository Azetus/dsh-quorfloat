//! Entry point: handshake first, then hand the process to the Tauri shell.
//!
//! The order is not cosmetic. The host's startup budget starts when the process
//! does and it escalates on expiry, so the `hello` frame is written **before** any
//! window, webview, font, or GPU work begins. A peer that initialised its window first
//! would be killed mid-handshake on a slow machine, and the failure would look
//! like a broken binary rather than a slow one.
//!
//! Nothing here may write to stdout. The host parses stdout as protocol frames, so
//! diagnostics go through the shared sink's `log`, which writes to stderr —
//! including the panic hook installed below, because a panic message on stdout
//! would corrupt the stream on the way out.
//!
//! The shape of the shell mirrors the old egui loop, one thread per duty:
//!
//! - **reader** — owns stdin and answers frames. It needs no window to be visible,
//!   which is why the host keeps receiving answers while the panel is hidden.
//! - **dispatcher** — the only thread that touches the native window: it applies the
//!   host's `window/visibility` commands, toggles on the hotkey, reports visibility
//!   back, and ends the process when the session does.
//! - **clock** — asks the follow layer to keep its conversation current, once per
//!   interval, so a hidden panel still notices the conversation started next to it.
//! - **hotkey watcher** — blocks on the global hotkey channel and wakes the
//!   dispatcher.
//!
//! The frontend (`frontend/`) receives a nudge event after every answered frame and
//! re-renders from the session state; the bridge that carries real state is M3.

use std::process::ExitCode;
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex, MutexGuard};

use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

use dsh_quorfloat::app::geometry::WindowState;
use dsh_quorfloat::app::session::{FrameSink, HotkeyReport, Identity, Session, SessionExit, WindowCommand};
use dsh_quorfloat::app::sink::{Reader, SharedSink, Wake};
use dsh_quorfloat::app::window_settings::WindowSettings;
use dsh_quorfloat::ipc::rpc;
use dsh_quorfloat::ipc::transport::StdioSink;
use dsh_quorfloat::runtime::diag::marker::Marker;
use dsh_quorfloat::runtime::hotkey::{self, Hotkey};
use dsh_quorfloat::VERSION;

/// How often the follow layer re-checks which conversation the panel is attached to.
///
/// The reader thread answers frames; this thread is what makes the *questions* keep
/// flowing while the window is hidden, so a panel sitting idle still discovers the
/// conversation that was just started next to it.
const CLOCK_INTERVAL_MS: u64 = 1000;

/// Height of the placeholder window until the frontend reports content heights (M3).
const PLACEHOLDER_HEIGHT: f64 = 400.0;

fn main() -> ExitCode {
    install_panic_hook();
    match run() {
        Ok(exit) => match exit {
            SessionExit::ShutdownRequested | SessionExit::PeerClosed => ExitCode::SUCCESS,
        },
        Err(message) => {
            eprintln!("dsh-quorfloat: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Send panic output to stderr so it can never reach the protocol stream.
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("dsh-quorfloat panicked: {info}");
    }));
}

/// Run one session to completion.
///
/// @returns why the session ended, or a message describing a startup failure.
fn run() -> Result<SessionExit, String> {
    let settings = WindowSettings::from_env();
    // Read before the window exists: the shell restores the position from this, and
    // the same value is what it writes back at the end of the session.
    let window_state = Arc::new(Mutex::new(WindowState::load()));
    // Registered before the window exists: a taken accelerator is a normal outcome
    // and must be known before `hello` reports it, so the host shows the real state
    // rather than a key that silently does nothing.
    let hotkey = Hotkey::register(&hotkey_to_attempt());
    let hotkey_active = hotkey.is_active();

    let marker = Marker::from_env();
    marker.write("start");
    // Where the panel opens, on the record: "the window came back somewhere odd" is a
    // question about this line and the file behind it.
    marker.write(&match lock(&window_state).position() {
        Some((x, y)) => format!("window restored at {x:.0},{y:.0}"),
        None => "window position not remembered".to_owned(),
    });

    let session = Arc::new(Mutex::new(Session::new(Identity {
        quorfloat_version: VERSION.to_owned(),
        // The host's spelling, not Rust's: it compares these against its own
        // `process.platform` / `process.arch` (see `platform.rs`).
        platform: dsh_quorfloat::ipc::platform::host_platform().to_owned(),
        arch: dsh_quorfloat::ipc::platform::host_arch().to_owned(),
        hotkey: HotkeyReport {
            // Empty when the host asked for no hotkey at all, which is the documented escape hatch
            // for a development profile that must not fight the desktop one for the accelerator.
            requested: hotkey.spec().unwrap_or_default().to_owned(),
            registered: hotkey.is_active(),
        },
    })));

    // Shared with the sink, so a breadcrumb written from inside the session (an
    // approval arriving, an answer being applied) lands in the same file as the
    // lifecycle lines written here. One file, one ordering, one story.
    let sink = Arc::new(SharedSink::new(Box::new(StdioSink::new(marker.clone()))));

    // The handshake, before the Tauri app exists and therefore before any window,
    // webview, font, or GPU initialisation can delay it.
    {
        let mut session = lock(&session);
        session.start(&mut Borrowed(&sink));
    }
    if let Some(reason) = hotkey.reason() {
        sink.log(&format!("global hotkey unavailable ({reason}); the panel is still reachable from the host"));
    }

    let (wake_tx, wake_rx) = channel();
    let hotkey_wake = wake_tx.clone();
    // Set by whichever side ends the session first. The window closing and the host
    // closing the channel are different events and the exit code depends on which
    // happened, so it is recorded rather than inferred afterwards.
    let outcome: Arc<Mutex<Option<SessionExit>>> = Arc::new(Mutex::new(None));

    let shell_session = Arc::clone(&session);
    let shell_sink = Arc::clone(&sink);
    let shell_outcome = Arc::clone(&outcome);
    let shell_marker = marker.clone();
    let shell_state = Arc::clone(&window_state);
    let shell_hotkey_wake = hotkey_wake.clone();

    let app = tauri::Builder::default()
        .setup(move |app| {
            // A panel that floats over other applications is an accessory, not an
            // application: it must not take the Dock slot or the activation that a
            // document window would.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let handle = app.handle().clone();
            // The reader thread answers frames while the panel is hidden; the frontend
            // must know the state changed so it re-renders from the session.
            let nudge: Arc<dyn Fn() + Send + Sync> = {
                let handle = handle.clone();
                Arc::new(move || {
                    let _ = handle.emit("quorfloat/nudge", ());
                })
            };

            // The grab must outlive this closure: dropping the manager would release
            // the accelerator. Leaking it is deliberate — the registration is
            // process-lifetime by design, and the OS reclaims it on exit.
            let _held_hotkey: &'static mut Hotkey = Box::leak(Box::new(hotkey));

            let window = {
                // The window is created here rather than by the configuration so the
                // development hook can point the webview at a Vite dev server:
                // `DSH_QUORFLOAT_DEV_URL` selects an external page (hot reload while
                // the host spawns this binary), otherwise the embedded assets load.
                let url = match std::env::var("DSH_QUORFLOAT_DEV_URL") {
                    Ok(raw) if !raw.trim().is_empty() => match raw.trim().parse::<tauri::Url>() {
                        Ok(url) => tauri::WebviewUrl::External(url),
                        Err(error) => {
                            shell_sink.log(&format!(
                                "DSH_QUORFLOAT_DEV_URL is not a URL ({error}); using the embedded frontend",
                            ));
                            tauri::WebviewUrl::App("index.html".into())
                        }
                    },
                    _ => tauri::WebviewUrl::App("index.html".into()),
                };
                tauri::WebviewWindowBuilder::new(app, "main", url)
                    .title("quorfloat")
                    .inner_size(f64::from(settings.width), PLACEHOLDER_HEIGHT)
                    .resizable(false)
                    .maximizable(false)
                    .minimizable(false)
                    .decorations(false)
                    .transparent(true)
                    .shadow(false)
                    .skip_taskbar(true)
                    .always_on_top(settings.always_on_top)
                    .visible(false)
                    .focused(false)
                    .accept_first_mouse(true)
                    .build()
                    .map_err(|error| format!("could not create the main window: {error}"))?
            };
            // Where the user left it — or nowhere, which is a real difference rather than
            // a default: a panel that always opened at the origin would be one the user
            // moves every launch.
            if let Some((x, y)) = lock(&shell_state).position() {
                if let Err(error) = window.set_position(tauri::Position::Physical(
                    tauri::PhysicalPosition::new(x as i32, y as i32),
                )) {
                    shell_sink.log(&format!("could not restore the window position: {error}"));
                }
            }

            let reader = Reader::new(
                Arc::clone(&shell_session),
                Arc::clone(&shell_sink),
                wake_tx.clone(),
                Arc::clone(&shell_outcome),
                nudge.clone(),
            );
            std::thread::Builder::new()
                .name("quorfloat-stdin".to_owned())
                .spawn(move || reader.run())
                .map_err(|error| format!("could not start the stdin reader: {error}"))?;

            spawn_dispatcher(
                wake_rx,
                Arc::clone(&shell_session),
                Arc::clone(&shell_sink),
                shell_marker,
                handle,
                settings.start_visible,
                hotkey_active,
                Arc::clone(&shell_state),
            )
            .map_err(|error| format!("could not start the window dispatcher: {error}"))?;

            // The hotkey is watched on its own thread rather than polled: the watcher
            // blocks until the user presses the key, then wakes the dispatcher.
            if hotkey_active {
                hotkey::watch_hotkey(shell_hotkey_wake, nudge.clone(), Arc::clone(&shell_sink));
            }

            spawn_clock(
                Arc::clone(&shell_session),
                Arc::clone(&shell_sink),
                Arc::clone(&shell_outcome),
                nudge,
            )
            .map_err(|error| format!("could not start the clock thread: {error}"))?;

            Ok(())
        })
        .build(tauri::generate_context!())
        .map_err(|error| format!("could not build the shell: {error}"))?;

    app.run(|_, _| {});
    // The dispatcher exits the process with `AppHandle::exit` once the session ends;
    // reaching here means the event loop stopped without a recorded exit.
    let recorded = lock(&outcome).clone().unwrap_or(SessionExit::PeerClosed);
    Ok(recorded)
}

/// Applies window commands and the hotkey to the native window, reports visibility
/// changes, and ends the process when the session does.
///
/// The one thread that touches the window: nothing else may show or hide it, so the
/// reported visibility and the real one cannot drift apart.
#[allow(clippy::too_many_arguments)]
fn spawn_dispatcher(
    wake: Receiver<Wake>,
    session: Arc<Mutex<Session>>,
    sink: Arc<SharedSink>,
    marker: Marker,
    handle: AppHandle,
    start_visible: bool,
    hotkey_active: bool,
    window_state: Arc<Mutex<WindowState>>,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("quorfloat-window".to_owned())
        .spawn(move || {
            let Some(window) = handle.get_webview_window("main") else {
                sink.log("the main window does not exist; the panel cannot appear");
                return;
            };
            // Measured capabilities: the shell is a fact of the binary, the hotkey
            // was measured before the handshake, and the window is real by the time
            // this thread runs. Nothing here is a claim.
            let capabilities: Vec<&'static str> = if hotkey_active {
                vec!["tauri", "hotkey", "window"]
            } else {
                vec!["tauri", "window"]
            };
            let mut visible = start_visible;
            if start_visible {
                let _ = window.show();
                let _ = window.set_focus();
            }
            // The first report is the measurement the host waits for: until it
            // arrives, the host does not know whether a panel can appear at all.
            report_visibility(&session, &sink, visible, &capabilities);

            loop {
                let wake = match wake.recv() {
                    Ok(wake) => wake,
                    Err(_) => {
                        sink.log("the wake channel closed; exiting");
                        record_end(&session, &sink, &marker, &SessionExit::PeerClosed);
                        handle.exit(0);
                        return;
                    }
                };
                match wake {
                    Wake::Frames => {
                        let commands = {
                            let mut session = lock(&session);
                            session.take_window_commands()
                        };
                        let mut changed = false;
                        for command in commands {
                            changed = true;
                            apply_window_command(&window, command, &mut visible);
                        }
                        if changed {
                            report_visibility(&session, &sink, visible, &capabilities);
                        }
                    }
                    Wake::Hotkey => {
                        apply_window_command(&window, WindowCommand::Toggle, &mut visible);
                        report_visibility(&session, &sink, visible, &capabilities);
                    }
                    Wake::Exit(exit) => {
                        // Remember where the panel was — a drag that ends inside the
                        // settle window is as real as any other move — then leave.
                        if let Ok(position) = window.outer_position() {
                            let now = rpc::now_millis();
                            let mut state = lock(&window_state);
                            state.observe((position.x as f32, position.y as f32), now);
                            state.flush(now);
                        }
                        record_end(&session, &sink, &marker, &exit);
                        handle.exit(0);
                        return;
                    }
                }
            }
        })
        .map(|_| ())
}

/// Apply one parsed command to the native window.
///
/// @param window - the panel's window.
/// @param command - what the host or the hotkey asked for.
/// @param visible - the dispatcher's record of the current state.
fn apply_window_command(window: &WebviewWindow, command: WindowCommand, visible: &mut bool) {
    match command {
        WindowCommand::Show => {
            let _ = window.show();
            let _ = window.set_focus();
            *visible = true;
        }
        WindowCommand::Hide => {
            let _ = window.hide();
            *visible = false;
        }
        WindowCommand::Toggle => {
            *visible = !*visible;
            if *visible {
                let _ = window.show();
                let _ = window.set_focus();
            } else {
                let _ = window.hide();
            }
        }
    }
}

/// Wake the follow layer once per interval.
///
/// @param session - the session to pump.
/// @param sink - where the follow requests go.
/// @param outcome - set when the session ends; the thread stops seeing it.
/// @param nudge - how to tell the frontend the state changed.
fn spawn_clock(
    session: Arc<Mutex<Session>>,
    sink: Arc<SharedSink>,
    outcome: Arc<Mutex<Option<SessionExit>>>,
    nudge: Arc<dyn Fn() + Send + Sync>,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("quorfloat-clock".to_owned())
        .spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_millis(CLOCK_INTERVAL_MS));
            if lock(&outcome).is_some() {
                return;
            }
            {
                let mut session = lock(&session);
                sink.with(|sink| session.pump_follow(sink));
            }
            nudge();
        })
        .map(|_| ())
}

/// Report the panel's measured visibility and capabilities to the host.
fn report_visibility(
    session: &Arc<Mutex<Session>>,
    sink: &Arc<SharedSink>,
    visible: bool,
    capabilities: &[&str],
) {
    let session = lock(session);
    session.report_visibility(visible, capabilities, &mut Borrowed(sink));
}

/// The same facts the old shell wrote when it finished: why the session ended, and
/// whether the handshake completed. Written by the dispatcher before it exits the
/// process, because `AppHandle::exit` does not come back.
fn record_end(
    session: &Arc<Mutex<Session>>,
    sink: &Arc<SharedSink>,
    marker: &Marker,
    exit: &SessionExit,
) {
    {
        let session = lock(session);
        if session.is_ready() {
            let host = session.host_config();
            marker.write(&format!(
                "ready host={} session={}",
                host.host_version.as_deref().unwrap_or("unknown"),
                host.session_id.as_deref().unwrap_or("unknown"),
            ));
        } else {
            marker.write("not-ready");
        }
    }
    sink.log(&format!("session ended: {exit:?}"));
    marker.write(&format!("end {exit:?}"));
}

/// The accelerator to register, from the host's spawn environment.
fn configured_hotkey() -> String {
    std::env::var("DSH_QUORFLOAT_HOTKEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "Alt+Space".to_owned())
}

/// The accelerator this run should try to hold.
///
/// The rule lives in the library (`app::preferences::hotkey_to_attempt`) so it can be tested as a
/// pure function; this only supplies the two facts it needs. Reading it here rather than inside
/// `App::new` is deliberate: the grab has to be taken before the window exists, so that `hello` can
/// report what is really held instead of what was hoped for.
///
/// @returns the accelerator to attempt.
fn hotkey_to_attempt() -> String {
    let remembered = dsh_quorfloat::app::preferences::path_from_env();
    let preferences = dsh_quorfloat::app::preferences::Preferences::load(
        remembered.as_deref().unwrap_or(std::path::Path::new("")),
    );
    dsh_quorfloat::app::preferences::hotkey_to_attempt(&preferences, &configured_hotkey())
}

/// Lock a mutex, recovering from poisoning.
///
/// The guarded values are plain state; a panic elsewhere cannot leave them
/// half-updated, and refusing to continue would turn one panic into a dead session.
///
/// @param mutex - the mutex to lock.
/// @returns the guard.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Hands the shared sink to a `&mut dyn FrameSink` parameter.
struct Borrowed<'a>(&'a SharedSink);

impl FrameSink for Borrowed<'_> {
    fn send(&mut self, frame: &serde_json::Value) -> std::io::Result<()> {
        self.0.send(frame)
    }

    fn log(&mut self, line: &str) {
        self.0.log(line);
    }
}

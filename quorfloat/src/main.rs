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
//! The shape of the shell, one thread per duty:
//!
//! - **reader** — owns stdin and answers frames. It needs no window to be visible,
//!   which is why the host keeps receiving answers while the panel is hidden.
//! - **dispatcher** — the only thread that touches the native window: it applies the
//!   host's `window/visibility` commands, toggles on the hotkey, applies the host's
//!   window configuration, runs the height coordination, reports visibility back,
//!   and ends the process when the session does.
//! - **clock** — asks the follow layer to keep its conversation current, once per
//!   interval, so a hidden panel still notices the conversation started next to it.
//! - **hotkey watcher** — blocks on the global hotkey channel and wakes the
//!   dispatcher.
//!
//! The frontend (`frontend/`) is fed through one channel: after anything changes,
//! the shell emits `quorfloat/state` with the bridge's snapshot, and the page
//! re-renders from it. The page speaks back through the Tauri commands below,
//! which are thin wrappers over the bridge (see `app/bridge.rs`).

use std::process::ExitCode;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

use dsh_quorfloat::app::bridge::{self, ShellView};
use dsh_quorfloat::app::geometry::WindowState;
use dsh_quorfloat::app::height::{Height, HeightAction, SHADOW_SIDE};
use dsh_quorfloat::app::preferences::Preferences;
use dsh_quorfloat::app::session::{
    FrameSink, HotkeyReport, Identity, Session, SessionExit, WindowCommand,
};
use dsh_quorfloat::app::sink::{Reader, SharedSink, Wake};
use dsh_quorfloat::app::window_settings::WindowSettings;
use dsh_quorfloat::ipc::rpc;
use dsh_quorfloat::ipc::transport::StdioSink;
use dsh_quorfloat::runtime::diag::marker::Marker;
use dsh_quorfloat::runtime::hotkey::{self, Hotkey};
use dsh_quorfloat::VERSION;

/// How often the follow layer re-checks which conversation the panel is attached to.
const CLOCK_INTERVAL_MS: u64 = 1000;

/// How often the dispatcher checks whether the platform made room for a height request.
const HEIGHT_TICK_MS: u64 = 50;

/// Height of the placeholder window until the frontend reports content heights.
const PLACEHOLDER_HEIGHT: f64 = 400.0;

/// Everything the Tauri commands and the worker threads share.
struct ShellRuntime {
    session: Arc<Mutex<Session>>,
    sink: Arc<SharedSink>,
    view: Arc<Mutex<ShellView>>,
    hotkey: Mutex<Option<Hotkey>>,
    height_tx: Sender<Wake>,
    pinned_path: Option<std::path::PathBuf>,
    preferences_path: Option<std::path::PathBuf>,
    emit: Arc<dyn Fn() + Send + Sync>,
}

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
    let pinned_path = dsh_quorfloat::app::pinned::path_from_env();
    let preferences_path = dsh_quorfloat::app::preferences::path_from_env();
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

    // The shell's own facts, beside the session: effective settings, the hotkey as
    // the settings chip reports it, and the height coordination.
    //
    // The user's preferences are overlaid on the environment baseline at
    // **startup** — the documented contract has two call sites, this one and the
    // host-config application in the dispatcher — so the very first snapshot the
    // frontend renders already carries the user's choices.
    let preferences = preferences_path.as_deref().map(Preferences::load).unwrap_or_default();
    let view = Arc::new(Mutex::new(ShellView {
        settings: settings.clone().with_preferences(&preferences),
        hotkey_requested: hotkey.spec().map(str::to_owned),
        hotkey_held: hotkey.held_spec().map(str::to_owned),
        hotkey_registered: hotkey.is_active(),
        hotkey_reason: hotkey.reason().map(str::to_owned),
        preferences,
        visible: settings.start_visible,
        height: Height::new(),
    }));
    // The effective settings at startup, on the record: the ground truth the
    // settings page's controls are rendered from, before any host frame or
    // user click has had a chance to change them.
    {
        let view = lock(&view);
        marker.write(&format!(
            "settings initial: keepOpen={} hideOnBlur={} theme={}",
            view.preferences.keep_open.unwrap_or(false),
            view.settings.hide_on_blur,
            theme_name_for_marker(view.settings.theme),
        ));
    }

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
    let shell_view = Arc::clone(&view);
    let shell_hotkey_wake = hotkey_wake.clone();

    let app = tauri::Builder::default()
        .setup(move |app| {
            // A panel that floats over other applications is an accessory, not an
            // application: it must not take the Dock slot or the activation that a
            // document window would.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let handle = app.handle().clone();
            // One channel into the frontend: after anything changes, a fresh snapshot
            // is emitted and the page re-renders from it.
            let emit: Arc<dyn Fn() + Send + Sync> = {
                let handle = handle.clone();
                let session = Arc::clone(&shell_session);
                let view = Arc::clone(&shell_view);
                Arc::new(move || {
                    let session = lock(&session);
                    let view = lock(&view);
                    let value = bridge::snapshot(&session, &view);
                    drop(session);
                    drop(view);
                    let _ = handle.emit("quorfloat/state", value);
                })
            };

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
            let window = tauri::WebviewWindowBuilder::new(app, "main", url)
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
                .map_err(|error| format!("could not create the main window: {error}"))?;
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

            app.manage(ShellRuntime {
                session: Arc::clone(&shell_session),
                sink: Arc::clone(&shell_sink),
                view: Arc::clone(&shell_view),
                hotkey: Mutex::new(Some(hotkey)),
                height_tx: wake_tx.clone(),
                pinned_path,
                preferences_path,
                emit: emit.clone(),
            });

            let reader = Reader::new(
                Arc::clone(&shell_session),
                Arc::clone(&shell_sink),
                wake_tx.clone(),
                Arc::clone(&shell_outcome),
                emit.clone(),
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
                Arc::clone(&shell_view),
                settings.start_visible,
                hotkey_active,
                Arc::clone(&shell_state),
                emit.clone(),
            )
            .map_err(|error| format!("could not start the window dispatcher: {error}"))?;

            // The hotkey is watched on its own thread rather than polled: the watcher
            // blocks until the user presses the key, then wakes the dispatcher.
            if hotkey_active {
                hotkey::watch_hotkey(shell_hotkey_wake, emit.clone(), Arc::clone(&shell_sink));
            }

            spawn_clock(
                Arc::clone(&shell_session),
                Arc::clone(&shell_sink),
                Arc::clone(&shell_outcome),
                emit,
            )
            .map_err(|error| format!("could not start the clock thread: {error}"))?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            submit,
            cancel,
            answer_approval,
            select_session,
            start_new,
            pin,
            request_workspaces,
            select_model,
            set_permission,
            dismiss_interaction,
            dismiss_handoff,
            log,
            set_preferences,
            report_content_height,
            set_visible,
        ])
        .build(tauri::generate_context!())
        .map_err(|error| format!("could not build the shell: {error}"))?;

    app.run(|_, _| {});
    // The dispatcher exits the process with `AppHandle::exit` once the session ends;
    // reaching here means the event loop stopped without a recorded exit.
    let recorded = lock(&outcome).clone().unwrap_or(SessionExit::PeerClosed);
    Ok(recorded)
}

// ── Tauri commands ───────────────────────────────────────────────────────────

/// Run one bridge command against the session, then refresh the frontend.
///
/// The lock order is session, then sink, then view — everywhere in this file — so
/// two threads cannot hold them in opposite orders and deadlock.
fn with_session(
    state: &ShellRuntime,
    operation: impl FnOnce(&mut Session, &mut dyn FrameSink) -> Value,
) -> Value {
    let mut session = lock(&state.session);
    let result = state.sink.with(|sink| operation(&mut session, sink));
    drop(session);
    (state.emit)();
    result
}

/// `submit` — send the user's text to the followed conversation.
#[tauri::command]
fn submit(
    state: tauri::State<'_, ShellRuntime>,
    text: String,
    workspace_id: Option<String>,
) -> Value {
    with_session(&state, |session, sink| {
        bridge::submit(session, sink, &text, workspace_id.as_deref())
    })
}

/// `cancel` — stop the turn being generated.
#[tauri::command]
fn cancel(state: tauri::State<'_, ShellRuntime>) -> Value {
    with_session(&state, |session, sink| bridge::cancel(session, sink))
}

/// `answer_approval` — settle one approval card.
#[tauri::command]
fn answer_approval(
    state: tauri::State<'_, ShellRuntime>,
    interaction_id: String,
    allow: bool,
) -> Value {
    with_session(&state, |session, sink| {
        bridge::answer_approval(session, sink, &interaction_id, allow)
    })
}

/// `select_session` — follow a different conversation.
#[tauri::command]
fn select_session(state: tauri::State<'_, ShellRuntime>, session_id: String) -> Value {
    with_session(&state, |session, sink| bridge::select_session(session, sink, &session_id))
}

/// `start_new` — go back to "new conversation" mode.
#[tauri::command]
fn start_new(state: tauri::State<'_, ShellRuntime>) -> Value {
    with_session(&state, |session, sink| bridge::start_new(session, sink))
}

/// `pin` — pin one conversation (or stop pinning), remembered across runs.
#[tauri::command]
fn pin(state: tauri::State<'_, ShellRuntime>, session_id: Option<String>) -> Value {
    let pinned_path = state.pinned_path.clone();
    with_session(&state, |session, sink| {
        bridge::pin(session, sink, session_id.as_deref(), pinned_path.as_deref())
    })
}

/// `request_workspaces` — ask the host for the workspace list.
#[tauri::command]
fn request_workspaces(state: tauri::State<'_, ShellRuntime>) -> Value {
    with_session(&state, |session, sink| bridge::request_workspaces(session, sink))
}

/// `select_model` — apply one model and reasoning effort.
#[tauri::command]
fn select_model(
    state: tauri::State<'_, ShellRuntime>,
    provider: String,
    model: String,
    effort: Option<String>,
) -> Value {
    with_session(&state, |session, sink| {
        bridge::select_model(session, sink, &provider, &model, effort.as_deref())
    })
}

/// `set_permission` — apply one permission preset.
#[tauri::command]
fn set_permission(state: tauri::State<'_, ShellRuntime>, value: String) -> Value {
    with_session(&state, |session, sink| bridge::set_permission(session, sink, &value))
}

/// `dismiss_interaction` — drop one card locally.
#[tauri::command]
fn dismiss_interaction(state: tauri::State<'_, ShellRuntime>, interaction_id: String) -> Value {
    with_session(&state, |session, _| bridge::dismiss_interaction(session, &interaction_id))
}

/// `dismiss_handoff` — clear the "go to the Harness window" notice.
#[tauri::command]
fn dismiss_handoff(state: tauri::State<'_, ShellRuntime>) -> Value {
    with_session(&state, |session, _| bridge::dismiss_handoff(session))
}

/// `log` — put one frontend breadcrumb on the record.
#[tauri::command]
fn log(state: tauri::State<'_, ShellRuntime>, line: String) -> Value {
    state.sink.with(|sink| bridge::log(sink, &line))
}

/// `set_preferences` — apply what the settings page changed.
///
/// The hotkey is rebound here, on the main thread, which is where the platform
/// wants registration. A refused accelerator keeps the previous grab, exactly as
/// `Hotkey::rebind` documents.
#[tauri::command]
fn set_preferences(
    state: tauri::State<'_, ShellRuntime>,
    theme: Option<String>,
    keep_open: Option<bool>,
    hotkey: Option<String>,
) -> Result<Value, String> {
    let incoming = json!({ "theme": theme, "keepOpen": keep_open, "hotkey": hotkey });
    let mut preferences = state
        .preferences_path
        .as_deref()
        .map(Preferences::load)
        .unwrap_or_default();
    preferences = bridge::apply_preferences(preferences, &incoming)?;
    if let Some(path) = state.preferences_path.as_deref() {
        preferences.save(path);
    }
    if let Some(spec) = preferences.hotkey.clone() {
        let mut slot = lock(&state.hotkey);
        let rebound = slot
            .take()
            .unwrap_or_else(|| Hotkey::register(&spec))
            .rebind(&spec);
        {
            let mut view = lock(&state.view);
            view.hotkey_requested = rebound.spec().map(str::to_owned);
            view.hotkey_held = rebound.held_spec().map(str::to_owned);
            view.hotkey_registered = rebound.is_active();
            view.hotkey_reason = rebound.reason().map(str::to_owned);
        }
        *slot = Some(rebound);
    }
    {
        // The host's configuration is the baseline; the user's choices sit on top —
        // the same single overlay as at startup, so a preference can never be lost
        // to a settings update.
        let mut view = lock(&state.view);
        view.preferences = preferences;
        view.settings = view.settings.with_preferences(&view.preferences);
        // On the record, with the *effective* values: this is the ground truth
        // that answers "did the switch's click reach the behaviour" without a
        // window to look at.
        state.sink.mark(&format!(
            "preferences set: keepOpen={} hideOnBlur={} theme={}",
            view.preferences.keep_open.unwrap_or(false),
            view.settings.hide_on_blur,
            theme_name_for_marker(view.settings.theme),
        ));
    }
    (state.emit)();
    Ok(json!({}))
}

/// The theme's wire name, for the preferences marker.
fn theme_name_for_marker(theme: dsh_quorfloat::app::theme::Preference) -> &'static str {
    match theme {
        dsh_quorfloat::app::theme::Preference::System => "system",
        dsh_quorfloat::app::theme::Preference::Light => "light",
        dsh_quorfloat::app::theme::Preference::Dark => "dark",
    }
}

/// `report_content_height` — the frontend measured its own height; the dispatcher
/// coordinates the native window with it.
#[tauri::command]
fn report_content_height(state: tauri::State<'_, ShellRuntime>, height: f64) -> Value {
    let _ = state.height_tx.send(Wake::Height(height as f32));
    json!({})
}

/// `set_visible` — the frontend asks the native window to show or hide, after its
/// own transition has run. The dispatcher stays the only thread that touches the
/// window: this command is a request, not a direct action.
#[tauri::command]
fn set_visible(state: tauri::State<'_, ShellRuntime>, visible: bool) -> Value {
    let command = if visible { WindowCommand::Show } else { WindowCommand::Hide };
    let _ = state.height_tx.send(Wake::Visibility(command));
    json!({})
}

// ── dispatcher ───────────────────────────────────────────────────────────────

/// Applies window commands, the host's window configuration and the hotkey to the
/// native window, runs the height coordination, reports visibility changes, and
/// ends the process when the session does.
///
/// The one thread that touches the window: nothing else may show, hide, or resize
/// it, so the reported visibility and the real one cannot drift apart.
#[allow(clippy::too_many_arguments)]
fn spawn_dispatcher(
    wake: Receiver<Wake>,
    session: Arc<Mutex<Session>>,
    sink: Arc<SharedSink>,
    marker: Marker,
    handle: AppHandle,
    view: Arc<Mutex<ShellView>>,
    start_visible: bool,
    hotkey_active: bool,
    window_state: Arc<Mutex<WindowState>>,
    emit: Arc<dyn Fn() + Send + Sync>,
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
            set_view_visible(&view, visible);
            // The first report is the measurement the host waits for: until it
            // arrives, the host does not know whether a panel can appear at all.
            report_visibility(&session, &sink, visible, &capabilities);

            loop {
                match wake.recv_timeout(Duration::from_millis(HEIGHT_TICK_MS)) {
                    Ok(Wake::Frames) => {
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
                            set_view_visible(&view, visible);
                            report_visibility(&session, &sink, visible, &capabilities);
                        }
                        // The host's `ready` may have changed the window section:
                        // apply it, then re-apply the user's preferences on top.
                        apply_host_window(&window, &session, &view, &marker);
                        emit();
                    }
                    Ok(Wake::Hotkey) => {
                        if visible {
                            // The frontend plays the exit transition and asks to
                            // hide when it finishes: hiding the native window here
                            // would cut the 180ms fade short. The host can still
                            // force-hide through `window/visibility` on Frames.
                            let _ = handle.emit("quorfloat/hotkey-hide", ());
                        } else {
                            apply_window_command(&window, WindowCommand::Show, &mut visible);
                            set_view_visible(&view, visible);
                            report_visibility(&session, &sink, visible, &capabilities);
                            emit();
                        }
                    }
                    Ok(Wake::Height(panel_height)) => {
                        apply_height(&window, &view, &marker, &emit, panel_height);
                    }
                    Ok(Wake::Visibility(command)) => {
                        apply_window_command(&window, command, &mut visible);
                        set_view_visible(&view, visible);
                        report_visibility(&session, &sink, visible, &capabilities);
                        emit();
                    }
                    Ok(Wake::Exit(exit)) => {
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
                    Err(RecvTimeoutError::Timeout) => {
                        // Height housekeeping: check whether the platform made room.
                        tick_height(&window, &view, &marker, &emit);
                        // And where the panel is now: a drag is remembered once it
                        // settles (the geometry module's one-second rule), so a
                        // crash or a kill does not lose the user's placement.
                        if let Ok(position) = window.outer_position() {
                            let mut state = lock(&window_state);
                            state.observe((position.x as f32, position.y as f32), rpc::now_millis());
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => {
                        sink.log("the wake channel closed; exiting");
                        record_end(&session, &sink, &marker, &SessionExit::PeerClosed);
                        handle.exit(0);
                        return;
                    }
                }
            }
        })
        .map(|_| ())
}

/// Record the current visibility in the shell's view, for the snapshot.
///
/// The dispatcher is the only writer, and every window command goes through it,
/// so the view and the window cannot disagree.
fn set_view_visible(view: &Arc<Mutex<ShellView>>, visible: bool) {
    lock(view).visible = visible;
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

/// Apply the host's `config.window` to the effective settings and the native window.
///
/// The overlay order is the documented one: host baseline first, then the user's
/// preferences on top (`app/bridge.rs`). Only a change reaches the platform.
fn apply_host_window(
    window: &WebviewWindow,
    session: &Arc<Mutex<Session>>,
    view: &Arc<Mutex<ShellView>>,
    marker: &Marker,
) {
    let host_window = {
        let session = lock(session);
        session.host_config().window.clone()
    };
    let (changed, width, always_on_top, hide_on_blur) = {
        let mut view = lock(view);
        let before = view.settings.clone();
        view.settings.apply_host(host_window.as_ref());
        view.settings = view.settings.with_preferences(&view.preferences);
        (
            view.settings != before,
            view.settings.width,
            view.settings.always_on_top,
            view.settings.hide_on_blur,
        )
    };
    if !changed {
        return;
    }
    let native_width = f64::from(width + SHADOW_SIDE * 2.0);
    if let Ok(size) = window.inner_size() {
        if let Err(error) = window.set_size(tauri::Size::Logical(tauri::LogicalSize::new(
            native_width,
            size.height as f64,
        ))) {
            marker.write(&format!("could not apply the host's width: {error}"));
        }
    }
    if let Err(error) = window.set_always_on_top(always_on_top) {
        marker.write(&format!("could not apply the host's alwaysOnTop: {error}"));
    }
    marker.write(&format!(
        "window config applied: width={width:.0} alwaysOnTop={always_on_top} hideOnBlur={hide_on_blur}",
    ));
}

/// Accept the frontend's measured panel height and coordinate the native window.
fn apply_height(
    window: &WebviewWindow,
    view: &Arc<Mutex<ShellView>>,
    marker: &Marker,
    emit: &Arc<dyn Fn() + Send + Sync>,
    panel_height: f32,
) {
    let action = {
        let mut view = lock(view);
        view.height.retarget(panel_height, rpc::now_millis())
    };
    if let Some(HeightAction::Resize { native }) = action {
        resize(window, view, marker, native);
        emit();
    }
}

/// The periodic height check: did the platform make room for the outstanding request?
fn tick_height(
    window: &WebviewWindow,
    view: &Arc<Mutex<ShellView>>,
    marker: &Marker,
    emit: &Arc<dyn Fn() + Send + Sync>,
) {
    let action = {
        let native = window
            .inner_size()
            .map(|size| size.height as f32)
            .unwrap_or(0.0);
        let mut view = lock(view);
        view.height.tick(rpc::now_millis(), native)
    };
    match action {
        Some(HeightAction::Resize { native }) => {
            resize(window, view, marker, native);
            emit();
        }
        Some(HeightAction::Cap { available_panel }) => {
            let (target, _) = {
                let view = lock(view);
                (view.height.target(), view.settings.width)
            };
            marker.write(&format!(
                "height capped at {available_panel:.0}: asked {target:.0}, the window never made room",
            ));
            emit();
        }
        None => {}
    }
}

/// Ask the platform for one native inner size, on the record.
fn resize(
    window: &WebviewWindow,
    view: &Arc<Mutex<ShellView>>,
    marker: &Marker,
    native: f32,
) {
    let width = {
        let view = lock(view);
        f64::from(view.settings.width + SHADOW_SIDE * 2.0)
    };
    if let Err(error) =
        window.set_size(tauri::Size::Logical(tauri::LogicalSize::new(width, f64::from(native))))
    {
        marker.write(&format!("could not set the window size: {error}"));
        return;
    }
    let panel = {
        let view = lock(view);
        view.height.target()
    };
    marker.write(&format!("layout panel={panel:.0} window={width:.0}x{native:.0}"));
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
/// the app is deliberate: the grab has to be taken before the window exists, so that `hello` can
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

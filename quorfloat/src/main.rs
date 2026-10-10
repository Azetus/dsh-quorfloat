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
use dsh_quorfloat::app::geometry::{self, ScreenRect, WindowState};
use dsh_quorfloat::app::height::{Height, HeightAction, SHADOW_SIDE};
use dsh_quorfloat::app::language::{self, Language};
use dsh_quorfloat::app::preferences::{self, Preferences};
use dsh_quorfloat::app::session::interaction::parse_answers;
use dsh_quorfloat::app::session::{
    FrameSink, HotkeyReport, Identity, Session, SessionExit, WindowCommand,
};
use dsh_quorfloat::app::sink::{Reader, SharedSink, Wake};
use dsh_quorfloat::app::window_settings::WindowSettings;
use dsh_quorfloat::ipc::rpc;
use dsh_quorfloat::ipc::transport::StdioSink;
use dsh_quorfloat::app::session::follow::PanelLifecycle;
use dsh_quorfloat::runtime::diag::marker::Marker;
use dsh_quorfloat::runtime::hotkey::{self, Hotkey};
use dsh_quorfloat::runtime::tray::{self, TrayMenu};
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
    /// The live menu bar, once the platform gave us one.
    ///
    /// Shared with the dispatcher because the language can change from two directions — a
    /// settings-page write on this thread, a host configuration push on the dispatcher's —
    /// and both must re-label the same items.
    tray: Arc<Mutex<Option<TrayMenu<tauri::Wry>>>>,
    /// Where lifecycle breadcrumbs are written; the tray and language lines go here.
    marker: Marker,
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
    // The panel's own language, and the one thing about it that only happens once.
    //
    // The host resolved a language before this process started: the panel's explicit setting
    // when it has one, otherwise the Harness's own locale preference on the first launch. If
    // nothing is remembered *here*, that resolved value is adopted and written down, so every
    // later launch reads an explicit setting instead of asking the Harness again — there is no
    // follow mode. A remembered choice wins over the baseline, which is what makes a change
    // made in the panel's settings page survive the next launch.
    let mut preferences = preferences_path.as_deref().map(Preferences::load).unwrap_or_default();
    let (language, seeded) = preferences::seed_language(preferences.language, settings.language);
    if seeded {
        preferences.language = Some(language);
        if let Some(path) = preferences_path.as_deref() {
            preferences.save(path);
        }
        marker.write(&format!("language seeded: {}", language.as_str()));
    }
    // What the file holds, on the record, before the window exists: whether that position can
    // actually be applied is a question about the displays, which can only be asked once there is
    // a window to ask through — the outcome is therefore written where the window is placed.
    marker.write(&match lock(&window_state).position() {
        Some((x, y)) => format!("window position remembered at {x:.0},{y:.0}"),
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
    // The pin file, read once: its session half is adopted by the follow layer
    // (which validates it against the list it will receive), its workspace half
    // seeds the new-conversation default (P0). The two are one fact, not two —
    // see `app/pinned.rs`.
    let pinned = pinned_path
        .as_deref()
        .map(dsh_quorfloat::app::pinned::Pinned::load)
        .unwrap_or_default();
    {
        let mut session = lock(&session);
        session.adopt_pinned(pinned.session.clone());
    }

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
    //
    // `preferences` was read and language-seeded above, before the marker's first lines;
    // the effective settings reuse it so the seed is visible to the very first frame.
    let view = Arc::new(Mutex::new(ShellView {
        settings: settings.clone().with_preferences(&preferences),
        hotkey_requested: hotkey.spec().map(str::to_owned),
        hotkey_held: hotkey.held_spec().map(str::to_owned),
        hotkey_registered: hotkey.is_active(),
        hotkey_reason: hotkey.reason().map(str::to_owned),
        preferences,
        pinned_workspace: pinned.workspace.clone(),
        visible: settings.start_visible,
        height: Height::new(),
    }));
    // The effective settings at startup, on the record: the ground truth the
    // settings page's controls are rendered from, before any host frame or
    // user click has had a chance to change them.
    let initial_language = {
        let view = lock(&view);
        marker.write(&format!(
            "settings initial: keepOpen={} hideOnBlur={} theme={} language={}",
            view.preferences.keep_open.unwrap_or(false),
            view.settings.hide_on_blur,
            theme_name_for_marker(view.settings.theme),
            view.settings.language.as_str(),
        ));
        view.settings.language
    };

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
    // Created here rather than inside `setup` because two places need it: the setup that
    // installs the tray, and the dispatcher, which applies a pushed configuration (and so
    // may learn about a language change) on its own thread.
    let shell_tray: Arc<Mutex<Option<TrayMenu<tauri::Wry>>>> = Arc::new(Mutex::new(None));
    let dispatcher_tray = Arc::clone(&shell_tray);

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
                // `frontend/index.html` ships `lang="zh-CN"` because the panel's copy was
                // Chinese first; the resolved language is set here, before any page script
                // runs, so the first paint already declares the right language for font
                // selection and assistive technology.
                .initialization_script(language::document_language_script(initial_language))
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
            // Where the user left it — or the anchor, if there is nothing to restore or what was
            // remembered is on no attached display (a coordinate from a monitor that has since
            // been unplugged, which is exactly how a panel ends up created where nobody can see
            // it and the hotkey looks broken). Both facts are on the record either way.
            place_window(&window, &settings, &shell_state, &shell_sink, &shell_marker);

            app.manage(ShellRuntime {
                session: Arc::clone(&shell_session),
                sink: Arc::clone(&shell_sink),
                view: Arc::clone(&shell_view),
                hotkey: Mutex::new(Some(hotkey)),
                height_tx: wake_tx.clone(),
                pinned_path,
                preferences_path,
                tray: Arc::clone(&shell_tray),
                marker: shell_marker.clone(),
                emit: emit.clone(),
            });

            // The menu bar's choices reach the dispatcher through the same queue as the hotkey
            // and the host's commands: one thread decides what the window does.
            let tray_wake = wake_tx.clone();

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

            // The tray is built after the dispatcher exists (it is what answers the menu), so
            // it needs its own handles: the dispatcher takes ownership below.
            let tray_handle = handle.clone();
            let tray_marker = shell_marker.clone();

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
                dispatcher_tray,
                emit.clone(),
            )
            .map_err(|error| format!("could not start the window dispatcher: {error}"))?;

            // The hotkey is watched on its own thread rather than polled: the watcher
            // blocks until the user presses the key, then wakes the dispatcher.
            // The menu bar icon. A failure here is reported and survived: the panel is
            // summonable by hotkey whether or not the platform gave us a tray.
            match tray::install(&tray_handle, tray_wake, &tray_marker, initial_language) {
                Ok(menu) => *lock(&shell_tray) = Some(menu),
                Err(error) => tray_marker.write(&format!("tray could not be created: {error}")),
            }

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
            answer_question,
            select_session,
            start_new,
            pin,
            pin_workspace,
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

// ── window placement ─────────────────────────────────────────────────────────

/// Put the panel where [`geometry::placement`] decided, and say which rule placed it.
///
/// One marker line per outcome, because "the panel came back somewhere odd" and "the hotkey did
/// nothing visible" are both questions about this moment: the refusal line is the only place the
/// dropped coordinate exists once the file has been rewritten.
///
/// @param window - the panel's window, just created and still hidden.
/// @param settings - the effective settings; `anchor` is the fallback placement.
/// @param state - the remembered geometry, read here because the decision needs it.
/// @param sink - for a platform error that is worth a log line.
/// @param marker - the record of what happened.
fn place_window(
    window: &WebviewWindow,
    settings: &WindowSettings,
    state: &Arc<Mutex<WindowState>>,
    sink: &SharedSink,
    marker: &Marker,
) {
    let displays = displays_now(window);
    let primary = primary_display(window);
    // The window's own physical size, taken from the window rather than from the configured
    // width: the configuration is in logical pixels and the displays are in physical ones, and
    // on a scaled display those differ by exactly the factor that would put the centre off.
    let size = window.outer_size().map_or_else(
        |_| {
            // Unreachable in practice (the window was created a moment ago on this thread), but
            // the fallback is scaled rather than assumed to be 1:1 for the same reason.
            let scale = window.scale_factor().unwrap_or(1.0) as f32;
            (settings.width * scale, PLACEHOLDER_HEIGHT as f32 * scale)
        },
        |size| (size.width as f32, size.height as f32),
    );
    let remembered = lock(state).position();
    let placement = geometry::placement(remembered, &displays, primary, size, settings.anchor);
    if let Some((x, y)) = placement.position() {
        if let Err(error) = window.set_position(tauri::Position::Physical(
            tauri::PhysicalPosition::new(x as i32, y as i32),
        )) {
            sink.log(&format!("could not place the window: {error}"));
        }
    }
    if let Some(note) = placement.note(settings.anchor) {
        marker.write(&note);
    }
}

/// The displays the platform reports, as the plain rectangles the geometry rules take.
///
/// Translation only — no decision happens here. A query that fails becomes an empty list, which
/// the rules read as "nothing can be verified" (see [`geometry::placement`]); that is the safe
/// reading of a platform that will not answer.
///
/// @param window - any window of this application, used as the handle to the platform.
/// @returns every display's frame, in physical pixels.
fn displays_now(window: &WebviewWindow) -> Vec<ScreenRect> {
    window
        .available_monitors()
        .map(|monitors| {
            monitors
                .iter()
                .map(|monitor| {
                    ScreenRect::new(
                        monitor.position().x as f32,
                        monitor.position().y as f32,
                        monitor.size().width as f32,
                        monitor.size().height as f32,
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The primary display's usable rectangle, which is what the anchor is measured against.
///
/// The **work area** rather than the frame: its top edge is below the macOS menu bar and the
/// Windows taskbar, so that is the real top of the screen for a window — and therefore the real
/// centre too.
///
/// @param window - any window of this application.
/// @returns the primary display's work area, or `None` when the platform names no primary
///   display (the anchor then has nowhere to point and the platform's own placement stands).
fn primary_display(window: &WebviewWindow) -> Option<ScreenRect> {
    window.primary_monitor().ok().flatten().map(|monitor| {
        let area = monitor.work_area();
        ScreenRect::new(
            area.position.x as f32,
            area.position.y as f32,
            area.size.width as f32,
            area.size.height as f32,
        )
    })
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
        // The ladder's P0 rung is the shell's fact. It is read inside the closure so the lock
        // order stays session-then-view, exactly as `with_session`'s own `emit` takes it.
        let pinned = lock(&state.view).pinned_workspace.clone();
        bridge::submit(session, sink, &text, workspace_id.as_deref(), pinned.as_deref())
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

/// `answer_question` — settle one question card with the user's choices.
///
/// `answers` arrives as the frontend's array of `{ id, selected, custom? }`; it is
/// parsed with the same total parser the session validates against, so an unreadable
/// entry is refused by name rather than dropped silently.
#[tauri::command]
fn answer_question(
    state: tauri::State<'_, ShellRuntime>,
    interaction_id: String,
    answers: Value,
) -> Value {
    let answers = parse_answers(Some(&answers));
    with_session(&state, |session, sink| {
        bridge::answer_question(session, sink, &interaction_id, answers)
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
///
/// The workspace projection the bridge computed is put into the shell's view
/// *before* the snapshot is emitted, so the frontend sees the pin and its
/// projection in one state.
#[tauri::command]
fn pin(state: tauri::State<'_, ShellRuntime>, session_id: Option<String>) -> Value {
    let pinned_path = state.pinned_path.clone();
    let mut session = lock(&state.session);
    let result = state.sink.with(|sink| {
        bridge::pin(&mut session, sink, session_id.as_deref(), pinned_path.as_deref())
    });
    drop(session);
    apply_pinned_workspace(&state.view, &result);
    (state.emit)();
    result
}

/// `pin_workspace` — pin the default directory for a new conversation, leaving
/// the current one (a new-conversation-state action by design).
#[tauri::command]
fn pin_workspace(state: tauri::State<'_, ShellRuntime>, workspace_id: Option<String>) -> Value {
    let pinned_path = state.pinned_path.clone();
    let mut session = lock(&state.session);
    let result = state.sink.with(|sink| {
        bridge::pin_workspace(&mut session, sink, workspace_id.as_deref(), pinned_path.as_deref())
    });
    drop(session);
    apply_pinned_workspace(&state.view, &result);
    (state.emit)();
    result
}

/// Put the bridge's workspace projection into the shell's view.
fn apply_pinned_workspace(view: &Arc<Mutex<ShellView>>, result: &Value) {
    let mut view = lock(view);
    view.pinned_workspace = result
        .get("pinnedWorkspace")
        .and_then(Value::as_str)
        .map(str::to_owned);
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
///
/// A language change also re-labels the menu bar, in place, and puts the new tag on the
/// document element: the tray's strings and `lang` are the panel's own and follow the same
/// setting as the page's copy.
#[tauri::command]
fn set_preferences(
    state: tauri::State<'_, ShellRuntime>,
    window: WebviewWindow,
    theme: Option<String>,
    keep_open: Option<bool>,
    hotkey: Option<String>,
    language: Option<String>,
) -> Result<Value, String> {
    let incoming =
        json!({ "theme": theme, "keepOpen": keep_open, "hotkey": hotkey, "language": language });
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
        let before_language = view.settings.language;
        view.preferences = preferences;
        view.settings = view.settings.with_preferences(&view.preferences);
        // On the record, with the *effective* values: this is the ground truth
        // that answers "did the switch's click reach the behaviour" without a
        // window to look at.
        state.sink.mark(&format!(
            "preferences set: keepOpen={} hideOnBlur={} theme={} language={}",
            view.preferences.keep_open.unwrap_or(false),
            view.settings.hide_on_blur,
            theme_name_for_marker(view.settings.theme),
            view.settings.language.as_str(),
        ));
        let language = view.settings.language;
        drop(view);
        sync_tray_language(&state.tray, language, &state.marker);
        if before_language != language {
            set_document_language(&window, language, &state.marker);
        }
    }
    (state.emit)();
    Ok(json!({}))
}

/// Re-label the menu bar for a language, and record the change once.
///
/// Idempotent by construction: the tray remembers the language its items carry, so the
/// callers (a settings-page write, and the dispatcher applying a pushed configuration) can
/// both call it unconditionally without producing a marker for a value that did not change.
/// A process without a tray has nothing to re-label and writes nothing.
///
/// @param tray - the live tray, when the platform gave us one.
/// @param language - the effective language.
/// @param marker - where the one line saying the language changed is written.
fn sync_tray_language(
    tray: &Arc<Mutex<Option<TrayMenu<tauri::Wry>>>>,
    language: Language,
    marker: &Marker,
) {
    // The claim and the platform work are deliberately separate. `TrayMenu::relabel` calls
    // into the platform, which blocks until the main thread runs it; holding this lock across
    // that would let the dispatcher (relabelling a pushed configuration) and the main thread
    // (relabelling a settings-page write) wait on each other. Claiming the new language under
    // the lock makes the check-and-set atomic, so exactly one caller relabels and writes the
    // marker, and the platform calls happen with no lock held.
    let claimed = {
        let mut slot = lock(tray);
        match slot.as_mut() {
            Some(menu) if menu.language() != language => {
                menu.set_language(language);
                Some(menu.clone())
            }
            _ => None,
        }
    };
    let Some(mut menu) = claimed else { return };
    menu.relabel(language, marker);
    marker.write(&format!("language changed: {}", language.as_str()));
}

/// Put the effective language on the webview's document element.
///
/// The initialization script covers the initial load; this covers every later change, because
/// `lang` is read by the platform (font selection, hyphenation, assistive technology) and a
/// panel whose copy switched to English while its document still claimed Chinese would render
/// English text with Chinese-preferred glyphs.
///
/// @param window - the panel's window.
/// @param language - the language in force.
/// @param marker - where a refused script is recorded.
fn set_document_language(window: &WebviewWindow, language: Language, marker: &Marker) {
    if let Err(error) = window.eval(language::document_language_script(language)) {
        marker.write(&format!("could not set the document language: {error}"));
    }
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
    tray: Arc<Mutex<Option<TrayMenu<tauri::Wry>>>>,
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
            // The language the document element currently carries; `None` until the first
            // tick applies it, because the initialization script may have run before the
            // document existed.
            let mut document_language: Option<Language> = None;
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
                            apply_window_command(&window, command, &mut visible, &marker);
                        }
                        if changed {
                            set_view_visible(&view, visible);
                            report_visibility(&session, &sink, visible, &capabilities);
                        }
                        // The host's `ready` may have changed the window section:
                        // apply it, then re-apply the user's preferences on top.
                        let language = apply_host_window(&window, &session, &view, &marker, &tray);
                        if document_language != Some(language) {
                            set_document_language(&window, language, &marker);
                            document_language = Some(language);
                        }
                        emit();
                    }
                    Ok(Wake::Tray(action)) => {
                        use dsh_quorfloat::runtime::tray::TrayAction;
                        match action {
                            TrayAction::ToggleVisibility => {
                                // The hotkey's path, for the hotkey's reason: a panel that is up
                                // plays its own exit transition and asks to hide when it ends,
                                // so hiding the native window here would cut the fade short.
                                if visible {
                                    marker.write("tray menu: hide");
                                    let _ = handle.emit("quorfloat/hotkey-hide", ());
                                } else {
                                    marker.write("tray menu: show");
                                    apply_window_command(
                                        &window,
                                        WindowCommand::Show,
                                        &mut visible,
                                        &marker,
                                    );
                                    set_view_visible(&view, visible);
                                    report_visibility(&session, &sink, visible, &capabilities);
                                    emit();
                                }
                            }
                            TrayAction::Restart | TrayAction::Quit => {
                                let stop = action == TrayAction::Quit;
                                marker.write(if stop {
                                    "tray menu: exit"
                                } else {
                                    "tray menu: restart"
                                });
                                let lifecycle = if stop {
                                    PanelLifecycle::Stop
                                } else {
                                    PanelLifecycle::Restart
                                };
                                {
                                    let mut session = lock(&session);
                                    let _ = sink.with(|sink| {
                                        session.request_panel_lifecycle(lifecycle, sink)
                                    });
                                }
                                // The menu bar is ours, so choosing from it made *us* the active
                                // application. Neither of these two brings a panel up, so the
                                // keyboard goes back to whatever the user was doing (the same
                                // rule the hide path follows, see `yield_activation`).
                                yield_activation(&window, &marker);
                            }
                        }
                    }
                    Ok(Wake::Hotkey) => {
                        if visible {
                            // The frontend plays the exit transition and asks to
                            // hide when it finishes: hiding the native window here
                            // would cut the 180ms fade short. The host can still
                            // force-hide through `window/visibility` on Frames.
                            let _ = handle.emit("quorfloat/hotkey-hide", ());
                        } else {
                            apply_window_command(&window, WindowCommand::Show, &mut visible, &marker);
                            set_view_visible(&view, visible);
                            // A summon means a fresh conversation, unless the pin
                            // says "continue this one" (user decision 2026-10-09).
                            // The host's own shows (`window/visibility` on Frames)
                            // do not reset — that is the host's intent, not a summon.
                            {
                                let mut session = lock(&session);
                                let _ = sink.with(|sink| bridge::on_show(&mut session, sink));
                            }
                            report_visibility(&session, &sink, visible, &capabilities);
                            emit();
                        }
                    }
                    Ok(Wake::Height(panel_height)) => {
                        apply_height(&window, &view, &marker, &emit, panel_height);
                    }
                    Ok(Wake::Visibility(command)) => {
                        apply_window_command(&window, command, &mut visible, &marker);
                        set_view_visible(&view, visible);
                        report_visibility(&session, &sink, visible, &capabilities);
                        emit();
                    }
                    Ok(Wake::Exit(exit)) => {
                        // Remember where the panel was — a drag that ends inside the
                        // settle window is as real as any other move — then leave.
                        let now = rpc::now_millis();
                        if let Ok(position) = window.outer_position() {
                            // The displays are read fresh here rather than reused from a tick:
                            // this is the write that outlives the process, so it is judged
                            // against the arrangement that exists at the moment of quitting.
                            let displays = displays_now(&window);
                            let mut state = lock(&window_state);
                            state.observe((position.x as f32, position.y as f32), &displays, now);
                            state.flush(&displays, now);
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
                        //
                        // The display list is read next to the position rather than cached: a
                        // position is only remembered while it is on a display that exists now,
                        // and "now" is the only moment at which that can be answered. The
                        // alternative — a cached list — would keep a hot-unplugged monitor
                        // "attached" for as long as the cache lives, which is precisely the
                        // coordinate this check exists to refuse. The cost is one platform query
                        // per tick, beside the position query that was already here.
                        if let Ok(position) = window.outer_position() {
                            let displays = displays_now(&window);
                            let mut state = lock(&window_state);
                            state.observe(
                                (position.x as f32, position.y as f32),
                                &displays,
                                rpc::now_millis(),
                            );
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
fn apply_window_command(
    window: &WebviewWindow,
    command: WindowCommand,
    visible: &mut bool,
    marker: &Marker,
) {
    match command {
        WindowCommand::Show => {
            resume_activation(window);
            let _ = window.show();
            let _ = window.set_focus();
            *visible = true;
        }
        WindowCommand::Hide => {
            let _ = window.hide();
            yield_activation(window, marker);
            *visible = false;
        }
        WindowCommand::Toggle => {
            *visible = !*visible;
            if *visible {
                resume_activation(window);
                let _ = window.show();
                let _ = window.set_focus();
            } else {
                let _ = window.hide();
                yield_activation(window, marker);
            }
        }
    }
}

/// Put the application back in a state where its window can take the keyboard.
///
/// [`yield_activation`] hides the *application*, so a later `window.show()` has to lift that
/// first — otherwise the panel would come back visible but unable to take focus.
#[cfg(target_os = "macos")]
fn resume_activation(window: &WebviewWindow) {
    let _ = window.app_handle().show();
}

/// Nothing to do where the platform has no application-level hide (see below).
#[cfg(not(target_os = "macos"))]
fn resume_activation(_window: &WebviewWindow) {}

/// Hand the keyboard back to whatever the user came from.
///
/// The panel is an accessory application (`set_activation_policy(Accessory)`), the way a launcher
/// is, so summoning it deactivates whatever the user was typing in. Hiding only the *window* left
/// that application deactivated with no window of ours to type into: close the panel and the caret
/// is nowhere — the reported symptom (2026-10-09, the user's focus was in the Harness input).
/// `App::hide()` is `[NSApp hide:nil]`: it hides the application and macOS activates the
/// application that was active before it, which is the move a launcher makes.
///
/// It is safe on every path: when the panel is not the active application (the click-away blur,
/// or a hide after the user switched somewhere else) hiding an inactive application changes no
/// activation, so nobody gets pulled back.
fn yield_activation(window: &WebviewWindow, marker: &Marker) {
    #[cfg(target_os = "macos")]
    {
        marker.write("window hide: yielding activation to the previous app");
        let _ = window.app_handle().hide();
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, marker);
    }
}

/// Apply the host's `config.window` to the effective settings and the native window.
///
/// The overlay order is the documented one: host baseline first, then the user's
/// preferences on top (`app/bridge.rs`). Only a change reaches the platform.
///
/// The language is part of that push, and it is the one field that also reaches a surface
/// this process owns rather than the webview: the menu bar is re-labelled here, because a
/// host-side configuration change never goes through the settings-page command.
///
/// @returns the effective language after the overlay, so the caller can also keep the
///   document element in step without locking the view a second time.
fn apply_host_window(
    window: &WebviewWindow,
    session: &Arc<Mutex<Session>>,
    view: &Arc<Mutex<ShellView>>,
    marker: &Marker,
    tray: &Arc<Mutex<Option<TrayMenu<tauri::Wry>>>>,
) -> Language {
    let host_window = {
        let session = lock(session);
        session.host_config().window.clone()
    };
    let (changed, width, always_on_top, hide_on_blur, language) = {
        let mut view = lock(view);
        let before = view.settings.clone();
        view.settings.apply_host(host_window.as_ref());
        view.settings = view.settings.with_preferences(&view.preferences);
        (
            view.settings != before,
            view.settings.width,
            view.settings.always_on_top,
            view.settings.hide_on_blur,
            view.settings.language,
        )
    };
    // Unconditional, and idempotent: the tray itself knows the language it carries, so a
    // tick that changed nothing writes nothing.
    sync_tray_language(tray, language, marker);
    if !changed {
        return language;
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
    language
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

//! Entry point: handshake first, then hand the process to the window.
//!
//! The order is not cosmetic. The host's startup budget starts when the process
//! does and it escalates on expiry, so the `hello` frame is written **before** any
//! window, font, or GPU work begins. A peer that initialised its window first
//! would be killed mid-handshake on a slow machine, and the failure would look
//! like a broken binary rather than a slow one.
//!
//! Nothing here may write to stdout. The host parses stdout as protocol frames, so
//! diagnostics go through the shared sink's `log`, which writes to stderr —
//! including the panic hook installed below, because a panic message on stdout
//! would corrupt the stream on the way out.

use std::process::ExitCode;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};

use dsh_quorfloat::app::{self, App, Reader, SharedSink};
use dsh_quorfloat::runtime::diag::marker::Marker;
use dsh_quorfloat::app::session::{FrameSink, HotkeyReport, Identity, Session, SessionExit};
use dsh_quorfloat::ipc::transport::{StdinSource, StdioSink};
use dsh_quorfloat::window::{self, Hotkey, WindowSettings};
use dsh_quorfloat::VERSION;
use eframe::egui;

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
    // Registered before the window exists: a taken accelerator is a normal outcome
    // and must be known before `hello` reports it, so the host shows the real state
    // rather than a key that silently does nothing.
    let hotkey = Hotkey::register(&configured_hotkey());
    let hotkey_active = hotkey.is_active();

    let marker = Marker::from_env();
    marker.write("start");

    let session = Arc::new(Mutex::new(Session::new(Identity {
        quorfloat_version: VERSION.to_owned(),
        // The host's spelling, not Rust's: it compares these against its own
        // `process.platform` / `process.arch` (see `platform.rs`).
        platform: dsh_quorfloat::ipc::platform::host_platform().to_owned(),
        arch: dsh_quorfloat::ipc::platform::host_arch().to_owned(),
        hotkey: HotkeyReport {
            requested: hotkey.spec().to_owned(),
            registered: hotkey.is_active(),
        },
    })));
    // A capture is a development switch, not a product feature: it writes conversation
    // text to disk, so nothing sets it for a user. Installed here rather than read
    // inside `Session::new`, which keeps the constructor free of the environment.
    if let Ok(mut session) = session.lock() {
        session.set_dump(dsh_quorfloat::runtime::diag::dump::Dump::from_env());
    }

    // Shared with the sink, so a breadcrumb written from inside the session (an
    // approval arriving, an answer being applied) lands in the same file as the
    // lifecycle lines written here. One file, one ordering, one story.
    let sink = Arc::new(SharedSink::new(Box::new(StdioSink::new(marker.clone()))));

    // The handshake, before eframe starts and therefore before any window, font, or
    // GPU initialisation can delay it.
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

    // Where the reader thread and the window both look for the render context.
    // The reader needs it to wake a hidden window; the window fills it in on its
    // first pass. A wake delivered before that is still queued on the channel.
    let egui_slot: Arc<Mutex<Option<egui::Context>>> = Arc::new(Mutex::new(None));

    let reader = Reader::new(
        Arc::clone(&session),
        Arc::clone(&sink),
        wake_tx,
        Arc::clone(&outcome),
        Arc::clone(&egui_slot),
    );
    std::thread::Builder::new()
        .name("quorfloat-stdin".to_owned())
        .spawn(move || reader.run())
        .map_err(|error| format!("could not start the stdin reader: {error}"))?;

    // The hotkey is watched on its own thread rather than polled in the render
    // callback: eframe repaints on demand, so an idle hidden panel performs no pass
    // and a poll would never run. See `window::watch_hotkey`.
    if hotkey.is_active() {
        window::watch_hotkey(hotkey_wake, Arc::clone(&egui_slot), Arc::clone(&sink));
    }

    // The same argument, for the other kind of idle work: the panel has to notice a
    // conversation started next to it while it is hidden, and a hidden panel performs
    // no pass of its own accord. This only wakes the loop; what is due is decided by
    // the follow layer.
    if let Err(error) = app::watch_clock(
        std::time::Duration::from_millis(app::CLOCK_INTERVAL_MS),
        Arc::clone(&egui_slot),
        Arc::clone(&outcome),
    ) {
        sink.log(&format!("could not start the clock thread; the panel may miss new conversations: {error}"));
    }

    let options = eframe::NativeOptions {
        viewport: window::viewport(&settings),
        ..Default::default()
    };
    let app_session = Arc::clone(&session);
    let app_sink = Arc::clone(&sink);
    let app_outcome = Arc::clone(&outcome);
    let app_slot = Arc::clone(&egui_slot);

    eframe::run_native(
        "quorfloat",
        options,
        Box::new(move |cc| {
            // Publish the context before the first pass so the reader thread can
            // wake the loop from the moment the window exists.
            if let Ok(mut slot) = app_slot.lock() {
                *slot = Some(cc.egui_ctx.clone());
            }
            let mut app = App::new(
                app_session,
                app_sink,
                hotkey,
                settings,
                wake_rx,
                app_outcome,
                hotkey_active,
            );
            // Reaching this point *is* the measurement: eframe calls the creator
            // only after the viewport exists, so `window` stops being a claim here.
            // If creation fails, `run_native` returns an error and this is never
            // reached — the host hears nothing rather than hearing a promise.
            app.note_window_created();
            // Before the first frame: a font set installed later would lay text out
            // once with the old one and rebuild the atlas to correct it.
            let fonts = dsh_quorfloat::fonts::install(&cc.egui_ctx);
            app.note_fonts(fonts);
            Ok(Box::new(EguiApp { inner: app, context: cc.egui_ctx.clone() }))
        }),
    )
    .map_err(|error| format!("could not start the window: {error}"))?;

    finish(&session, &outcome, &marker, &sink)
}

/// Record the outcome and report why the session ended.
fn finish(
    session: &Arc<Mutex<Session>>,
    outcome: &Arc<Mutex<Option<SessionExit>>>,
    marker: &Marker,
    sink: &SharedSink,
) -> Result<SessionExit, String> {
    let exit = lock(outcome).clone().unwrap_or(SessionExit::PeerClosed);
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
    Ok(exit)
}

/// Adapts [`App`] to eframe's trait, which needs the context handed back in.
struct EguiApp {
    inner: App,
    context: egui::Context,
}

impl eframe::App for EguiApp {
    /// Runs on every pass, **including while the window is hidden**: eframe calls
    /// this through `run_logic` when there is nothing to draw. That is why every
    /// host-facing duty lives here — the panel spends most of its life hidden, and
    /// a hidden panel that stopped pumping would stop answering `ping` and be
    /// declared dead while sitting idle.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.context = ctx.clone();
        self.inner.set_context(ctx.clone());
        self.inner.logic();
        // Closing from here keeps the command inside an egui pass, which is the
        // only place a viewport command takes effect.
        self.inner.close_if_finished();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.inner.draw(ui);
    }
}

/// The accelerator to register, from the host's spawn environment.
fn configured_hotkey() -> String {
    std::env::var("DSH_QUORFLOAT_HOTKEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "Alt+Space".to_owned())
}

/// Lock a mutex, recovering from poisoning.
///
/// The guarded values are plain state; a panic elsewhere cannot leave them
/// half-updated, and refusing to continue would turn one panic into a dead session.
///
/// @param mutex - the mutex to lock.
/// @returns the guard.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
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

/// Keeps [`StdinSource`] referenced so its frame counters stay part of the crate's
/// public surface for the tests that assert on malformed input.
#[allow(dead_code)]
fn _source_is_used() -> Option<StdinSource> {
    None
}

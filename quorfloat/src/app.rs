//! The live application: one event loop driving the window and the host channel.
//!
//! The hard part is not the window or the protocol; it is that **a hidden window
//! receives no frames**. egui only runs `update` when there is something to draw,
//! so a panel that is hidden — which is its normal state — would never poll the
//! hotkey, never service the host's heartbeats, and be declared dead by the host
//! while sitting idle. Everything in this module exists to avoid that:
//!
//! - **All liveness work happens off the render loop.** A reader thread owns stdin
//!   and answers frames. It needs no window to be visible.
//! - **The reader thread can wake the loop.** It shares one `egui::Context` and
//!   calls `request_repaint` after every frame, so configuration and visibility
//!   changes are drawn even while the panel is hidden.
//! - **The main thread only renders and toggles.** It drains a channel, decides
//!   whether the panel should be on screen, and draws.
//!
//! The session state stays in [`crate::session::Session`], shared behind a mutex.
//! The reader thread is the only writer; the main thread reads it to pick up the
//! host's configuration.

use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use eframe::egui;

use crate::session::{FrameSink, Session, SessionExit};
use crate::transport::StdinSource;
use crate::window::{Hotkey, WindowSettings};

/// Something that needs the main thread's attention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wake {
    /// At least one frame arrived. The frames were already answered on the reader
    /// thread; this asks the main thread to redraw with any new configuration.
    Frames,
    /// The hotkey was pressed.
    Hotkey,
    /// The host asked this process to stop, or the channel ended.
    Exit(SessionExit),
}

/// A frame sink that can be written from several threads.
///
/// The handshake is written by the main thread before the window exists, and every
/// later frame is written by the reader thread. Both must serialise onto one
/// stdout, so the sink is shared rather than owned.
pub struct SharedSink {
    inner: Mutex<Box<dyn FrameSink + Send>>,
}

impl SharedSink {
    /// Wrap a sink for cross-thread use.
    ///
    /// @param sink - the underlying sink, normally stdout.
    #[must_use]
    pub fn new(sink: Box<dyn FrameSink + Send>) -> Self {
        Self { inner: Mutex::new(sink) }
    }

    /// Run one operation with the sink locked.
    ///
    /// A poisoned mutex is recovered rather than propagated: a panic in one writer
    /// must not turn the protocol channel into a permanent failure, and the mutex
    /// guards only a file handle, which no panic can leave half-updated.
    ///
    /// @param operation - what to do with the sink.
    /// @returns whatever the operation returned.
    pub fn with<T>(&self, operation: impl FnOnce(&mut dyn FrameSink) -> T) -> T {
        let mut guard = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        operation(guard.as_mut())
    }

    /// Write one frame.
    ///
    /// Inherent rather than only a trait method so it can take `&self`: a shared
    /// sink lives behind an `Arc`, so callers never have `&mut self` to give the
    /// trait's signature. The mutex provides the interior mutability.
    ///
    /// @param frame - the message to encode and send.
    /// @returns an error when the peer is gone.
    pub fn send(&self, frame: &serde_json::Value) -> std::io::Result<()> {
        self.with(|sink| sink.send(frame))
    }

    /// Record one log line, on stderr.
    ///
    /// @param line - text to record.
    pub fn log(&self, line: &str) {
        self.with(|sink| sink.log(line));
    }
}

impl FrameSink for SharedSink {
    fn send(&mut self, frame: &serde_json::Value) -> std::io::Result<()> {
        self.with(|sink| sink.send(frame))
    }

    fn log(&mut self, line: &str) {
        self.with(|sink| sink.log(line));
    }
}

/// The stdin pump, running on its own thread.
pub struct Reader {
    session: Arc<Mutex<Session>>,
    sink: Arc<SharedSink>,
    wake: Sender<Wake>,
    /// Records why the session ended, so the window closing and the host closing
    /// the channel do not have to be told apart afterwards.
    outcome: Arc<Mutex<Option<SessionExit>>>,
    /// Nudges the render loop. Filled in once the window exists; a wake delivered
    /// before that is still queued on the channel and picked up by the first pass.
    egui: Arc<Mutex<Option<egui::Context>>>,
}

impl Reader {
    /// Assemble a reader.
    ///
    /// @param session - the session state, shared with the main thread.
    /// @param sink - where answers go.
    /// @param wake - how to notify the main thread.
    /// @param outcome - where to record why the session ended.
    /// @param egui - the render context slot, filled once the window exists.
    #[must_use]
    pub fn new(
        session: Arc<Mutex<Session>>,
        sink: Arc<SharedSink>,
        wake: Sender<Wake>,
        outcome: Arc<Mutex<Option<SessionExit>>>,
        egui: Arc<Mutex<Option<egui::Context>>>,
    ) -> Self {
        Self { session, sink, wake, outcome, egui }
    }

    /// Read frames until the stream ends, answering each one.
    ///
    /// Blocking reads are exactly what is wanted here: this thread owns stdin, and
    /// the thread ends when the host closes it. Nothing about it needs the window
    /// to be visible, which is why the host keeps receiving answers while the panel
    /// is hidden.
    pub fn run(self) {
        let mut source = StdinSource::new();
        loop {
            let Some(frame) = source.next_frame() else {
                self.finish(SessionExit::PeerClosed);
                return;
            };
            let exit = {
                let mut session = match self.session.lock() {
                    Ok(session) => session,
                    Err(poisoned) => poisoned.into_inner(),
                };
                session.on_frame(frame, &mut BorrowedSink(&self.sink))
            };
            self.notify(Wake::Frames);
            if let Some(exit) = exit {
                self.finish(exit);
                return;
            }
        }
    }

    /// Record the ending and tell the main thread to stop.
    fn finish(&self, exit: SessionExit) {
        {
            let mut outcome = match self.outcome.lock() {
                Ok(outcome) => outcome,
                Err(poisoned) => poisoned.into_inner(),
            };
            *outcome = Some(exit.clone());
        }
        self.notify(Wake::Exit(exit));
    }

    /// Tell the main thread something happened, and ask for a redraw.
    ///
    /// A send failure means the main thread is gone, which ends the process anyway;
    /// there is nothing to recover.
    fn notify(&self, wake: Wake) {
        let _ = self.wake.send(wake);
        // Without this the panel would not redraw while hidden — and hidden is its
        // normal state, so the configuration the host sends would never be
        // reflected in what the user eventually sees.
        let context = match self.egui.lock() {
            Ok(slot) => slot.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        };
        if let Some(ctx) = context {
            ctx.request_repaint();
        }
    }
}

/// Hands the shared sink to a `&mut dyn FrameSink` parameter.
///
/// The session takes a trait object, while the shared sink is only reachable
/// behind an `Arc`; this adapter is the smallest way to bridge the two without
/// adding a second locking scheme.
struct BorrowedSink<'a>(&'a SharedSink);

impl FrameSink for BorrowedSink<'_> {
    fn send(&mut self, frame: &serde_json::Value) -> std::io::Result<()> {
        self.0.send(frame)
    }

    fn log(&mut self, line: &str) {
        self.0.log(line);
    }
}

/// The application as egui sees it.
pub struct App {
    session: Arc<Mutex<Session>>,
    sink: Arc<SharedSink>,
    hotkey: Hotkey,
    settings: WindowSettings,
    wake: Receiver<Wake>,
    /// Whether the panel is currently shown, as last commanded.
    visible: bool,
    /// Whether the host has been told the current visibility.
    reported_visible: Option<bool>,
    /// Why the session ended, if it has. Shared with the reader thread so either
    /// side can record it, and the single source of truth: a second copy here would
    /// be a second thing to keep in step, and the two would eventually disagree.
    outcome: Arc<Mutex<Option<SessionExit>>>,
    /// The render context, set on every pass. Absent until eframe calls `logic`.
    context: Option<egui::Context>,
    /// Whether the post-startup hide has been issued.
    startup_hidden: bool,
}

impl App {
    /// Assemble the application.
    ///
    /// @param session - session state shared with the reader thread.
    /// @param sink - the shared frame sink.
    /// @param hotkey - the registration outcome, success or failure.
    /// @param settings - window geometry and appearance.
    /// @param wake - receives work from the reader thread.
    #[must_use]
    pub fn new(
        session: Arc<Mutex<Session>>,
        sink: Arc<SharedSink>,
        hotkey: Hotkey,
        settings: WindowSettings,
        wake: Receiver<Wake>,
        outcome: Arc<Mutex<Option<SessionExit>>>,
    ) -> Self {
        Self {
            session,
            sink,
            hotkey,
            settings,
            wake,
            // Starts hidden to match the viewport, which is built hidden so a
            // launch (or an automatic restart) cannot flash a panel on screen.
            visible: false,
            reported_visible: None,
            outcome,
            context: None,
            startup_hidden: false,
        }
    }

    /// Record why the session ended.
    ///
    /// @param exit - the ending to remember.
    fn record(&mut self, exit: SessionExit) {
        let mut outcome = match self.outcome.lock() {
            Ok(outcome) => outcome,
            Err(poisoned) => poisoned.into_inner(),
        };
        *outcome = Some(exit);
    }

    /// Record the render context. Called on every pass, before [`App::logic`].
    ///
    /// @param ctx - egui's context for this frame.
    pub fn set_context(&mut self, ctx: egui::Context) {
        self.context = Some(ctx);
    }

    /// Re-assert the hidden state once, after the framework has shown the window.
    ///
    /// `ViewportBuilder::with_visible(false)` is not enough on its own: eframe
    /// unconditionally shows the root viewport after the first painted frame, to
    /// avoid a white flash on startup (`eframe::native::epi_integration`,
    /// `post_rendering`). Commands issued during that frame are processed after it,
    /// so hiding here lands *after* the forced show and the panel ends up hidden —
    /// which is the state it is supposed to launch in. Without this the panel
    /// appears on every launch and every automatic restart, stealing focus from
    /// whatever the user was doing.
    fn hide_after_startup(&mut self) {
        if self.startup_hidden {
            return;
        }
        self.startup_hidden = true;
        if let Some(ctx) = &self.context {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
    }

    /// Whether the session should end, and why.
    ///
    /// Reads the shared slot, so an ending recorded by the reader thread is visible
    /// here even if its wake message has not been drained yet.
    #[must_use]
    pub fn exit_requested(&self) -> Option<SessionExit> {
        match self.outcome.lock() {
            Ok(outcome) => outcome.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Whether the panel is currently shown.
    #[must_use]
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Do everything that must happen whether or not the window is drawn.
    ///
    /// eframe calls this on every pass **including while the window is hidden**,
    /// which is what makes a hidden panel still answer the host. It never blocks:
    /// the reader thread has already done the waiting, and blocking here would
    /// freeze the window.
    pub fn logic(&mut self) {
        self.hide_after_startup();
        self.pump();
        self.report_visibility();
    }

    /// Drain pending work from the reader thread.
    pub fn pump(&mut self) {
        while let Ok(wake) = self.wake.try_recv() {
            match wake {
                Wake::Hotkey => self.toggle(),
                Wake::Exit(exit) => self.record(exit),
                // The frames themselves were answered on the reader thread; this
                // only asks for a redraw with any configuration they carried.
                Wake::Frames => {}
            }
        }
        self.adopt_host_config();
    }

    /// Show or hide the panel.
    fn toggle(&mut self) {
        self.visible = !self.visible;
        if let Some(ctx) = &self.context {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(self.visible));
            // Focusing on show is what makes the panel usable from the keyboard
            // immediately; without it the user has to click into a window that
            // already has their attention.
            if self.visible {
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
        }
    }

    /// Tell the host where the panel is, when that has changed.
    ///
    /// The host routes approvals by which surface the user is looking at, so this
    /// is load-bearing rather than diagnostic. It fires once per change, not once
    /// per frame: a notification per repaint would be indistinguishable from a
    /// heartbeat and would drown the change that matters.
    pub fn report_visibility(&mut self) {
        if self.reported_visible == Some(self.visible) {
            return;
        }
        self.reported_visible = Some(self.visible);
        let visible = self.visible;
        let session = match self.session.lock() {
            Ok(session) => session,
            Err(poisoned) => poisoned.into_inner(),
        };
        session.report_visibility(visible, &mut BorrowedSink(&self.sink));
    }

    /// Pick up the window settings the host has published.
    pub fn adopt_host_config(&mut self) {
        let window = {
            let session = match self.session.lock() {
                Ok(session) => session,
                Err(poisoned) => poisoned.into_inner(),
            };
            session.host_config().window.clone()
        };
        self.settings.apply_host(window.as_ref());
    }

    /// The window settings currently in effect.
    #[must_use]
    pub fn settings(&self) -> &WindowSettings {
        &self.settings
    }
}

impl App {
    /// Draw the panel's contents.
    ///
    /// Separate from [`App::logic`] because eframe only calls this when there is
    /// something to paint. Anything the host depends on must not live here.
    ///
    /// @param ui - the root area, with no margin or background of its own.
    pub fn draw(&mut self, ui: &mut egui::Ui) {
        egui::Frame::NONE
            .fill(egui::Color32::from_rgb(24, 24, 28))
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new("quorfloat")
                        .size(15.0)
                        .color(egui::Color32::from_rgb(220, 220, 230)),
                );
                ui.add_space(4.0);
                // Say what the panel cannot do rather than showing a key that
                // silently does nothing: a taken accelerator is a normal outcome,
                // and the user needs to know to pick another one.
                let status = if self.hotkey.is_active() {
                    format!("{} 隐藏", self.hotkey.spec())
                } else {
                    format!("热键未注册：{}", self.hotkey.reason().unwrap_or("unknown"))
                };
                ui.label(
                    egui::RichText::new(status)
                        .size(12.0)
                        .color(egui::Color32::from_rgb(150, 150, 165)),
                );
            });
    }

    /// Close the window if the session is over.
    ///
    /// Called from [`App::logic`]'s caller so the close command is issued from
    /// inside an egui pass, which is the only place viewport commands take effect.
    pub fn close_if_finished(&self) {
        if self.exit_requested().is_some() {
            if let Some(ctx) = &self.context {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::Inbound;
    use crate::session::{FrameSink, HotkeyReport, Identity};
    use std::sync::mpsc::channel;

    /// A sink that records instead of writing, reachable from the test afterwards.
    #[derive(Clone, Default)]
    struct Recorded {
        frames: Arc<Mutex<Vec<serde_json::Value>>>,
        logs: Arc<Mutex<Vec<String>>>,
    }

    impl Recorded {
        fn frames(&self) -> Vec<serde_json::Value> {
            self.frames.lock().expect("not poisoned").clone()
        }

        fn logs(&self) -> Vec<String> {
            self.logs.lock().expect("not poisoned").clone()
        }
    }

    /// The sink handed to the app; writes go into a shared [`Recorded`].
    struct RecordingSink(Recorded);

    impl FrameSink for RecordingSink {
        fn send(&mut self, frame: &serde_json::Value) -> std::io::Result<()> {
            self.0.frames.lock().expect("not poisoned").push(frame.clone());
            Ok(())
        }

        fn log(&mut self, line: &str) {
            self.0.logs.lock().expect("not poisoned").push(line.to_owned());
        }
    }

    fn identity() -> Identity {
        Identity {
            quorfloat_version: "0.0.1".to_owned(),
            platform: "darwin".to_owned(),
            arch: "arm64".to_owned(),
            hotkey: HotkeyReport { requested: "Alt+Space".to_owned(), registered: false },
        }
    }

    /// Build an app whose hotkey never registered, so no real grab is attempted.
    fn app() -> (App, Recorded, Sender<Wake>) {
        let session = Arc::new(Mutex::new(Session::new(identity())));
        let recorded = Recorded::default();
        let sink = Arc::new(SharedSink::new(Box::new(RecordingSink(recorded.clone()))));
        let (tx, rx) = channel();
        let hotkey = Hotkey::Unavailable { reason: "test".to_owned(), spec: "Alt+Space".to_owned() };
        let outcome = Arc::new(Mutex::new(None));
        let app = App::new(session, sink, hotkey, WindowSettings::default(), rx, outcome);
        (app, recorded, tx)
    }

    #[test]
    fn the_panel_starts_hidden_and_says_so() {
        // A panel that appeared without reporting would leave approvals routed to a
        // window nobody is looking at.
        let (mut app, recorded, _tx) = app();
        app.pump();
        app.report_visibility();
        let frames = recorded.frames();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0]["method"], "window/visibility");
        assert_eq!(frames[0]["params"]["visible"], false);
    }

    #[test]
    fn visibility_is_reported_once_per_change_not_once_per_frame() {
        // egui calls `update` continuously while the panel is up; without the guard
        // the host would receive a notification per repaint.
        let (mut app, recorded, _tx) = app();
        for _ in 0..5 {
            app.report_visibility();
        }
        assert_eq!(recorded.frames().len(), 1, "five frames produced one report");
    }

    #[test]
    fn a_hotkey_press_toggles_and_is_reported() {
        let (mut app, recorded, tx) = app();
        app.report_visibility();
        tx.send(Wake::Hotkey).expect("the app is listening");
        app.pump();
        assert!(app.is_visible());
        app.report_visibility();
        let frames = recorded.frames();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[1]["params"]["visible"], true);
    }

    #[test]
    fn the_panel_hides_itself_once_after_startup() {
        // eframe unconditionally shows the root viewport after the first painted
        // frame, so `with_visible(false)` alone leaves the panel on screen at
        // launch. The hide has to be re-issued, and exactly once — repeating it
        // every pass would fight the hotkey.
        let (mut app, _recorded, _tx) = app();
        assert!(!app.startup_hidden);
        app.logic();
        assert!(app.startup_hidden, "the hide was issued on the first pass");
        // A second pass must not issue it again: by then the user may have pressed
        // the hotkey, and re-hiding would make the panel impossible to open.
        app.logic();
        assert!(app.startup_hidden);
        assert!(!app.is_visible());
    }

    #[test]
    fn a_frame_wake_does_not_toggle_anything() {
        // The reader thread wakes on every frame it answers. Treating those as
        // hotkey presses would make the panel appear and vanish during heartbeats.
        let (mut app, _recorded, tx) = app();
        tx.send(Wake::Frames).expect("listening");
        app.pump();
        assert!(!app.is_visible());
    }

    #[test]
    fn an_exit_wake_ends_the_session_after_the_current_frame() {
        // The host's `shutdown` is answered on the reader thread; the window closes
        // here so the process exits through its normal path rather than by signal.
        let (mut app, _recorded, tx) = app();
        assert!(app.exit_requested().is_none());
        tx.send(Wake::Exit(SessionExit::ShutdownRequested)).expect("listening");
        app.pump();
        assert_eq!(app.exit_requested(), Some(SessionExit::ShutdownRequested));
    }

    #[test]
    fn the_wake_channel_never_blocks_the_render_loop() {
        // A blocking receive here would freeze the window whenever the host was
        // quiet, which is most of the time.
        let (mut app, _recorded, _tx) = app();
        let started = std::time::Instant::now();
        for _ in 0..100 {
            app.pump();
        }
        assert!(started.elapsed() < std::time::Duration::from_millis(500));
    }

    #[test]
    fn the_host_ready_payload_reaches_the_app() {
        // The session stores what `ready` carried; the app copies it into the
        // settings it draws with, so the window is the configured size.
        let (mut app, _recorded, _tx) = app();
        {
            let mut session = app.session.lock().expect("not poisoned");
            let mut sink = RecordingSink(Recorded::default());
            session.on_frame(
                Inbound::Notification {
                    method: "ready".to_owned(),
                    params: Some(serde_json::json!({"config": {"window": {"width": 512}}})),
                },
                &mut sink,
            );
        }
        app.pump();
        assert_eq!(app.settings().width, 512.0);
    }

    #[test]
    fn a_shared_sink_serialises_threads_without_losing_frames() {
        // Both the main thread (handshake) and the reader thread (answers) write.
        // Losing either would show up as a handshake timeout under load.
        let recorded = Recorded::default();
        let sink = Arc::new(SharedSink::new(Box::new(RecordingSink(recorded.clone()))));
        let mut handles = Vec::new();
        for index in 0..4 {
            let sink = Arc::clone(&sink);
            handles.push(std::thread::spawn(move || {
                for _ in 0..25 {
                    sink.send(&serde_json::json!({ "from": index })).expect("write succeeds");
                }
            }));
        }
        for handle in handles {
            handle.join().expect("no thread panics");
        }
        assert_eq!(recorded.frames().len(), 100);
    }

    #[test]
    fn a_poisoned_sink_still_serves() {
        // A panic in one writer must not turn the protocol channel into a permanent
        // failure: the mutex guards a file handle, which no panic can leave
        // half-updated.
        let recorded = Recorded::default();
        let sink = Arc::new(SharedSink::new(Box::new(RecordingSink(recorded.clone()))));
        let poisoner = Arc::clone(&sink);
        let _ = std::thread::spawn(move || {
            poisoner.with(|_| panic!("deliberate"));
        })
        .join();
        sink.send(&serde_json::json!({ "after": "poison" })).expect("write succeeds");
        assert_eq!(recorded.frames().len(), 1);
    }

    #[test]
    fn a_borrowed_sink_forwards_to_the_shared_one() {
        // The adapter is what lets the session keep taking `&mut dyn FrameSink`
        // while the real sink is shared behind an `Arc`.
        let recorded = Recorded::default();
        let sink = SharedSink::new(Box::new(RecordingSink(recorded.clone())));
        let mut borrowed = BorrowedSink(&sink);
        borrowed.send(&serde_json::json!({"a": 1})).expect("write succeeds");
        borrowed.log("hello");
        assert_eq!(recorded.frames().len(), 1);
        assert_eq!(recorded.logs(), vec!["hello".to_owned()]);
    }
}

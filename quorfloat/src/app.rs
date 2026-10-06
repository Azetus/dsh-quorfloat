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

use crate::fonts::{self, FontStatus};
use crate::session::{ApprovalVerdict, FrameSink, Handoff, Interaction, InteractionKind, InteractionState, Session, SessionExit, WindowCommand};
use crate::ipc::transport::StdinSource;
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

    /// Record one durable breadcrumb.
    ///
    /// Forwarded explicitly rather than left to the trait's default: the default is
    /// a no-op, so forgetting this hop makes the marker file exist and stay empty —
    /// which reads as "nothing happened" rather than as "the plumbing is missing".
    ///
    /// @param line - text to record.
    pub fn mark(&self, line: &str) {
        self.with(|sink| sink.mark(line));
    }
}

impl FrameSink for SharedSink {
    fn send(&mut self, frame: &serde_json::Value) -> std::io::Result<()> {
        self.with(|sink| sink.send(frame))
    }

    fn log(&mut self, line: &str) {
        self.with(|sink| sink.log(line));
    }

    fn mark(&mut self, line: &str) {
        self.with(|sink| sink.mark(line));
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

    fn mark(&mut self, line: &str) {
        self.0.mark(line);
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
    /// What this process has established it can do, measured rather than declared.
    ///
    /// `hotkey` is known before the handshake (registration is synchronous);
    /// `window` is added only once eframe has actually created one. A capability
    /// reported before it is real would have the host promise the user a panel that
    /// never appears.
    capabilities: Vec<&'static str>,
    /// What to tell the user about the fonts, when there is something to tell.
    ///
    /// A panel that cannot draw its own text has to say so: silent boxes look like a
    /// rendering bug in the conversation rather than a missing file.
    fonts_warning: Option<String>,
    /// Whether the glyph check has run. It cannot run until a pass has happened,
    /// because a font set does not exist before then.
    fonts_checked: bool,
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
        hotkey_active: bool,
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
            capabilities: measured_capabilities(hotkey_active),
            fonts_warning: None,
            fonts_checked: false,
        }
    }

    /// Record that the window was created, which is the moment `window` stops being
    /// a claim and becomes a fact.
    ///
    /// Called from eframe's app creator, so it runs only after the viewport exists.
    /// Nothing later can un-create it: a window that is closed ends the process.
    pub fn note_window_created(&mut self) {
        if !self.capabilities.contains(&"window") {
            self.capabilities.push("window");
        }
    }

    /// What this process can actually do.
    #[must_use]
    pub fn capabilities(&self) -> &[&'static str] {
        &self.capabilities
    }

    /// Record what the font installation produced.
    ///
    /// Called from eframe's app creator, before the first frame, so a missing font is
    /// visible in the very first paint rather than discovered by a user reading boxes.
    ///
    /// @param status - what [`crate::fonts::install`] found and loaded.
    pub fn note_fonts(&mut self, status: FontStatus) {
        self.sink.log(&status.describe());
        self.sink.mark(&status.describe());
        self.fonts_warning = status.warning();
    }

    /// Check, once, that the panel can actually draw its own text.
    ///
    /// Installing a font is a claim about the file; this is the measurement, taken
    /// through egui's own glyph lookup. It runs on the first pass because there is no
    /// font set before that — and a claim that never gets checked is how a panel ends
    /// up shipping boxes.
    fn verify_fonts(&mut self) {
        if self.fonts_checked {
            return;
        }
        let Some(ctx) = self.context.clone() else { return };
        self.fonts_checked = true;
        if fonts::covers_panel_text(&ctx) {
            self.sink.log("fonts: the panel's own text has glyphs");
            self.sink.mark("fonts coverage=ok");
            return;
        }
        // Either nothing was loaded, or something was and it does not carry these
        // characters. Both are worth saying out loud, and the second one is the reason
        // this check exists at all.
        let message = "中文无法显示：字体未生效".to_owned();
        self.sink.log("fonts: the panel's own text has no glyphs");
        self.sink.mark("fonts coverage=missing");
        self.fonts_warning = Some(message);
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
        self.verify_fonts();
        self.pump();
        self.apply_window_commands();
        self.pump_follow();
        self.report_visibility();
    }

    /// Keep the followed conversation current.
    ///
    /// Not driven by an incoming frame, unlike everything in [`App::pump`]: a panel
    /// with nothing arriving still has to notice the conversation that was started
    /// beside it. [`watch_clock`] is what makes this run at all while the panel is
    /// hidden — which is its normal state.
    pub fn pump_follow(&mut self) {
        let mut session = match self.session.lock() {
            Ok(session) => session,
            Err(poisoned) => poisoned.into_inner(),
        };
        session.pump_follow(&mut BorrowedSink(&self.sink));
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

    /// Flip the panel's visibility.
    fn toggle(&mut self) {
        self.set_visible(!self.visible);
    }

    /// Show or hide the panel.
    ///
    /// The single place visibility changes, so the global hotkey and a command from
    /// the host cannot drift apart in what they do — including the focus behaviour,
    /// which is easy to remember in one path and forget in the other.
    ///
    /// @param visible - the state to move to. Setting the state the panel is
    ///   already in still re-issues the command, which is deliberate: after startup
    ///   the framework has shown the window against this object's wishes, so
    ///   re-asserting "hidden" is how that gets corrected.
    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
        if let Some(ctx) = &self.context {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(visible));
            // Focusing on show is what makes the panel usable from the keyboard
            // immediately; without it the user has to click into a window that
            // already has their attention.
            if visible {
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
        }
    }

    /// Apply every window command the host has sent.
    ///
    /// Drained through the session, so the command path is the same whether the
    /// request came from the host or from the hotkey. Applying in order matters:
    /// a show followed by a hide in the same batch must end hidden.
    pub fn apply_window_commands(&mut self) {
        let commands = {
            let mut session = match self.session.lock() {
                Ok(session) => session,
                Err(poisoned) => poisoned.into_inner(),
            };
            session.take_window_commands()
        };
        for command in commands {
            match command {
                WindowCommand::Show => self.set_visible(true),
                WindowCommand::Hide => self.set_visible(false),
                WindowCommand::Toggle => self.toggle(),
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
        let capabilities = self.capabilities.clone();
        session.report_visibility(visible, &capabilities, &mut BorrowedSink(&self.sink));
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

    /// Read the interactions, the hand-off, and the follow state under one short lock.
    ///
    /// Copied rather than borrowed because the alternative is holding the session
    /// mutex across a paint, and the reader thread needs that same mutex to answer
    /// `ping`. A stalled liveness answer is how the host decides this process is
    /// dead, so the window must never be able to cause one. The copy is a handful of
    /// small values, and only while the panel is actually being painted.
    ///
    /// @returns what the window renders this pass.
    #[must_use]
    fn state(&self) -> PanelState {
        let session = match self.session.lock() {
            Ok(session) => session,
            Err(poisoned) => poisoned.into_inner(),
        };
        let follow = session.follow();
        let transcript = session.transcript();
        PanelState {
            interactions: session.interactions().to_vec(),
            handoff: session.handoff().cloned(),
            entries: transcript.shared_entries(),
            live: transcript.live_entry(),
            title: transcript.title().map(str::to_owned),
            follow: FollowView {
                session_id: follow.session_id().map(str::to_owned),
                label: follow.label().map(str::to_owned),
                started: follow.is_started(),
                events: follow.events(),
                resyncs: follow.resyncs(),
            },
        }
    }

    /// Answer one approval, or drop a card the user has finished with.
    ///
    /// The only path from a click to the protocol, so the two cannot drift: the
    /// state machine that decides whether an answer is allowed lives in the session,
    /// and this only supplies the sink it must write through.
    ///
    /// @param action - what the user asked for.
    fn apply_card_action(&mut self, action: CardAction) {
        let mut session = match self.session.lock() {
            Ok(session) => session,
            Err(poisoned) => poisoned.into_inner(),
        };
        let mut sink = BorrowedSink(&self.sink);
        match action {
            CardAction::Answer { id, verdict } => {
                session.answer_interaction(&id, verdict, &mut sink);
            }
            CardAction::Dismiss { id } => {
                session.dismiss_interaction(&id);
            }
            CardAction::DismissHandoff => session.dismiss_handoff(),
        }
    }
}

/// What a painted card asked the user to do.
#[derive(Debug, Clone, PartialEq, Eq)]
enum CardAction {
    /// Answer one approval.
    Answer {
        /// Which interaction.
        id: String,
        /// What the user decided.
        verdict: ApprovalVerdict,
    },
    /// Stop showing a card.
    Dismiss {
        /// Which interaction.
        id: String,
    },
    /// Stop showing the hand-off banner.
    DismissHandoff,
}

/// Everything the window renders, read under one short lock.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PanelState {
    /// Requests waiting for the user, oldest first.
    interactions: Vec<Interaction>,
    /// The last hand-off the host announced, if any.
    handoff: Option<Handoff>,
    /// The conversation this panel follows.
    follow: FollowView,
    /// The conversation so far, oldest first.
    ///
    /// Shared, not cloned: this is read on every repaint and may hold megabytes.
    entries: std::sync::Arc<Vec<crate::transcript::Entry>>,
    /// The assistant message being generated, when one is.
    live: Option<crate::transcript::Entry>,
    /// The harness's name for this conversation, when it has chosen one.
    title: Option<String>,
}

/// What the status line needs to know about the followed conversation.
///
/// Shown because two states look identical from every other angle: "attached to
/// nothing" and "attached and receiving nothing". The first one means no approval can
/// ever reach this panel, so it must not be invisible.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FollowView {
    /// The conversation followed, if any.
    session_id: Option<String>,
    /// The workspace name that came with it.
    label: Option<String>,
    /// Whether discovery has started at all.
    started: bool,
    /// Persistent events received for it.
    events: u64,
    /// Times the host reported a gap.
    resyncs: u64,
}

impl FollowView {
    /// The line to show under the header.
    #[must_use]
    fn status(&self) -> String {
        match (&self.session_id, self.started) {
            (Some(id), _) => {
                // Everything the harness calls a session starts with `session-`, so the
                // first eight characters of one are the word "session". Take from after
                // the prefix, which is the part that actually distinguishes two of them.
                let short: String = id.strip_prefix("session-").unwrap_or(id).chars().take(8).collect();
                let label = self.label.as_deref().unwrap_or("会话");
                let gap = if self.resyncs == 0 {
                    String::new()
                } else {
                    format!(" · 补齐 {} 次", self.resyncs)
                };
                format!("跟随 {label} · {short} · 事件 {}{gap}", self.events)
            }
            // Started but nothing to follow: the Harness window has no conversation
            // yet, which is the normal state of a fresh environment.
            (None, true) => "未跟随会话（Harness 中还没有会话）".to_owned(),
            (None, false) => "尚未连接".to_owned(),
        }
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
        let state = self.state();
        // Clicks are collected while painting and applied after it, so no session
        // lock is held inside the layout closure.
        let mut action: Option<CardAction> = None;
        let hotkey_status = if self.hotkey.is_active() {
            format!("{} 隐藏", self.hotkey.spec())
        } else {
            format!("热键未注册：{}", self.hotkey.reason().unwrap_or("unknown"))
        };
        let follow_status = state.follow.status();
        let fonts_warning = self.fonts_warning.clone();

        egui::Frame::NONE
            .fill(BACKGROUND)
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                // The harness's own title when it has one: it is the name the user sees
                // in the Harness window, so the panel says the same thing rather than
                // guessing from the first message.
                let heading = state.title.clone().unwrap_or_else(|| "quorfloat".to_owned());
                wrapped(ui, egui::RichText::new(heading).size(15.0).color(TEXT));
                ui.add_space(2.0);
                ui.label(egui::RichText::new(hotkey_status).size(12.0).color(MUTED));
                ui.add_space(2.0);
                ui.label(egui::RichText::new(follow_status).size(12.0).color(MUTED));
                if let Some(warning) = &fonts_warning {
                    ui.add_space(2.0);
                    wrapped(ui, egui::RichText::new(warning).size(12.0).color(WARN));
                }
                ui.add_space(8.0);

                if let Some(handoff) = &state.handoff {
                    handoff_banner(ui, handoff, &mut action);
                    ui.add_space(8.0);
                }

                if state.interactions.is_empty() {
                    ui.label(egui::RichText::new("没有待处理的请求").size(12.0).color(MUTED));
                } else {
                    // `auto_shrink` vertically, and bounded: a request is actionable so it
                    // has to be on screen, but a year-old card must not push the
                    // conversation out of the panel — which is exactly what an
                    // `auto_shrink([false, false])` area does even when it is empty.
                    let cards = (ui.available_height() * 0.6).max(120.0);
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, true])
                        .max_height(cards)
                        .show(ui, |ui| {
                            for card in &state.interactions {
                                match card.kind() {
                                    InteractionKind::Approval => approval_card(ui, card, &mut action),
                                    InteractionKind::Question => question_card(ui, card, &mut action),
                                }
                                ui.add_space(8.0);
                            }
                        });
                }

                ui.add_space(8.0);
                conversation(ui, &state);
            });

        if let Some(action) = action {
            self.apply_card_action(action);
        }
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

/// Draw the conversation.
///
/// Sticks to the bottom, which is the whole of the "follow the answer as it is written"
/// behaviour: egui only keeps a scroll area pinned while it is already at the bottom, so
/// a user who has scrolled up to read something is not yanked back by the next token.
///
/// @param ui - where to draw.
/// @param state - the conversation, plus the message still being generated.
fn conversation(ui: &mut egui::Ui, state: &PanelState) {
    // Whatever the header and any cards did not take. This is the panel's main body, so
    // it is the part that must never be squeezed to nothing: an empty area that expands
    // to fill the window is what left the conversation invisible, not empty.
    let height = ui.available_height().max(CONVERSATION_MIN_HEIGHT);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .max_height(height)
        .show(ui, |ui| {
            if state.entries.is_empty() && state.live.is_none() {
                ui.label(egui::RichText::new("尚无对话内容").size(12.0).color(MUTED));
                return;
            }
            for entry in state.entries.iter() {
                entry_ui(ui, entry);
            }
            if let Some(live) = &state.live {
                entry_ui(ui, live);
            }
        });
}

/// Draw one line of the conversation.
///
/// @param ui - where to draw.
/// @param entry - the line.
fn entry_ui(ui: &mut egui::Ui, entry: &crate::transcript::Entry) {
    use crate::transcript::{Block, Entry};

    ui.add_space(6.0);
    ui.label(egui::RichText::new(speaker(entry)).size(11.0).color(MUTED));
    ui.add_space(2.0);
    match entry {
        Entry::User { text } => {
            wrapped(ui, egui::RichText::new(text).size(13.0).color(TEXT));
        }
        Entry::Assistant { blocks, streaming } => {
            for block in blocks {
                match block {
                    Block::Text(text) => wrapped(ui, egui::RichText::new(text).size(13.0).color(TEXT)),
                    // Reasoning is drawn, not hidden: it is what the model is doing, and
                    // a panel that shows only conclusions makes a slow answer look stuck.
                    Block::Reasoning(text) => {
                        wrapped(ui, egui::RichText::new(text).size(12.0).color(MUTED).italics());
                    }
                    Block::Call { name, arguments } => {
                        wrapped(ui, egui::RichText::new(format!("$ {name} {arguments}")).size(12.0).color(MUTED).monospace());
                    }
                }
                ui.add_space(2.0);
            }
            if *streaming {
                ui.label(egui::RichText::new("生成中…").size(11.0).color(MUTED));
            }
        }
        Entry::Tool { text, is_error, .. } => {
            let colour = if *is_error { BAD } else { MUTED };
            wrapped(ui, egui::RichText::new(text).size(12.0).color(colour).monospace());
        }
        Entry::System { text } => wrapped(ui, egui::RichText::new(text).size(12.0).color(MUTED)),
        Entry::Notice { text, .. } => wrapped(ui, egui::RichText::new(text).size(12.0).color(MUTED)),
    }
}

/// Who a line is from, as the panel labels it.
///
/// @param entry - the line.
/// @returns the label above it.
#[must_use]
fn speaker(entry: &crate::transcript::Entry) -> String {
    use crate::transcript::Entry;
    match entry {
        Entry::User { .. } => "你".to_owned(),
        Entry::Assistant { .. } => "quorfloat".to_owned(),
        Entry::Tool { name, .. } => {
            name.as_ref().map_or_else(|| "工具".to_owned(), |name| format!("工具 · {name}"))
        }
        Entry::System { .. } => "系统".to_owned(),
        // The kind is in the label rather than only in the text: an unrecognised event
        // is exactly the case where the reader needs to know what it was called.
        Entry::Notice { kind, .. } => format!("事件 · {kind}"),
    }
}

/// How tall the conversation is allowed to be when nothing else needs the space.
///
/// A floor, not a preference: the panel's reason to exist is the conversation, so it
/// gets its own room even when the window is small.
const CONVERSATION_MIN_HEIGHT: f32 = 160.0;

/// The locale the panel asks the asker's own text for.
///
/// Every string this file writes is Chinese, so a localized `displayReason` is
/// requested in Chinese too and falls back to `en` (see [`Interaction::detail`]).
const DISPLAY_LOCALE: &str = "zh";

const BACKGROUND: egui::Color32 = egui::Color32::from_rgb(24, 24, 28);
const CARD: egui::Color32 = egui::Color32::from_rgb(34, 34, 41);
const TEXT: egui::Color32 = egui::Color32::from_rgb(226, 226, 234);
const MUTED: egui::Color32 = egui::Color32::from_rgb(150, 150, 165);
const ALLOW: egui::Color32 = egui::Color32::from_rgb(38, 92, 58);
const REJECT: egui::Color32 = egui::Color32::from_rgb(104, 42, 46);
const BANNER: egui::Color32 = egui::Color32::from_rgb(58, 50, 30);
const WARN: egui::Color32 = egui::Color32::from_rgb(228, 196, 122);
const OK: egui::Color32 = egui::Color32::from_rgb(134, 205, 158);
const BAD: egui::Color32 = egui::Color32::from_rgb(226, 140, 140);

/// Draw a label that wraps, so peer-supplied text cannot widen the panel.
///
/// @param ui - where to draw.
/// @param text - the already-styled text.
fn wrapped(ui: &mut egui::Ui, text: egui::RichText) {
    ui.add(egui::Label::new(text).wrap());
}

/// Draw the announcement that a request was routed to another surface.
///
/// The wording has to be true, and "true" depends on what the host reported. A hint
/// now means "the panel did not take this" — the host only announces a hand-off when
/// it defers — so the panel must not imply it is holding the request. And "go to the
/// Harness window" is only advice it can give when a surface is actually live: with
/// none, the honest sentence is that nothing can answer it.
///
/// @param ui - where to draw.
/// @param handoff - what the host announced.
/// @param action - collects the dismissal if the user asks for one.
fn handoff_banner(ui: &mut egui::Ui, handoff: &Handoff, action: &mut Option<CardAction>) {
    let live = !handoff.surfaces().is_empty();
    let (headline, instruction) = match (handoff.kind(), live) {
        (InteractionKind::Question, true) => ("追问待回答", "请切到 Harness 窗口输入"),
        (InteractionKind::Question, false) => ("追问待回答", "当前没有可以回答它的 Harness 窗口"),
        (InteractionKind::Approval, true) => ("审批待确认", "请切到 Harness 窗口确认"),
        (InteractionKind::Approval, false) => ("审批待确认", "当前没有可以确认它的 Harness 窗口"),
    };
    egui::Frame::NONE
        .fill(BANNER)
        .inner_margin(egui::Margin::same(10))
        .corner_radius(6)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(headline).size(13.0).color(WARN).strong());
                ui.label(egui::RichText::new(instruction).size(12.0).color(TEXT));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("知道了").clicked() {
                        *action = Some(CardAction::DismissHandoff);
                    }
                });
            });
        });
}

/// Draw one approval card, with its buttons only while an answer is still possible.
///
/// @param ui - where to draw.
/// @param card - the interaction to render.
/// @param action - collects what the user clicked.
fn approval_card(ui: &mut egui::Ui, card: &Interaction, action: &mut Option<CardAction>) {
    egui::Frame::NONE
        .fill(CARD)
        .inner_margin(egui::Margin::same(12))
        .corner_radius(6)
        .show(ui, |ui| {
            let headline = match card.tool_name() {
                Some(tool) => format!("工具 {tool} 请求提权"),
                None => "有工具请求提权".to_owned(),
            };
            ui.label(egui::RichText::new(headline).size(13.0).color(TEXT).strong());
            ui.add_space(4.0);
            wrapped(
                ui,
                egui::RichText::new(
                    card.detail(DISPLAY_LOCALE).unwrap_or_else(|| "请求方没有说明原因".to_owned()),
                )
                .size(12.0)
                .color(MUTED),
            );
            ui.add_space(8.0);
            match card.state() {
                InteractionState::Pending => {
                    ui.horizontal(|ui| {
                        let allow = egui::Button::new(egui::RichText::new("允许一次").color(TEXT)).fill(ALLOW);
                        if ui.add(allow).clicked() {
                            *action = Some(CardAction::Answer {
                                id: card.id().to_owned(),
                                verdict: ApprovalVerdict::AllowOnce,
                            });
                        }
                        let reject = egui::Button::new(egui::RichText::new("拒绝").color(TEXT)).fill(REJECT);
                        if ui.add(reject).clicked() {
                            *action = Some(CardAction::Answer {
                                id: card.id().to_owned(),
                                verdict: ApprovalVerdict::Reject,
                            });
                        }
                    });
                }
                InteractionState::Submitting { .. } => {
                    // Buttons are gone rather than disabled: a second click would be a
                    // second decision for one question, and the host refuses it anyway.
                    ui.label(egui::RichText::new("已提交，等待 Harness 确认…").size(12.0).color(MUTED));
                }
                InteractionState::Applied { verdict } => {
                    let (text, colour) = match verdict {
                        ApprovalVerdict::AllowOnce => ("已允许一次", OK),
                        ApprovalVerdict::Reject => ("已拒绝", BAD),
                    };
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(text).size(12.0).color(colour));
                        close_button(ui, card, action);
                    });
                }
                InteractionState::Refused { reason } => {
                    wrapped(
                        ui,
                        egui::RichText::new(format!("已失效：{reason}")).size(12.0).color(BAD),
                    );
                    ui.add_space(4.0);
                    close_button(ui, card, action);
                }
            }
        });
}

/// Draw one question card.
///
/// It has no answer widgets, and says so. This build cannot answer a question — the
/// answer shape is a list of selected option ids, and there is no widget for that
/// yet — and a card that offered a disabled button would imply the panel could
/// answer if only the user tried harder. What it *can* do is show the question, which
/// is what lets the user decide whether to switch to the Harness window.
///
/// @param ui - where to draw.
/// @param card - the interaction to render.
/// @param action - collects what the user clicked.
fn question_card(ui: &mut egui::Ui, card: &Interaction, action: &mut Option<CardAction>) {
    let questions = card.questions();
    egui::Frame::NONE
        .fill(CARD)
        .inner_margin(egui::Margin::same(12))
        .corner_radius(6)
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(format!("追问（{} 个问题）", card.question_count()))
                    .size(13.0)
                    .color(TEXT)
                    .strong(),
            );
            ui.add_space(4.0);
            wrapped(
                ui,
                egui::RichText::new("本窗口不能回答追问，请切到 Harness 窗口输入").size(12.0).color(WARN),
            );
            for (header, question) in &questions {
                ui.add_space(4.0);
                let text = match header {
                    Some(header) => format!("• {header} — {question}"),
                    None => format!("• {question}"),
                };
                wrapped(ui, egui::RichText::new(text).size(12.0).color(MUTED));
            }
            if card.question_count() > questions.len() {
                ui.add_space(4.0);
                wrapped(ui, egui::RichText::new("（还有更多，未全部显示）").size(12.0).color(MUTED));
            }
            ui.add_space(8.0);
            if ui.button("知道了").clicked() {
                *action = Some(CardAction::Dismiss { id: card.id().to_owned() });
            }
        });
}

/// Draw the button that stops showing a resolved card.
///
/// @param ui - where to draw.
/// @param card - the card being closed.
/// @param action - collects the dismissal.
fn close_button(ui: &mut egui::Ui, card: &Interaction, action: &mut Option<CardAction>) {
    if ui.small_button("关闭").clicked() {
        *action = Some(CardAction::Dismiss { id: card.id().to_owned() });
    }
}

/// How often the clock thread wakes the render loop.
///
/// One second, and deliberately not the follow layer's own interval: this is only a
/// ceiling on how long clock-driven work can be delayed, while the policy (how often
/// to look for a new conversation) lives in [`crate::follow`]. Waking a hidden panel
/// costs a pass that paints nothing.
pub const CLOCK_INTERVAL_MS: u64 = 1000;

/// Wake the render loop on a timer.
///
/// eframe repaints on demand, so with the panel hidden and no frames arriving `logic`
/// is never called — and hidden is the state this panel spends its life in. Anything
/// that comes due *by clock* rather than by event therefore needs something to wake
/// the loop, and that is all this thread does. It is the same shape as
/// [`crate::window::watch_hotkey`], and for the same reason: a poll inside the render
/// callback cannot run when there is no render callback.
///
/// The interval is a ceiling, not a schedule: what is actually due is decided by
/// [`crate::follow::Follow`], which is where the policy belongs.
///
/// @param interval - how long to sleep between wakes.
/// @param egui - the render context, filled in once the window exists.
/// @param outcome - the shared ending slot; the thread stops once it is filled.
/// @returns an error when the thread could not be started.
pub fn watch_clock(
    interval: std::time::Duration,
    egui: Arc<Mutex<Option<egui::Context>>>,
    outcome: Arc<Mutex<Option<SessionExit>>>,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("quorfloat-clock".to_owned())
        .spawn(move || {
            loop {
                std::thread::sleep(interval);
                {
                    let finished = match outcome.lock() {
                        Ok(slot) => slot.is_some(),
                        Err(poisoned) => poisoned.into_inner().is_some(),
                    };
                    if finished {
                        return;
                    }
                }
                let context = match egui.lock() {
                    Ok(slot) => slot.clone(),
                    Err(poisoned) => poisoned.into_inner().clone(),
                };
                if let Some(ctx) = context {
                    ctx.request_repaint();
                }
            }
        })
        .map(|_| ())
}

/// The capabilities known before any window exists.
///
/// A free function rather than a method because it is the input to construction:
/// the hotkey is registered before the app is built, and its outcome is a fact by
/// then. `window` is deliberately absent — see [`App::note_window_created`].
///
/// @param hotkey_active - whether the global hotkey registered.
/// @returns the capability names established so far.
#[must_use]
fn measured_capabilities(hotkey_active: bool) -> Vec<&'static str> {
    let mut capabilities = vec!["egui"];
    if hotkey_active {
        capabilities.push("hotkey");
    }
    capabilities
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::rpc::Inbound;
    use crate::session::{FrameSink, HotkeyReport, Identity};
    use std::sync::mpsc::channel;

    /// A sink that records instead of writing, reachable from the test afterwards.
    #[derive(Clone, Default)]
    struct Recorded {
        frames: Arc<Mutex<Vec<serde_json::Value>>>,
        logs: Arc<Mutex<Vec<String>>>,
        marks: Arc<Mutex<Vec<String>>>,
    }

    impl Recorded {
        fn frames(&self) -> Vec<serde_json::Value> {
            self.frames.lock().expect("not poisoned").clone()
        }

        fn logs(&self) -> Vec<String> {
            self.logs.lock().expect("not poisoned").clone()
        }

        fn marks(&self) -> Vec<String> {
            self.marks.lock().expect("not poisoned").clone()
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

        fn mark(&mut self, line: &str) {
            self.0.marks.lock().expect("not poisoned").push(line.to_owned());
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
        let (app, recorded, _session, wake) = app_and_session();
        (app, recorded, wake)
    }

    /// A snapshot carrying one user message and one assistant message.
    fn conversation_frame() -> Inbound {
        Inbound::Notification {
            method: "session/snapshot".to_owned(),
            params: Some(serde_json::json!({
                "sessionId": "session-1", "generation": 1, "cursor": 1, "hasMore": false,
                "records": [
                    {"type": "event", "event": {"type": "user/message", "seq": 0, "time": 1,
                     "data": {"role": "user", "content": [{"type": "text", "text": "帮我看看"}]}}},
                    {"type": "event", "event": {"type": "assistant/message", "seq": 1, "time": 2,
                     "data": {"message": {"role": "assistant",
                        "content": [{"type": "reasoning", "text": "想一下"},
                                    {"type": "text", "text": "看到了"}]}}}},
                ],
            })),
        }
    }

    #[test]
    fn the_window_state_carries_the_conversation() {
        // The window reads this every frame; if the plumbing dropped it, the panel would
        // show a conversation that never appears while the protocol looked perfect.
        let (mut app, recorded, session, _wake) = app_and_session();
        deliver(&session, &recorded, conversation_frame());

        let state = app.state();
        assert_eq!(state.entries.len(), 2, "{:?}", state.entries);
        assert_eq!(speaker(&state.entries[0]), "你");
        assert_eq!(speaker(&state.entries[1]), "quorfloat");
        assert!(state.live.is_none());
    }

    #[test]
    fn a_stream_in_flight_is_offered_as_a_live_entry() {
        let (mut app, recorded, session, _wake) = app_and_session();
        deliver(&session, &recorded, conversation_frame());
        deliver(
            &session,
            &recorded,
            Inbound::Notification {
                method: "session/stream".to_owned(),
                params: Some(serde_json::json!({
                    "sessionId": "session-1", "generation": 1,
                    // The live shape, not the replay shape: see `transcript.rs`.
                    "frame": {"type": "chunk", "revision": 3, "index": 1, "time": 2,
                              "chunk": {"type": "text-delta", "index": 0, "text": "正在回答"}},
                })),
            },
        );

        let state = app.state();
        let live = state.live.expect("the stream is offered while it is being written");
        assert_eq!(speaker(&live), "quorfloat");
        assert!(matches!(&live, crate::transcript::Entry::Assistant { streaming: true, .. }));
    }

    #[test]
    fn drawing_a_conversation_does_not_panic() {
        // A panic inside the layout closure takes the panel down with no message the
        // user can act on, and the code that draws blocks, tool output and notices is
        // the code with the most ways to get an index wrong.
        let (mut app, recorded, session, _wake) = app_and_session();
        deliver(&session, &recorded, conversation_frame());
        deliver(
            &session,
            &recorded,
            Inbound::Notification {
                method: "session/stream".to_owned(),
                params: Some(serde_json::json!({
                    "sessionId": "session-1", "generation": 1,
                    // Deliberately a delta kind this build does not know: the drawing
                    // path has to survive an event it cannot interpret, because that is
                    // what a harness upgrade looks like from here.
                    "frame": {"type": "chunk", "revision": 9, "index": 4, "time": 5,
                              "chunk": {"type": "tool-input-delta", "index": 2, "delta": "{\"command\":"}},
                })),
            },
        );

        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.draw(ui));
        output.textures_delta.clear();
    }

    /// Draw the panel at a given size and return every piece of text it produced, with
    /// where on screen it landed.
    fn drawn_text(app: &mut App, size: egui::Vec2) -> Vec<(String, egui::Rect)> {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| app.draw(ui));
        let texts = output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => {
                    Some((text.galley.text().to_owned(), egui::Rect::from_min_size(text.pos, text.galley.size())))
                }
                _ => None,
            })
            .collect();
        output.textures_delta.clear();
        texts
    }

    #[test]
    fn the_conversation_is_drawn_inside_the_panel() {
        // The regression this exists for: an empty `auto_shrink([false, false])` area
        // above the conversation expands to the whole window, so the conversation is laid
        // out below the visible area. The data is perfect, the panel is blank, and the
        // only scrollbar belongs to the empty area — which is exactly what a user
        // reported as "no conversation text, scrollbar does nothing".
        let (mut app, recorded, session, _wake) = app_and_session();
        deliver(&session, &recorded, conversation_frame());

        let size = egui::vec2(420.0, 420.0);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        let texts = drawn_text(&mut app, size);
        let visible = |needle: &str| {
            texts.iter().any(|(text, rect)| {
                text.contains(needle)
                    && rect.height() > 1.0
                    && rect.min.y >= screen.min.y - 1.0
                    && rect.max.y <= screen.max.y + 1.0
                    && rect.min.x >= screen.min.x - 1.0
            })
        };
        assert!(visible("帮我看看"), "the user's message is where it can be seen: {texts:#?}");
        assert!(visible("看到了"), "and so is the answer: {texts:#?}");
    }

    #[test]
    fn the_conversation_keeps_its_room_when_a_card_is_waiting() {
        // A pending request is actionable and must be visible — but not at the cost of
        // the conversation disappearing, which is the same bug wearing a card.
        let (mut app, recorded, session, _wake) = app_and_session();
        deliver(&session, &recorded, conversation_frame());
        deliver(
            &session,
            &recorded,
            Inbound::Notification {
                method: "interaction/open".to_owned(),
                params: Some(serde_json::json!({
                    "interactionId": "a-1", "sessionId": "session-1", "kind": "approval",
                    "payload": {"toolName": "bash", "reason": "test"},
                })),
            },
        );

        let size = egui::vec2(420.0, 460.0);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        let texts = drawn_text(&mut app, size);
        assert!(
            texts.iter().any(|(text, rect)| text.contains("帮我看看") && rect.max.y <= screen.max.y + 1.0),
            "the conversation survives a card: {texts:#?}",
        );
    }

    #[test]
    fn a_notice_names_its_event_kind() {
        // The label is where an unrecognised event announces itself.
        let notice = crate::transcript::Entry::Notice {
            kind: "something/new".to_owned(),
            text: "新的东西".to_owned(),
        };
        assert_eq!(speaker(&notice), "事件 · something/new");
    }

    /// The same app, plus the session, for tests that deliver host frames into it.
    fn app_and_session() -> (App, Recorded, Arc<Mutex<Session>>, Sender<Wake>) {
        let session = Arc::new(Mutex::new(Session::new(identity())));
        let recorded = Recorded::default();
        let sink = Arc::new(SharedSink::new(Box::new(RecordingSink(recorded.clone()))));
        let (tx, rx) = channel();
        let hotkey = Hotkey::Unavailable { reason: "test".to_owned(), spec: "Alt+Space".to_owned() };
        let outcome = Arc::new(Mutex::new(None));
        let app = App::new(
            Arc::clone(&session),
            sink,
            hotkey,
            WindowSettings::default(),
            rx,
            outcome,
            false,
        );
        (app, recorded, session, tx)
    }

    /// Deliver one host frame, on the same path the reader thread uses.
    fn deliver(session: &Arc<Mutex<Session>>, recorded: &Recorded, frame: Inbound) {
        let mut sink = RecordingSink(recorded.clone());
        let mut guard = session.lock().expect("not poisoned");
        guard.on_frame(frame, &mut sink);
    }

    /// The host's `interaction/open` for one approval.
    fn approval_open(id: &str) -> Inbound {
        Inbound::Notification {
            method: "interaction/open".to_owned(),
            params: Some(serde_json::json!({
                "interactionId": id,
                "sessionId": "session-1",
                "kind": "approval",
                "payload": {
                    "toolName": "bash",
                    "reason": "escalate sandbox to danger-full-access: V5-B",
                    "displayReason": {"zh": "允许本次操作使用 danger-full-access 权限：V5-B"},
                },
            })),
        }
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
    fn a_host_command_shows_and_hides_the_panel() {
        // The direction that was missing: without it the panel can only be summoned
        // by the global hotkey, so a host-side action (opening a pending approval)
        // has no way to bring it into view.
        let (mut app, recorded, _tx) = app();
        app.report_visibility();
        {
            let mut session = app.session.lock().expect("not poisoned");
            let mut sink = RecordingSink(Recorded::default());
            session.on_frame(
                Inbound::Notification {
                    method: "window/visibility".to_owned(),
                    params: Some(serde_json::json!({"visible": true})),
                },
                &mut sink,
            );
        }
        app.apply_window_commands();
        assert!(app.is_visible());
        app.report_visibility();
        let frames = recorded.frames();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[1]["params"]["visible"], true);
    }

    #[test]
    fn a_host_command_and_the_hotkey_take_the_same_path() {
        // Both funnel through `set_visible`, so the focus behaviour cannot be
        // present in one path and forgotten in the other.
        let (mut app, _recorded, _tx) = app();
        app.set_visible(true);
        assert!(app.is_visible());
        app.pump();
        assert!(app.is_visible(), "a pump does not undo a host command");
    }

    #[test]
    fn commands_are_applied_in_order() {
        // A show followed by a hide in one batch must end hidden; collapsing the
        // batch to "the last command" would also pass this, but applying in order is
        // what makes a toggle in the middle mean what it says.
        let (mut app, _recorded, _tx) = app();
        {
            let mut session = app.session.lock().expect("not poisoned");
            let mut sink = RecordingSink(Recorded::default());
            for visible in [true, false] {
                session.on_frame(
                    Inbound::Notification {
                        method: "window/visibility".to_owned(),
                        params: Some(serde_json::json!({"visible": visible})),
                    },
                    &mut sink,
                );
            }
        }
        app.apply_window_commands();
        assert!(!app.is_visible(), "the last command wins");
    }

    #[test]
    fn capabilities_are_measured_not_declared() {
        // The failure this prevents: `hello` claims `window` because this build
        // links a GUI toolkit, on a machine where no window can ever appear. The
        // host would then offer the user a panel that never shows up.
        let (mut app, recorded, _tx) = app();
        assert_eq!(app.capabilities(), ["egui"], "nothing is claimed before it is known");
        app.note_window_created();
        assert!(app.capabilities().contains(&"window"));
        app.report_visibility();
        assert_eq!(recorded.frames()[0]["params"]["capabilities"], serde_json::json!(["egui", "window"]));
    }

    #[test]
    fn a_registered_hotkey_is_reported_and_a_failed_one_is_not() {
        // A taken accelerator is a normal outcome; reporting it as working would
        // leave the user pressing a key that does nothing.
        assert_eq!(measured_capabilities(false), ["egui"]);
        assert_eq!(measured_capabilities(true), ["egui", "hotkey"]);
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
        borrowed.mark("a breadcrumb");
        assert_eq!(recorded.frames().len(), 1);
        assert_eq!(recorded.logs(), vec!["hello".to_owned()]);
        // The breadcrumb has to survive the extra hop, or the marker file exists and
        // stays empty — which is worse than not having one, because it looks like
        // nothing happened.
        assert_eq!(recorded.marks(), vec!["a breadcrumb".to_owned()]);
    }

    #[test]
    fn an_approval_arriving_becomes_a_card_the_window_can_draw() {
        let (app, recorded, session, _wake) = app_and_session();
        deliver(&session, &recorded, approval_open("approval-1"));
        let state = app.state();
        assert!(state.handoff.is_none());
        assert_eq!(state.interactions.len(), 1);
        let card = &state.interactions[0];
        assert_eq!(card.id(), "approval-1");
        assert_eq!(card.tool_name(), Some("bash"));
        assert!(card.is_actionable(), "an unanswered approval offers both buttons");
        assert_eq!(
            card.detail(DISPLAY_LOCALE).as_deref(),
            Some("允许本次操作使用 danger-full-access 权限：V5-B"),
        );
    }

    #[test]
    fn allowing_sends_the_answer_through_the_shared_sink() {
        // The wiring this covers: a click has to reach the protocol, and the sink it
        // writes through is shared with the reader thread. That path is the one place
        // the window layer can get the host's answer wrong.
        let (mut app, recorded, session, _wake) = app_and_session();
        deliver(&session, &recorded, approval_open("approval-1"));
        app.apply_card_action(CardAction::Answer {
            id: "approval-1".to_owned(),
            verdict: ApprovalVerdict::AllowOnce,
        });
        let frames = recorded.frames();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0]["method"], "interaction/answer");
        assert_eq!(frames[0]["params"]["answer"]["outcome"], "allowed-once");
        assert_eq!(
            app.state().interactions[0].state(),
            &InteractionState::Submitting { verdict: ApprovalVerdict::AllowOnce }
        );
    }

    #[test]
    fn one_card_cannot_be_answered_twice() {
        let (mut app, recorded, session, _wake) = app_and_session();
        deliver(&session, &recorded, approval_open("approval-1"));
        app.apply_card_action(CardAction::Answer {
            id: "approval-1".to_owned(),
            verdict: ApprovalVerdict::Reject,
        });
        app.apply_card_action(CardAction::Answer {
            id: "approval-1".to_owned(),
            verdict: ApprovalVerdict::AllowOnce,
        });
        let answers: Vec<_> = recorded
            .frames()
            .into_iter()
            .filter(|frame| frame["method"] == "interaction/answer")
            .collect();
        assert_eq!(answers.len(), 1, "the second decision never left the process");
        assert_eq!(answers[0]["params"]["answer"]["outcome"], "rejected");
    }

    #[test]
    fn the_hosts_verdict_reaches_the_card() {
        let (mut app, recorded, session, _wake) = app_and_session();
        deliver(&session, &recorded, approval_open("approval-1"));
        app.apply_card_action(CardAction::Answer {
            id: "approval-1".to_owned(),
            verdict: ApprovalVerdict::AllowOnce,
        });
        let id = recorded.frames()[0]["id"].clone();
        deliver(
            &session,
            &recorded,
            Inbound::Response { id, outcome: Ok(serde_json::json!({"accepted": true})) },
        );
        let cards = app.state().interactions;
        assert_eq!(cards[0].state(), &InteractionState::Applied { verdict: ApprovalVerdict::AllowOnce });
        assert!(!cards[0].is_actionable(), "a resolved card offers no buttons");
        app.apply_card_action(CardAction::Dismiss { id: "approval-1".to_owned() });
        assert!(app.state().interactions.is_empty());
    }

    #[test]
    fn a_hand_off_is_shown_and_can_be_dismissed() {
        let (mut app, recorded, session, _wake) = app_and_session();
        deliver(
            &session,
            &recorded,
            Inbound::Notification {
                method: "interaction/hint".to_owned(),
                params: Some(serde_json::json!({"kind": "question", "reason": "harness-not-visible"})),
            },
        );
        let state = app.state();
        assert!(state.interactions.is_empty(), "a hint is not a card: it has no id and cannot be answered");
        assert_eq!(state.handoff.expect("the banner is shown").kind(), InteractionKind::Question);
        app.apply_card_action(CardAction::DismissHandoff);
        assert!(app.state().handoff.is_none());
    }
}

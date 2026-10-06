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
//! The session state stays in [`crate::app::session::Session`], shared behind a mutex.
//! The reader thread is the only writer; the main thread reads it to pick up the
//! host's configuration.

use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

use eframe::egui;

pub mod pinned;
pub mod session;
pub mod sink;

use crate::ui::fonts::{self, FontStatus};
use crate::app::session::interaction::{Handoff, Interaction};
use crate::app::sink::{BorrowedSink, SharedSink, Wake};
use crate::app::session::{Delivery, Session, SessionExit, WindowCommand};
use crate::runtime::hotkey::Hotkey;
use crate::ui::window::WindowSettings;









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
    /// Whether the window's position has been observed at least once this run.
    window_seen: bool,
    /// Whether the resolved theme has been reported once this run.
    theme_seen: bool,
    /// What the user pinned, and where that is remembered.
    pinned: crate::app::pinned::Pinned,
    /// Where the pin is remembered, or nowhere when there is no home to write to.
    pinned_path: Option<std::path::PathBuf>,
    /// Whether the workspace list has been asked for in this run.
    workspaces_asked: bool,
    /// Whether the panel has held focus at least once since it was shown.
    ///
    /// The rule for hiding on blur is about the user *leaving*: a panel that was never
    /// focused — shown behind another window, or opened by a script — must not vanish the
    /// moment it appears, which is what a plain "not focused" test would do.
    focused_once: bool,
    /// Where to write one picture of the panel, when the environment asks for one.
    ///
    /// A development aid with no part in the running panel: this process draws the panel's
    /// corners and its own shadow, and the only honest way to check either is to look at
    /// them. It needs no screen-recording permission because the pixels come from egui's own
    /// framebuffer rather than from the desktop — which is also its limit: it shows what the
    /// panel draws, not how the platform composites it.
    screenshot: Option<std::path::PathBuf>,
    /// Whether that picture has been asked for yet this run.
    screenshot_asked: bool,
    /// Whether it has been written, so it is written once and not once a frame.
    screenshot_written: bool,
    /// Where the window is, and where that is remembered.
    ///
    /// Owned here rather than by the window layer: the window reports where it is, and the
    /// decision to write a file is application state, not drawing.
    window_state: crate::ui::WindowState,
    /// What the user has typed but not sent.
    ///
    /// Owned here rather than by the widget, because a widget forgets: this has to survive
    /// the frames in which the panel does not paint, and a send the host refused.
    draft: String,
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
        window_state: crate::ui::WindowState,
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
            window_state,
            window_seen: false,
            theme_seen: false,
            focused_once: false,
            workspaces_asked: false,
            pinned: crate::app::pinned::Pinned::load(
                crate::app::pinned::path_from_env().as_deref().unwrap_or(std::path::Path::new("")),
            ),
            pinned_path: crate::app::pinned::path_from_env(),
            screenshot: std::env::var_os("DSH_QUORFLOAT_SCREENSHOT")
                .map(std::path::PathBuf::from)
                .filter(|path| !path.as_os_str().is_empty()),
            screenshot_asked: false,
            screenshot_written: false,
            draft: String::new(),
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
    /// @param status - what [`crate::ui::fonts::install`] found and loaded.
    /// Point the pin file somewhere else, for tests that must not touch the real one.
    ///
    /// @param path - where pins should be written.
    #[cfg(test)]
    pub(crate) fn note_pinned_path(&mut self, path: std::path::PathBuf) {
        self.pinned_path = Some(path);
    }

    /// @param status - what [`crate::ui::fonts::install`] found and loaded.
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

    /// Ask for the workspace list the first time the panel is on screen.
    ///
    /// The picker can ask for it too, but the *bar* needs it before anyone opens a menu: it
    /// names the workspace the panel is in, and "工作区" is not a name. One request per run,
    /// and only once the panel is actually being looked at.
    fn ask_for_workspaces_once(&mut self) {
        if self.workspaces_asked {
            return;
        }
        let Some(ctx) = self.context.clone() else { return };
        if !ctx.input(|input| input.viewport().visible().unwrap_or(false)) {
            return;
        }
        self.workspaces_asked = true;
        let mut session = match self.session.lock() {
            Ok(session) => session,
            Err(poisoned) => poisoned.into_inner(),
        };
        let mut sink = BorrowedSink(&self.sink);
        session.request_workspaces(&mut sink);
    }

    /// Put the panel away when the user has gone back to another window.
    ///
    /// @see should_hide_on_blur for the rule itself: it is a function so that it can be
    ///   tested without a window manager, which is the only part of this a test can drive.
    fn hide_when_the_user_leaves(&mut self) {
        let Some(ctx) = self.context.clone() else { return };
        let (visible, focused) = ctx.input(|input| {
            (
                input.viewport().visible().unwrap_or(true),
                input.viewport().focused.unwrap_or(false),
            )
        });
        if focused {
            self.focused_once = true;
        }
        if should_hide_on_blur(self.settings.hide_on_blur, visible, self.focused_once, focused) {
            self.focused_once = false;
            self.set_visible(false);
        }
        if !visible {
            // Forgotten while hidden, so that the next appearance has to earn focus again
            // rather than hiding itself because the *previous* appearance once had it.
            self.focused_once = false;
        }
    }

    /// Ask for one picture of the panel, and write it when egui hands it over.
    ///
    /// The ask has to come from inside a pass, and the answer arrives as an event on a later
    /// one — hence two pieces of state rather than a straight-line call.
    fn write_screenshot(&mut self) {
        let Some(path) = self.screenshot.clone() else { return };
        let Some(ctx) = &self.context else { return };

        if self.screenshot_written {
            return;
        }
        if let Some(image) = ctx.input(|input| {
            input.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        }) {
            match crate::ui::screenshot::write_ppm(&path, &image) {
                Ok(()) => {
                    self.screenshot_written = true;
                    self.sink.mark(&format!("screenshot written to {}", path.display()));
                }
                Err(error) => {
                    self.screenshot_written = true;
                    self.sink.mark(&format!("screenshot failed: {error}"));
                }
            }
            return;
        }
        // Once the panel is actually on screen: a hidden window has no framebuffer to hand
        // over, and asking then would capture nothing rather than the panel.
        let visible = ctx.input(|input| input.viewport().visible().unwrap_or(true));
        if !self.screenshot_asked && visible {
            self.screenshot_asked = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
    }

    /// Report where the window is, so the next run can open there.
    ///
    /// Called from every pass rather than from the drag handler: the window can also be
    /// moved by the system (a display change, a window manager shortcut), and a panel that
    /// only remembers the moves it made itself is a panel that forgets the others.
    fn remember_window_position(&mut self) {
        let Some(ctx) = &self.context else { return };
        // `outer_rect` is the whole window; the user thinks of the panel's top-left corner,
        // and that is what comes back from the file.
        let Some((x, y)) = ctx.input(|input| input.viewport().outer_rect.map(|rect| (rect.min.x, rect.min.y)))
        else {
            return;
        };
        let now = crate::ipc::rpc::now_millis();
        if !self.window_seen {
            // The first sighting is the interesting one: it says whether the platform
            // honoured the position this process asked for, which is the difference
            // between "the memory works" and "we keep asking and keep being overruled".
            self.window_seen = true;
            let requested = self
                .window_state
                .requested()
                .map_or_else(|| "none".to_owned(), |(x, y)| format!("{x:.0},{y:.0}"));
            // Visibility belongs in this line. "The panel is not on screen" then has an
            // answer in the log rather than a guess about which layer ate it — which is
            // exactly the question that cost a round of screenshots of the wrong window.
            let visible = ctx.input(|input| input.viewport().visible().unwrap_or(true));
            self.sink.mark(&format!(
                "window first seen at {x:.0},{y:.0} (requested {requested}, visible {visible})"
            ));
        }
        if self.window_state.observe((x, y), now) {
            self.sink.mark(&format!("window position remembered {x:.0},{y:.0}"));
        }
    }

    /// Record why the session ended.
    ///
    /// @param exit - the ending to remember.
    fn record(&mut self, exit: SessionExit) {
        self.window_state.flush(crate::ipc::rpc::now_millis());
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
        // Unless the panel was asked to open showing. This command used to run
        // unconditionally, which quietly defeated that request: the window was created
        // visible and hidden again a frame later, so "start visible" looked like a setting
        // that did nothing — and a screenshot of the panel came back as a picture of the
        // window behind it.
        if self.settings.start_visible {
            self.startup_hidden = true;
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
        self.write_screenshot();
        self.hide_when_the_user_leaves();
        self.ask_for_workspaces_once();
        self.remember_window_position();
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
        let conversations = session.conversations().to_vec();
        let workspaces = session.workspaces().to_vec();
        // The conversation actually attached — not `target_conversation`, which is only the
        // *deliberate* target and is `None` while the panel follows the newest one. Taking the
        // target here is what left the top bar showing its fallback text and the current row
        // without a tick, in the first screenshot of the picker.
        let attached = follow.session_id().map(str::to_owned);
        // Which workspace the attached conversation is in: the session list carries no
        // workspace id, so the directory both lists agree on is what matches them.
        let current_workspace = attached
            .as_deref()
            .and_then(|id| conversations.iter().find(|entry| entry.session_id == id))
            .and_then(|entry| entry.cwd.as_deref())
            .and_then(|cwd| {
                workspaces
                    .iter()
                    .find(|workspace| paths_match(&workspace.path, cwd))
                    .map(|workspace| workspace.workspace_id.clone())
            });
        PanelState {
            hotkey: self.hotkey_status(),
            prompt_line: session.prompt_delivery().describe(),
            prompt_sending: session.prompt_delivery().is_sending(),
            turn_active: transcript.is_turn_active(),
            fonts_warning: self.fonts_warning.clone(),
            interactions: session.interactions().to_vec(),
            handoff: session.handoff().cloned(),
            entries: transcript.shared_entries(),
            live: transcript.live_entry(),
            title: transcript.title().map(str::to_owned),
            conversations,
            workspaces,
            attached,
            pinned: session.pinned_conversation().map(str::to_owned),
            pinned_workspace: self.pinned.workspace.clone(),
            current_workspace,
            max_height: self.settings.max_height,
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
    fn apply_card_action(&mut self, action: crate::ui::Action) {
        if action == crate::ui::Action::Hide {
            // Before the session lock, because this needs `self` mutably and the lock holds
            // it. The conversation is not lost: hiding is what the hotkey does, and the
            // panel comes back to the same place it was — which is why the design offers
            // this rather than a close.
            self.set_visible(false);
            return;
        }

        use crate::ui::Action;
        let mut session = match self.session.lock() {
            Ok(session) => session,
            Err(poisoned) => poisoned.into_inner(),
        };
        let mut sink = BorrowedSink(&self.sink);
        match action {
            // Handled before the session lock is taken, below; this arm exists so that the
            // match stays exhaustive without pretending the lock is not held here.
            crate::ui::Action::Hide => {}
            Action::Answer { id, verdict } => {
                session.answer_interaction(&id, verdict, &mut sink);
            }
            Action::Dismiss { id } => {
                session.dismiss_interaction(&id);
            }
            Action::DismissHandoff => session.dismiss_handoff(),
            Action::Send { text } => {
                // The draft is cleared only when the host may actually have it: a refused
                // send leaves the text where the user can fix it, which is the difference
                // between "that failed" and "my message vanished".
                if matches!(session.send_prompt(&text, &mut sink), Delivery::Failed { .. }) {
                    // The session has already logged why. Keeping the text is the point:
                    // the user can fix a refusal, and cannot fix a vanished message.
                } else {
                    self.draft.clear();
                }
            }
            Action::Cancel => {
                session.cancel_turn(&mut sink);
            }
            Action::ChooseConversation { session_id } => {
                session.choose_conversation(&session_id, &mut sink);
            }
            Action::CreateConversation { workspace_id } => {
                session.create_conversation(workspace_id.as_deref(), &mut sink);
            }
            Action::PinConversation { session_id } => {
                // Written before the request goes out: a pin the user set is a fact about
                // the panel, whether or not the host can switch to it this second.
                session.pin_conversation(session_id.clone(), &mut sink);
                self.pinned.session = session_id;
                if let Some(path) = &self.pinned_path {
                    self.pinned.save(path);
                }
            }
            Action::RefreshWorkspaces => {
                session.request_workspaces(&mut sink);
            }
            Action::PinWorkspace { workspace_id } => {
                self.pinned.workspace = workspace_id;
                if let Some(path) = &self.pinned_path {
                    self.pinned.save(path);
                }
            }
        }
    }
}


/// Everything the window renders, read under one short lock.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PanelState {
    /// The hotkey line: which key hides the panel, or why none does.
    pub(crate) hotkey: String,
    /// One line about the last prompt, when there is something to say.
    pub(crate) prompt_line: Option<String>,
    /// Whether a prompt is awaiting the host's verdict.
    pub(crate) prompt_sending: bool,
    /// Whether the host says a turn is in progress, which is when "stop" is offered.
    pub(crate) turn_active: bool,
    /// What to tell the user about the fonts, when there is something to tell.
    pub(crate) fonts_warning: Option<String>,
    /// Requests waiting for the user, oldest first.
    pub(crate) interactions: Vec<Interaction>,
    /// The last hand-off the host announced, if any.
    pub(crate) handoff: Option<Handoff>,
    /// The conversation this panel follows.
    pub(crate) follow: FollowView,
    /// The conversation so far, oldest first.
    ///
    /// Shared, not cloned: this is read on every repaint and may hold megabytes.
    pub(crate) entries: std::sync::Arc<Vec<crate::app::session::transcript::Entry>>,
    /// The assistant message being generated, when one is.
    pub(crate) live: Option<crate::app::session::transcript::Entry>,
    /// The harness's name for this conversation, when it has chosen one.
    pub(crate) title: Option<String>,
    /// The conversations the host last listed, newest first.
    pub(crate) conversations: Vec<crate::app::session::follow::SessionSummary>,
    /// The workspaces the host last listed.
    pub(crate) workspaces: Vec<crate::app::session::follow::Workspace>,
    /// The conversation the panel is attached to, if any.
    pub(crate) attached: Option<String>,
    /// The conversation the user pinned, if any.
    pub(crate) pinned: Option<String>,
    /// The workspace new conversations are created in, if one is pinned.
    pub(crate) pinned_workspace: Option<String>,
    /// The workspace the attached conversation belongs to, matched by directory.
    pub(crate) current_workspace: Option<String>,
    /// The tallest the panel may grow, in logical pixels.
    ///
    /// Handed down rather than read from the settings where they live, because the drawing
    /// layer is what decides how much of the conversation fits — and it must decide it
    /// without reaching for the window.
    pub(crate) max_height: f32,
}

/// What the status line needs to know about the followed conversation.
///
/// Shown because two states look identical from every other angle: "attached to
/// nothing" and "attached and receiving nothing". The first one means no approval can
/// ever reach this panel, so it must not be invisible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FollowView {
    /// The conversation followed, if any.
    pub(crate) session_id: Option<String>,
    /// The workspace name that came with it.
    pub(crate) label: Option<String>,
    /// Whether discovery has started at all.
    pub(crate) started: bool,
    /// Persistent events received for it.
    pub(crate) events: u64,
    /// Times the host reported a gap.
    pub(crate) resyncs: u64,
}

impl FollowView {
    /// The line to show under the header.
    #[must_use]
    pub(crate) fn status(&self) -> String {
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
    /// The hotkey line the panel shows.
    ///
    /// Built here rather than in the renderer because it is a fact about this process,
    /// not about the view: a failed grab has a reason, and the reason is worth a line.
    #[must_use]
    fn hotkey_status(&self) -> String {
        if self.hotkey.is_active() {
            format!("{} 隐藏", self.hotkey.spec())
        } else {
            format!("热键未注册：{}", self.hotkey.reason().unwrap_or("unknown"))
        }
    }

    /// Draw the panel's contents.
    ///
    /// Separate from [`App::logic`] because eframe only calls this when there is
    /// something to paint. Anything the host depends on must not live here.
    ///
    /// A thin delegate on purpose: the drawing lives in [`crate::ui`], and the click it
    /// reports is applied *after* the frame, so no session lock is ever held inside a
    /// layout closure.
    ///
    /// @param ui - the root area, with no margin or background of its own.
    pub fn draw(&mut self, ui: &mut egui::Ui) {
        let layout = self.draw_panel(ui);
        self.follow_content_height(ui.ctx(), layout.desired_height);
    }

    /// Give the window the height the content asked for, one animation step at a time.
    ///
    /// The panel's height *is* its content now: compact while there is nothing to read,
    /// growing as an answer arrives, and capped where a long one starts to scroll. The
    /// animation is not decoration — a window that jumps two hundred pixels when a reply
    /// lands is a window that looks like it crashed and came back.
    ///
    /// @param ctx - the context to send the resize through.
    /// @param desired - the panel height the drawing layer measured.
    fn follow_content_height(&mut self, ctx: &egui::Context, desired: f32) {
        let outer = desired
            + f32::from(crate::ui::theme::SHADOW_ROOM_TOP)
            + f32::from(crate::ui::theme::SHADOW_ROOM_BOTTOM);
        let animated = if self.settings.reduce_motion {
            outer
        } else {
            let id = egui::Id::new("quorfloat-panel-height");
            ctx.animate_value_with_time(id, outer, crate::ui::theme::SPEED_EXPAND)
        };
        // Only when it differs: a resize command every frame is a window manager working
        // every frame, and `inner_rect` already says what the platform settled on.
        let current = ctx.input(|input| input.viewport().inner_rect.map(|rect| rect.height()));
        if current.is_some_and(|current| (current - animated).abs() < 0.5) {
            return;
        }
        let width = self.settings.width + f32::from(crate::ui::theme::SHADOW_ROOM_SIDE) * 2.0;
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(width, animated)));
    }

    /// Draw the panel itself, and report what it measured.
    fn draw_panel(&mut self, ui: &mut egui::Ui) -> crate::ui::PanelLayout {
        // Before anything is drawn: the panel names a family for every icon it draws, and a
        // family bound to no fonts is a panic in epaint rather than a blank space.
        crate::ui::fonts::ensure_icons(ui.ctx());
        // One resolution per frame, before anything is drawn: the palette is process state
        // (see `ui/theme.rs`), and this is its single writer.
        let mode = self.settings.theme.resolve(ui.ctx());
        crate::ui::theme::set_mode(mode);
        if !self.theme_seen {
            // Once, on the record: "why is the panel light" is otherwise a question about a
            // platform setting this process cannot show you.
            self.theme_seen = true;
            let preference = match self.settings.theme {
                crate::ui::theme::Preference::System => "system",
                crate::ui::theme::Preference::Light => "light",
                crate::ui::theme::Preference::Dark => "dark",
            };
            let mode = match mode {
                crate::ui::theme::Mode::Light => "light",
                crate::ui::theme::Mode::Dark => "dark",
            };
            self.sink.mark(&format!("theme {mode} (preference {preference})"));
        }
        let state = self.state();
        let mut action: Option<crate::ui::Action> = None;
        let layout = crate::ui::draw(ui, &state, &mut self.draft, &mut action);
        if let Some(action) = action {
            self.apply_card_action(action);
        }
        layout
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












/// How often the clock thread wakes the render loop.
///
/// One second, and deliberately not the follow layer's own interval: this is only a
/// ceiling on how long clock-driven work can be delayed, while the policy (how often
/// to look for a new conversation) lives in [`crate::app::session::follow`]. Waking a hidden panel
/// costs a pass that paints nothing.
pub const CLOCK_INTERVAL_MS: u64 = 1000;

/// Wake the render loop on a timer.
///
/// eframe repaints on demand, so with the panel hidden and no frames arriving `logic`
/// is never called — and hidden is the state this panel spends its life in. Anything
/// that comes due *by clock* rather than by event therefore needs something to wake
/// the loop, and that is all this thread does. It is the same shape as
/// [`crate::runtime::hotkey::watch_hotkey`], and for the same reason: a poll inside the render
/// callback cannot run when there is no render callback.
///
/// The interval is a ceiling, not a schedule: what is actually due is decided by
/// [`crate::app::session::follow::Follow`], which is where the policy belongs.
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

/// Whether a conversation's directory is the workspace's directory.
///
/// Exact after normalising a trailing separator, and nothing cleverer: a session opened in a
/// *subdirectory* of a workspace is a session the workspace list cannot name, and guessing by
/// prefix would label it with the wrong workspace the moment two workspaces are nested.
///
/// @param workspace_path - the workspace's directory, as the host reports it.
/// @param cwd - the conversation's directory.
/// @returns whether they are the same directory.
#[must_use]
fn paths_match(workspace_path: &str, cwd: &str) -> bool {
    let trim = |path: &str| path.trim_end_matches(['/', '\\']).to_owned();
    !workspace_path.is_empty() && trim(workspace_path) == trim(cwd)
}

/// Whether the panel should put itself away, given who is focused.
///
/// Four conditions, and each is a way this could have been wrong:
///
/// - the setting has to be on at all;
/// - the panel has to be visible, or this is a hide command for something already hidden;
/// - the user has to have been *in* the panel since it appeared, because a panel that opens
///   behind another window is not focused and must not vanish on its first frame;
/// - and focus has to be somewhere else now.
///
/// @param hide_on_blur - the setting.
/// @param visible - whether the panel is on screen.
/// @param focused_once - whether it has held focus since it was shown.
/// @param focused - whether it holds focus now.
/// @returns whether to hide.
#[must_use]
fn should_hide_on_blur(hide_on_blur: bool, visible: bool, focused_once: bool, focused: bool) -> bool {
    hide_on_blur && visible && focused_once && !focused
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::session::interaction::{ApprovalVerdict, InteractionKind, InteractionState};
    use crate::app::sink::BorrowedSink;
    use crate::ui::{Action, DISPLAY_LOCALE, speaker};
    use std::sync::mpsc::Sender;
    use crate::ipc::rpc::Inbound;
    use crate::app::session::{FrameSink, HotkeyReport, Identity};
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
        let (app, recorded, session, _wake) = app_and_session();
        deliver(&session, &recorded, conversation_frame());

        let state = app.state();
        assert_eq!(state.entries.len(), 2, "{:?}", state.entries);
        assert_eq!(speaker(&state.entries[0]), "你");
        assert_eq!(speaker(&state.entries[1]), "quorfloat");
        assert!(state.live.is_none());
    }

    #[test]
    fn a_stream_in_flight_is_offered_as_a_live_entry() {
        let (app, recorded, session, _wake) = app_and_session();
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
        assert!(matches!(&live, crate::app::session::transcript::Entry::Assistant { streaming: true, .. }));
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
        // The way `main` does it: before the first frame, because egui builds its font atlas
        // when a pass starts and a family named in the same frame it was added is not in it.
        crate::ui::fonts::ensure_icons(&ctx);
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
    fn a_send_with_no_conversation_keeps_the_text() {
        // No session is followed in this app, so the send is refused. The draft is the
        // only copy of what the user typed: clearing it would lose the message and tell
        // them nothing.
        let (mut app, _recorded, _session, _wake) = app_and_session();
        app.draft = "还没发出去".to_owned();
        app.apply_card_action(Action::Send { text: app.draft.clone() });
        assert_eq!(app.draft, "还没发出去");
    }

    #[test]
    fn the_composer_stays_visible_when_the_conversation_fills_the_panel() {
        // The reported bug: the conversation was laid out before the composer and took
        // every pixel it was offered, so the input box — the one control that must always
        // be reachable — ended up below the bottom edge of the window.
        //
        // The design fixes it structurally rather than arithmetically: the composer is the
        // *first* thing under the top bar, so it claims its space by being drawn first and
        // the conversation can only have what is left. This test is what keeps that true if
        // someone later moves the composer back under the thread.
        let (mut app, recorded, session, _wake) = app_and_session();
        deliver(&session, &recorded, conversation_frame());

        let size = egui::vec2(420.0, 300.0);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        let texts = drawn_text(&mut app, size);
        let visible = |needle: &str| {
            texts.iter().any(|(text, rect)| {
                text.contains(needle) && rect.height() > 1.0 && rect.max.y <= screen.max.y + 1.0
            })
        };
        assert!(visible("问点什么"), "the input box is inside the window: {texts:#?}");
        assert!(visible("发送"), "and so is the button beside it: {texts:#?}");
        // The conversation is still there — the composer taking its share is not the same
        // as the content above it disappearing.
        assert!(visible("帮我看看"), "the conversation did not vanish: {texts:#?}");
    }

    /// Every way the hide-on-blur rule can be asked, including the two that must not hide.
    #[test]
    fn the_panel_puts_itself_away_only_when_the_user_has_left_it() {
        // Left it: focused before, elsewhere now.
        assert!(should_hide_on_blur(true, true, true, false));
        // Never focused it: shown behind another window or opened by a script, so it stays.
        assert!(!should_hide_on_blur(true, true, false, false));
        // Already hidden: nothing to do.
        assert!(!should_hide_on_blur(true, false, true, false));
        // Still in it.
        assert!(!should_hide_on_blur(true, true, true, true));
        // And the setting turns the whole rule off.
        assert!(!should_hide_on_blur(false, true, true, false));
    }

    /// The window follows the conversation, and the conversation is what decides.
    #[test]
    fn the_window_grows_with_the_conversation_and_is_compact_without_one() {
        let size = egui::vec2(708.0, 620.0);
        let (mut app, recorded, session, _wake) = app_and_session();
        let empty = last_inner_size(&mut app, size);
        assert!(empty > 0.0, "a size is sent even when there is nothing to read: {empty}");

        deliver(&session, &recorded, conversation_frame());
        let full = last_inner_size(&mut app, size);
        assert!(full > empty, "a conversation makes the panel taller: {empty} -> {full}");
        // And the cap holds: the panel never asks for more than its maximum plus the room its
        // shadow needs, which is what keeps a long answer scrolling instead of growing.
        let ceiling = app.settings().max_height
            + f32::from(crate::ui::theme::SHADOW_ROOM_TOP)
            + f32::from(crate::ui::theme::SHADOW_ROOM_BOTTOM);
        assert!(full <= ceiling + 1.0, "{full} is not more than {ceiling}");
    }

    /// Run passes until the height settles, and answer what the panel asked for.
    fn last_inner_size(app: &mut App, size: egui::Vec2) -> f32 {
        let ctx = egui::Context::default();
        crate::ui::fonts::ensure_icons(&ctx);
        let mut height = 0.0;
        // Several passes: the height is animated, so the first pass only starts it.
        for _ in 0..8 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    focused: true,
                    ..Default::default()
                },
                |ui| app.draw(ui),
            );
            for command in output
                .viewport_output
                .values()
                .flat_map(|viewport| &viewport.commands)
            {
                if let egui::ViewportCommand::InnerSize(requested) = command {
                    height = requested.y;
                }
            }
            output.textures_delta.clear();
        }
        height
    }



    /// Pinning and unpinning are the one place the panel writes a preference, and the file is
    /// what makes "always open this one" mean anything across runs.
    #[test]
    fn pinning_a_conversation_writes_it_down_and_unpinning_forgets_it() {
        let (mut app, recorded, session, _wake) = app_and_session();
        let path = std::env::temp_dir().join(format!("quorfloat-pin-test-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        app.note_pinned_path(path.clone());
        deliver(&session, &recorded, sessions_list_frame());

        apply(&mut app, crate::ui::Action::PinConversation { session_id: Some("session-b".to_owned()) });
        assert_eq!(
            crate::app::pinned::Pinned::load(&path).session.as_deref(),
            Some("session-b"),
        );
        assert_eq!(session.lock().expect("session").pinned_conversation(), Some("session-b"));

        // Unpinning removes the file rather than leaving one that says "nothing".
        apply(&mut app, crate::ui::Action::PinConversation { session_id: None });
        assert!(!path.exists(), "unpinning removes the file");
        assert_eq!(session.lock().expect("session").pinned_conversation(), None);
        let _ = std::fs::remove_file(&path);
    }

    /// The workspace a new conversation is created in is pinned the same way.
    #[test]
    fn pinning_a_workspace_writes_it_down_too() {
        let (mut app, _recorded, _session, _wake) = app_and_session();
        let path = std::env::temp_dir().join(format!("quorfloat-pin-ws-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        app.note_pinned_path(path.clone());

        apply(&mut app, crate::ui::Action::PinWorkspace { workspace_id: Some("ws-1".to_owned()) });
        assert_eq!(crate::app::pinned::Pinned::load(&path).workspace.as_deref(), Some("ws-1"));
        apply(&mut app, crate::ui::Action::PinWorkspace { workspace_id: None });
        assert!(!path.exists());
        let _ = std::fs::remove_file(&path);
    }

    /// The bug this test was written for: the bar's drag handle covered the whole bar, so a
    /// press on a picker was swallowed by the handle and the menu never opened. A handle is a
    /// control too, and controls must not sit on top of each other.
    #[test]
    fn pressing_a_picker_opens_it_instead_of_dragging_the_window() {
        let (mut app, _recorded, _session, _wake) = app_and_session();
        let ctx = egui::Context::default();
        crate::ui::fonts::ensure_icons(&ctx);
        let size = egui::vec2(708.0, 620.0);
        // Where the workspace picker is: just right of the brand, inside the top bar.
        let press = egui::pos2(
            f32::from(crate::ui::theme::SHADOW_ROOM_SIDE + crate::ui::theme::PAD_TOP.left) + 110.0,
            f32::from(crate::ui::theme::SHADOW_ROOM_TOP + crate::ui::theme::PAD_TOP.top) + 8.0,
        );
        let popup = egui::Id::new(("quorfloat-picker", 0_u8));

        // The passes matter: egui hit-tests a press against the widget rects registered by the
        // *previous* pass, so a press sent in the first pass reaches nothing at all — which is
        // how the first version of this test managed to fail for the wrong reason.
        let plan: Vec<Vec<egui::Event>> = vec![
            vec![egui::Event::PointerMoved(press)],
            vec![egui::Event::PointerButton {
                pos: press,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
            vec![egui::Event::PointerButton {
                pos: press,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        ];
        let mut opened = false;
        for events in plan {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    focused: true,
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                },
                |ui| app.draw(ui),
            );
            output.textures_delta.clear();
            opened |= egui::Popup::is_id_open(&ctx, popup);
        }
        assert!(opened, "the workspace picker opened on its own press");
    }

    #[test]
    fn dragging_the_header_asks_the_platform_to_move_the_window() {
        // An undecorated window has no title bar, so without this the panel cannot be moved
        // at all — which is exactly what a user reported. The drag is handed to the
        // platform rather than implemented by moving the viewport ourselves: a window moved
        // by its own contents lags the pointer and fights the compositor.
        let (mut app, _recorded, _session, _wake) = app_and_session();
        let ctx = egui::Context::default();
        let size = egui::vec2(420.0, 320.0);

        // Three passes, because egui hit-tests a press against the widget rects registered
        // by the *previous* pass: one pass to register the handle and place the pointer,
        // one to press, one to move. A two-pass version of this test proved nothing — it
        // passed with the drag handle removed.
        // A point inside the top bar, computed from the same tokens the bar is drawn with:
        // the panel is inset by the room its shadow needs, and the bar by its own padding.
        let press = egui::pos2(
            f32::from(crate::ui::theme::SHADOW_ROOM_SIDE + crate::ui::theme::PAD_TOP.left) + 4.0,
            f32::from(crate::ui::theme::SHADOW_ROOM_TOP + crate::ui::theme::PAD_TOP.top) + 4.0,
        );
        let plan: Vec<Vec<egui::Event>> = vec![
            vec![egui::Event::PointerMoved(press)],
            vec![egui::Event::PointerButton {
                pos: press,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
            vec![egui::Event::PointerMoved(egui::pos2(90.0, 40.0))],
        ];

        let mut asked = false;
        for events in plan {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    // Without this the window is not interactive at all: egui ignores the
                    // pointer for an unfocused window, and the drag never reaches a widget.
                    // (The first version of this test omitted it and proved nothing.)
                    focused: true,
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                },
                |ui| app.draw(ui),
            );
            // Viewport commands travel in the per-viewport output, not in the platform
            // output: platform commands are clipboard and URL actions.
            asked |= output
                .viewport_output
                .values()
                .flat_map(|viewport| &viewport.commands)
                .any(|command| matches!(command, egui::ViewportCommand::StartDrag));
            output.textures_delta.clear();
        }
        assert!(asked, "a drag on the header asks the platform to start moving the window");
    }

    #[test]
    fn a_notice_names_its_event_kind() {
        // The label is where an unrecognised event announces itself.
        let notice = crate::app::session::transcript::Entry::Notice {
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
            // No path, so the tests cannot touch the real user's remembered position.
            crate::ui::WindowState::default(),
        );
        (app, recorded, session, tx)
    }

    /// Apply one action, the way the frame after a click does.
    fn apply(app: &mut App, action: crate::ui::Action) {
        app.apply_card_action(action);
    }

    /// A host frame listing two conversations in one workspace, one of them newer.
    fn sessions_list_frame() -> Inbound {
        Inbound::Response {
            id: serde_json::json!(1),
            outcome: Ok(serde_json::json!({"items": [
                {"sessionId": "session-a", "updatedAt": 10, "cwd": "/work/project"},
                {"sessionId": "session-b", "updatedAt": 20, "cwd": "/work/project"},
            ]})),
        }
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
        app.apply_card_action(Action::Answer {
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
        app.apply_card_action(Action::Answer {
            id: "approval-1".to_owned(),
            verdict: ApprovalVerdict::Reject,
        });
        app.apply_card_action(Action::Answer {
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
        app.apply_card_action(Action::Answer {
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
        app.apply_card_action(Action::Dismiss { id: "approval-1".to_owned() });
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
        app.apply_card_action(Action::DismissHandoff);
        assert!(app.state().handoff.is_none());
    }
}

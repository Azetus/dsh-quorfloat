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
pub mod preferences;
pub mod session;
pub mod workspace;
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
    /// The last layout line written, so the record changes only when the numbers do.
    last_layout: Option<String>,
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
    /// When the last picture was successfully written.
    ///
    /// A timestamp rather than a flag, because one picture is not enough: the window's height is a
    /// *result* of drawing, so the first frames show a panel that is still growing and anything below
    /// the fold is missing from them. See [`crate::ui::screenshot`]. The timestamp is what rate-limits
    /// the repeat so this stays a development aid rather than a per-frame cost.
    screenshot_taken_at: Option<std::time::Instant>,
    /// When the last write was recorded in the marker.
    ///
    /// Its own timestamp so the record is periodic rather than one-shot: "the dump has stopped updating"
    /// and "the dump is working" have to be distinguishable after the fact, and the first version's
    /// single line made them look the same.
    screenshot_marked_at: Option<std::time::Instant>,
    /// Where the window is, and where that is remembered.
    ///
    /// Owned here rather than by the window layer: the window reports where it is, and the
    /// decision to write a file is application state, not drawing.
    window_state: crate::ui::WindowState,
    /// The Markdown viewer's parse cache, kept across frames.
    ///
    /// It cannot live in `PanelState`: that is rebuilt from the session on every frame, so the
    /// cache would be thrown away exactly as often as it is used.
    markdown: egui_commonmark::CommonMarkCache,
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
    /// What the user chose in the settings view, and where that is remembered.
    ///
    /// Layered on top of the host's configuration rather than replacing it, so the host stays in
    /// charge of everything the user has not decided here (see [`WindowSettings::with_preferences`]).
    preferences: crate::app::preferences::Preferences,
    /// Where the preferences are remembered, or nowhere when there is no home to write to.
    preferences_path: Option<std::path::PathBuf>,
    /// Whether the settings view is showing in place of the conversation.
    ///
    /// State rather than drawing, because it outlives a frame — and because the panel starts on the
    /// settings page when the environment asks it to, which is a decision made before any drawing.
    settings_open: bool,
    /// Why the chord just pressed was refused, while the row is listening.
    ///
    /// The second half of "the recorder is listening": a key press that produces nothing has to produce
    /// *something*, or the only honest reading is that the recorder is broken.
    hotkey_hint: Option<String>,
    /// Whether the settings row is listening for a chord.
    ///
    /// State here rather than only in the drawing layer, because **the panel's own key handlers have
    /// to stand down while it is true**: `Esc` closes the panel, `Enter` sends, and both are keys a
    /// user may be trying to record. A recorder that swallowed chords but let the panel act on them
    /// would close itself mid-recording.
    hotkey_recording: bool,
    /// Once, so the binding is on the record exactly when it changes.
    hotkey_seen: Option<String>,
}

impl App {
    /// Assemble the application.
    ///
    /// @param session - session state shared with the reader thread.
    /// @param sink - the shared frame sink.
    /// @param hotkey - the registration outcome, success or failure.
    /// @param settings - window geometry and appearance.
    /// @param wake - receives work from the reader thread.
    /// @param preferences_path - where the user's own settings are remembered, or `None` for nowhere.
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
        preferences_path: Option<std::path::PathBuf>,
    ) -> Self {
        let preferences = crate::app::preferences::Preferences::load(
            preferences_path.as_deref().unwrap_or(std::path::Path::new("")),
        );
        Self {
            session,
            sink,
            hotkey,
            // The user's own choices sit on top of what the host sent, here as well as on every
            // later config update: the host starts the process with the configured theme, and a
            // preference the user set in the panel has to win from the very first frame — otherwise
            // the panel would come up in the wrong palette and correct itself once.
            settings: settings.with_preferences(&preferences),
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
            last_layout: None,
            pinned: crate::app::pinned::Pinned::load(
                crate::app::pinned::path_from_env().as_deref().unwrap_or(std::path::Path::new("")),
            ),
            pinned_path: crate::app::pinned::path_from_env(),
            screenshot: std::env::var_os("DSH_QUORFLOAT_SCREENSHOT")
                .map(std::path::PathBuf::from)
                .filter(|path| !path.as_os_str().is_empty()),
            screenshot_asked: false,
            screenshot_taken_at: None,
            screenshot_marked_at: None,
            markdown: egui_commonmark::CommonMarkCache::default(),
            draft: String::new(),
            fonts_warning: None,
            fonts_checked: false,
            preferences,
            preferences_path,
            settings_open: crate::ui::settings_requested(),
            hotkey_recording: crate::ui::hotkey_recording_requested(),
            hotkey_hint: None,
            hotkey_seen: None,
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

    /// Open the workspace picker, for the one rung of the ladder only a person can choose.
    fn open_workspace_picker(&self) {
        let Some(ctx) = &self.context else { return };
        egui::Popup::open_id(ctx, crate::ui::picker_popup_id(crate::ui::PickerKind::Workspace));
    }

    /// Drop a remembered pin that the host no longer honours.
    ///
    /// The follow layer clears its own copy when a pinned conversation disappears from the
    /// list or is refused; this is what makes the file agree, so the next run does not try the
    /// same dead conversation again.
    fn forget_a_pin_the_host_refused(&mut self) {
        if self.pinned.session.is_none() {
            return;
        }
        let still_pinned = match self.session.lock() {
            Ok(session) => session.pinned_conversation().is_some(),
            Err(poisoned) => poisoned.into_inner().pinned_conversation().is_some(),
        };
        if !still_pinned {
            self.pinned.session = None;
            if let Some(path) = &self.pinned_path {
                self.pinned.save(path);
            }
        }
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

    /// Keep one picture of the panel on disk, refreshed while the panel is on screen.
    ///
    /// The ask has to come from inside a pass and the answer arrives as an event on a later one — hence
    /// state rather than a straight-line call.
    ///
    /// **Repeatedly, not once.** This used to write the first frame and stop, which made it lie: the
    /// window's height follows its content, so the early frames show a panel still growing, and whatever
    /// sits below that fold is simply absent from the picture. A settings row that had not been laid out
    /// yet looked like a change that had not taken effect, and the honest reading of that is "the tool is
    /// broken", not "the style did not apply". Now the file is overwritten every second for as long as
    /// the panel is visible, so the picture on disk is always the current one.
    fn write_screenshot(&mut self) {
        let Some(path) = self.screenshot.clone() else { return };
        let Some(ctx) = &self.context else { return };

        if let Some(image) = ctx.input(|input| {
            input.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        }) {
            let now = std::time::Instant::now();
            match crate::ui::screenshot::write_ppm(&path, &image) {
                Ok(()) => {
                    // On the record every so often, not every second: the marker is how "is the dump
                    // still updating" is answered afterwards, and one line at startup cannot answer it.
                    if crate::ui::screenshot::worth_recording(self.screenshot_marked_at, now) {
                        self.screenshot_marked_at = Some(now);
                        self.sink.mark(&format!("screenshot refreshed at {}", path.display()));
                    }
                }
                Err(error) => {
                    self.sink.mark(&format!("screenshot failed: {error}"));
                }
            }
            self.screenshot_taken_at = Some(now);
            // A request is no longer outstanding, whatever the outcome.
            self.screenshot_asked = false;
            return;
        }
        // Once the panel is actually on screen: a hidden window has no framebuffer to hand over, and
        // asking then would capture nothing rather than the panel.
        let visible = ctx.input(|input| input.viewport().visible().unwrap_or(true));
        if crate::ui::screenshot::due(visible, self.screenshot_asked, self.screenshot_taken_at) {
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
        self.forget_a_pin_the_host_refused();
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
        // And then the user's own choices again, because `apply_host` has just overwritten the
        // fields it knows about: a config that arrives after a preference was set would otherwise
        // quietly undo it, which is how a setting appears to forget itself.
        self.settings = self.settings.with_preferences(&self.preferences);
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
        let current_workspace = current_workspace(&session);
        PanelState {
            hotkey: self.hotkey_spec(),
            hotkey_reason: self.hotkey_reason(),
            keep_open: !self.settings.hide_on_blur,
            settings_open: self.settings_open,
            theme: self.settings.theme,
            recording: self.hotkey_recording,
            recording_hint: self.hotkey_hint.clone(),
            stats: session.stats(),
            options: session.options().cloned(),
            setting_failure: session.setting_failure().map(str::to_owned),
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
            workspaces_asked: session.workspaces_asked(),
            creating_on_submit: session.target_conversation().is_none(),
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
        // The settings page is about the panel, not about the conversation, so it is decided
        // before the lock is taken for the same reason hiding is: it needs `self` mutably, and
        // nothing in it may wait on the session.
        match action {
            crate::ui::Action::OpenSettings => {
                self.set_settings_open(true);
                return;
            }
            crate::ui::Action::CloseSettings => {
                self.set_settings_open(false);
                return;
            }
            _ => {}
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
                // Where a *new* conversation goes is the ladder's business (see
                // `app/workspace.rs`); when one is already attached this is not used.
                let workspace = crate::app::workspace::to_create_in(
                    self.pinned.workspace.as_deref(),
                    current_workspace(&session).as_deref(),
                    session.transcript().is_turn_active(),
                    session.workspaces(),
                    session.conversations(),
                );
                // The draft is cleared only when the host may actually have it: a refused
                // send leaves the text where the user can fix it, which is the difference
                // between "that failed" and "my message vanished". A send that is waiting for
                // a conversation to be created has the text in hand, so it counts as taken.
                // Nowhere to create a conversation: rather than let the send fail with a
                // message about a workspace the user has never seen, the panel opens the
                // picker that chooses one. That is P4 of the ladder, and it is the only rung
                // a machine cannot climb.
                if session.follow().session_id().is_none() && workspace.is_none() {
                    self.open_workspace_picker();
                }
                if matches!(
                    session.send_prompt(&text, workspace.as_deref(), &mut sink),
                    Delivery::Failed { .. },
                ) {
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
            Action::NewConversation => {
                session.start_new_conversation(&mut sink);
                // The pin goes with it, and so does the file the next run reads.
                self.pinned.session = None;
                if let Some(path) = &self.pinned_path {
                    self.pinned.save(path);
                }
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
            Action::OpenSettings | Action::CloseSettings => {
                // Handled above, before the lock; this arm exists so that the match stays
                // exhaustive without pretending the lock is not held here.
            }
            Action::SetTheme(preference) => {
                self.preferences.theme = Some(preference);
                self.settings.theme = preference;
                // Written before the next frame draws, so what the user sees and what the next run
                // reads are the same thing. The theme itself needs no more than the assignment
                // above: `draw_panel` resolves the palette from the settings once per frame, so the
                // change lands on the very next one.
                self.save_preferences();
            }
            Action::StartHotkeyRecording => {
                self.hotkey_recording = true;
                self.hotkey_hint = None;
            }
            Action::StopHotkeyRecording => {
                self.hotkey_recording = false;
                self.hotkey_hint = None;
            }
            Action::RejectHotkey { hint } => {
                // The row keeps listening: a chord that cannot be used is a mistake to correct, not a
                // reason to end the recording and make the user click again.
                self.hotkey_hint = Some(hint);
            }
            Action::SelectModel { provider, model, effort } => {
                session.select_model(&provider, &model, effort.as_deref(), &mut sink);
            }
            Action::SelectEffort { effort } => {
                // The model is unchanged, so the current one is taken from what the host last
                // reported rather than from the click: the click named an effort, not a model.
                let current = session
                    .options()
                    .and_then(|options| options.current_model.clone());
                match current {
                    Some((provider, model)) => {
                        session.select_model(&provider, &model, Some(&effort), &mut sink);
                    }
                    None => {
                        // Nothing to pair the effort with, which means the panel never learned
                        // which model is in use. Saying so is better than sending a selection with
                        // an invented provider.
                        self.sink.log("an effort was chosen before the model was known");
                    }
                }
            }
            Action::SetPermission { value } => session.set_permission(&value, &mut sink),
            Action::RefreshOptions => session.request_options(&mut sink),
            Action::SetHotkey(spec) => {
                // Only a change is worth acting on: re-registering the same accelerator would drop
                // and retake a grab the user already has, and a desktop where it was taken by
                // somebody else in the meantime would turn "no change" into "lost my shortcut".
                if self.hotkey.spec() == Some(spec.as_str()) && self.hotkey.is_active() {
                    return;
                }
                // `rebind` consumes the registration and gives one back, which is what makes "the old
                // grab is released before the new one is asked for" a property of the type rather
                // than a convention: there is no moment here when two of them are alive.
                self.hotkey = std::mem::replace(&mut self.hotkey, Hotkey::register("")).rebind(&spec);
                // Remembered either way. A shortcut the user chose is their preference even when the
                // desktop would not give it up: silently reverting to the host's key would mean the
                // choice appears to work and then undoes itself on the next start.
                self.preferences.hotkey = Some(spec.clone());
                self.save_preferences();
                // The row stops asking once it has an answer. Nothing else is needed: the chip reports
                // the accelerator that was asked for, and `hotkey_reason` carries why it is not held —
                // so a refused change is visible without a field to keep open.
                self.hotkey_recording = false;
                self.hotkey_hint = None;
                // On the record, because this is the one setting whose effect is invisible: a
                // shortcut either opens the panel or does nothing, and "nothing" has no other trace.
                let outcome = match self.hotkey.is_active() {
                    true => format!("hotkey {spec} registered"),
                    false => format!(
                        "hotkey {spec} refused: {}",
                        self.hotkey.reason().unwrap_or("unknown"),
                    ),
                };
                self.sink.mark(&outcome);
            }
            Action::KeepOpenOnBlur(keep_open) => {
                self.preferences.keep_open = Some(keep_open);
                self.settings.hide_on_blur = !keep_open;
                self.save_preferences();
            }
        }
    }

    /// Show the settings page, or go back to the conversation.
    ///
    /// Separate from the action match because it needs no session: the page replaces the
    /// conversation on screen and touches nothing about it, which is why leaving it can put the user
    /// back exactly where they were. It is also the whole of what the gear does.
    ///
    /// @param open - whether the page should be showing.
    fn set_settings_open(&mut self, open: bool) {
        if self.settings_open == open {
            return;
        }
        self.settings_open = open;
        // On the record, and as a mark rather than a log line: the host swallows this process's
        // stderr, so the marker is the only place a later question about "was the settings page ever
        // open" can be answered.
        self.sink.mark(&format!("settings {}", if open { "open" } else { "closed" }));
    }

    /// Remember what the user chose in the settings view.
    ///
    /// Failure is ignored, like every other preference file this process writes: not being able to
    /// remember a choice is a smaller problem than refusing to run because of it.
    fn save_preferences(&self) {
        if let Some(path) = &self.preferences_path {
            self.preferences.save(path);
        }
    }
}


/// Everything the window renders, read under one short lock.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PanelState {
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
    /// Whether the workspace list has ever been asked for.
    ///
    /// The picker asks once and then stops: an empty list is an answer, not a reason to ask
    /// again every frame.
    pub(crate) workspaces_asked: bool,
    /// The conversation the panel is attached to, if any.
    pub(crate) attached: Option<String>,
    /// Whether sending would start a new conversation.
    ///
    /// True when nothing is pinned and nothing was chosen: the panel is in "new conversation"
    /// mode, and the conversation appears when the user submits. The composer says so.
    pub(crate) creating_on_submit: bool,
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
    /// The accelerator the panel registered, if it registered one.
    ///
    /// The bare spec rather than a sentence: the settings view puts it in a chip beside the words
    /// "呼出快捷键", and the sentence that used to live here read "Alt+Space 隐藏" — which is a status
    /// line, not a value. The sentence form was only ever shown in the top bar's tooltip.
    pub(crate) hotkey: Option<String>,
    /// Why there is no hotkey, when there is none. A sentence, shown on hover.
    pub(crate) hotkey_reason: Option<String>,
    /// Whether the panel stays open when the user moves to another window.
    pub(crate) keep_open: bool,
    /// Whether the settings view is showing in place of the conversation.
    pub(crate) settings_open: bool,
    /// The theme preference in effect, which the settings view shows as the chosen one.
    ///
    /// The *preference* rather than the resolved palette: a user who has chosen "follow the system"
    /// needs to see that choice reported as theirs, even on a machine that is currently dark.
    pub(crate) theme: crate::ui::theme::Preference,
    /// Whether the hotkey row is listening for a chord.
    ///
    /// The composer and the top bar read this and do nothing while it is true: a recorder that let
    /// `Enter` send a message, or `Esc` put the panel away, would be unusable for the keys most people
    /// want to bind.
    pub(crate) recording: bool,
    /// Why the last chord was refused, while the row is listening.
    pub(crate) recording_hint: Option<String>,
    /// The followed conversation's statistics, once the host has reported any.
    pub(crate) stats: Option<crate::app::session::Stats>,
    /// What may be chosen for it — models, reasoning efforts, permission presets.
    pub(crate) options: Option<crate::app::session::SessionOptions>,
    /// Why the last model or permission change was refused, if it was.
    ///
    /// A refused change is invisible otherwise: the panel would go on showing the old value as if
    /// the click had never happened.
    pub(crate) setting_failure: Option<String>,
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
}

impl App {
    /// The accelerator the panel registered, if it registered one.
    ///
    /// The bare spec, because that is what the settings view shows in its chip: the host writes
    /// accelerators the way a user reads them, and that is the value. A failed grab is not a value,
    /// so it comes back as `None` and the reason is reported separately — one field cannot be both
    /// a shortcut and a sentence about why there is no shortcut.
    #[must_use]
    fn hotkey_spec(&self) -> Option<String> {
        // Straight from the registration, which already knows the difference: `spec` is what was
        // asked for, `is_active` is whether it was had, and the chip shows the first while the
        // tooltip carries the second's reason. Reporting `None` here for a *failed* registration
        // would leave the settings page unable to say which shortcut is the one that did not work.
        self.hotkey.spec().map(str::to_owned)
    }

    /// Why there is no hotkey, when there is none.
    ///
    /// Built here rather than in the renderer because it is a fact about this process: a failed grab
    /// has a reason, and the reason is worth a line.
    #[must_use]
    fn hotkey_reason(&self) -> Option<String> {
        if self.hotkey.is_active() {
            return None;
        }
        Some(self.hotkey.reason().unwrap_or("unknown").to_owned())
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
        self.follow_content_height(ui.ctx(), layout);
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
    fn follow_content_height(&mut self, ctx: &egui::Context, layout: crate::ui::PanelLayout) {
        self.record_layout(ctx, layout);
        let desired = layout.desired_height;
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

    /// Put the panel's own arithmetic on the record, when it changes.
    ///
    /// "The panel is cut off at the bottom" is a question about four numbers, and a screenshot
    /// can only show the answer: this says which of them is wrong.
    ///
    /// @param ctx - the context, for the window's actual size.
    /// @param layout - what the drawing layer measured.
    fn record_layout(&mut self, ctx: &egui::Context, layout: crate::ui::PanelLayout) {
        let size = ctx.input(|input| input.viewport().inner_rect.map(|rect| rect.size()));
        let rounded = |value: f32| (value * 10.0).round() / 10.0;
        let line = format!(
            "layout chrome={} thread_pad={} content={} footer={} panel={} window={}",
            rounded(layout.chrome_above),
            rounded(layout.thread_padding),
            rounded(layout.thread_content),
            rounded(layout.footer),
            rounded(layout.desired_height),
            size.map_or_else(|| "?".to_owned(), |size| format!("{}x{}", rounded(size.x), rounded(size.y))),
        );
        if self.last_layout.as_deref() != Some(line.as_str()) {
            self.last_layout = Some(line.clone());
            self.sink.mark(&line);
        }
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
        // Which accelerator this run actually holds, once — and again if it changes, because the
        // settings page can change it. A global hotkey is the one thing about this panel with no
        // visible trace when it fails: the key either opens the panel or does nothing at all.
        {
            let current = self.hotkey.spec().unwrap_or("(none)").to_owned();
            if self.hotkey_seen.as_deref() != Some(current.as_str()) {
                self.hotkey_seen = Some(current.clone());
                match self.hotkey.reason() {
                    Some(reason) => self.sink.mark(&format!("hotkey {current} unavailable: {reason}")),
                    None => self.sink.mark(&format!("hotkey {current} held")),
                }
            }
        }
        let state = self.state();
        let mut action: Option<crate::ui::Action> = None;
        let layout = crate::ui::draw(ui, &state, &mut self.draft, &mut action, &mut self.markdown);
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

/// The workspace the conversation on screen belongs to, if the lists can say.
///
/// The session list carries no workspace id, so the directory both lists agree on is what
/// matches them: see [`crate::app::workspace::same_directory`] for why that is exact rather
/// than a prefix match.
///
/// @param session - the session being drawn.
/// @returns the workspace id, when one matches.
#[must_use]
fn current_workspace(session: &Session) -> Option<String> {
    let attached = session.follow().session_id()?;
    let cwd = session
        .conversations()
        .iter()
        .find(|conversation| conversation.session_id == attached)?
        .cwd
        .as_deref()?;
    session
        .workspaces()
        .iter()
        .find(|workspace| crate::app::workspace::same_directory(&workspace.path, cwd))
        .map(|workspace| workspace.workspace_id.clone())
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
                                    {"type": "text",
                                     "text": "看到了，**重点**是这一行：\n\n```rust\nlet name = String::from(\"Quorvox\");\n```\n"}]}}}},
                ],
            })),
        }
    }

    /// Point this process's preference file at a scratch path, once.
    ///
    /// `App::new` reads the path from the environment, which is process-global state, so this is done
    /// once and the tests that change a setting share the file rather than racing for the variable.
    /// Without it they would write `~/.dsh-quorfloat/preferences.json` — the real user's own choice
    /// of theme — which is the same reason `app_and_session` passes a default window state.
    /// This test's own preferences file.
    ///
    /// **Its own, not the process's.** The path used to be read from `DSH_QUORFLOAT_PREFERENCES` inside
    /// `App::new`, which made it process-global: every test that changed a setting wrote the same file,
    /// so they overwrote and deleted each other's state and failed in ways that looked like bugs in the
    /// code under test. The constructor takes the path now, and a test names its own — unique per name,
    /// so two tests cannot collide however they are scheduled.
    ///
    /// @param name - what this test is about, for the file name.
    /// @returns a path that does not exist yet.
    fn preferences_for_test(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "quorfloat-app-preferences-{}-{name}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        path
    }

    /// The user's own settings, end to end: set one, and it is still set after a restart.
    ///
    /// One test rather than several, deliberately. `App::new` reads the path from a process-global
    /// environment variable, so tests that each want their own file would race for it; these steps
    /// share one file and run in the order written. What is being checked is the whole chain — the
    /// action, the live state, the file, and the next launch — because a preference that takes
    /// effect but is never written, or is written but never read back, is the same bug to a user.
    #[test]
    fn a_setting_takes_effect_now_and_is_still_set_after_a_restart() {
        let path = preferences_for_test("restart");
        let (mut app, recorded, _session, _wake) = app_and_session_with(&path);
        assert_eq!(app.state().theme, crate::ui::theme::Preference::System, "the host decides first");
        assert!(!path.exists(), "and nothing has been written yet");

        app.apply_card_action(crate::ui::Action::SetTheme(crate::ui::theme::Preference::Dark));
        assert_eq!(
            app.state().theme,
            crate::ui::theme::Preference::Dark,
            "the choice is in effect on the next frame's state",
        );
        let written = std::fs::read_to_string(&path).expect("the choice reaches the disk");
        assert!(written.contains("dark"), "and it is the value that was chosen: {written}");

        // The host publishes its own configuration afterwards, which is the moment a local choice is
        // most likely to be lost: `apply_host` writes over every field it knows about.
        {
            let mut session = app.session.lock().expect("session");
            session.on_frame(
                crate::ipc::rpc::Inbound::Notification {
                    method: "ready".to_owned(),
                    params: Some(serde_json::json!({
                        "config": {"window": {"theme": "light", "hideOnBlur": true}},
                    })),
                },
                &mut RecordingSink(recorded.clone()),
            );
        }
        app.adopt_host_config();
        assert_eq!(
            app.state().theme,
            crate::ui::theme::Preference::Dark,
            "a configuration arriving later does not undo what the user chose",
        );

        // The switch, which is the design's `keepOpen` and the inverse of the config's `hideOnBlur`.
        assert!(!app.state().keep_open, "the host's `hideOnBlur: true` means: do not keep open");
        app.apply_card_action(crate::ui::Action::KeepOpenOnBlur(true));
        assert!(app.state().keep_open, "and turning the switch on keeps the panel open");
        assert!(!app.settings().hide_on_blur, "which is the same fact, spelled the config's way");

        // Now the restart: a fresh app over the same file, as the next launch would build it.
        let (restarted, _recorded, _session, _wake) = app_and_session_with(&path);
        assert_eq!(
            restarted.state().theme,
            crate::ui::theme::Preference::Dark,
            "the theme is remembered across a restart",
        );
        assert!(restarted.state().keep_open, "and so is the switch");

        // Clearing one writes it back to absent rather than to `false`, so that "the user has not
        // decided" stays distinguishable from "the user decided no".
        app.apply_card_action(crate::ui::Action::SetTheme(crate::ui::theme::Preference::System));
        let cleared = std::fs::read_to_string(&path).expect("still a file, for the switch");
        assert!(cleared.contains("system") || cleared.contains("null"), "{cleared}");
    }

    #[test]
    fn pressing_the_gear_opens_the_settings_page() {
        // The command path the tests below skip: they call the action directly, so none of them
        // would notice a gear that is drawn but never wired, or one drawn where the design's padding
        // says but at a rectangle the pointer cannot reach. The gear's position is the design's own
        // arithmetic — the panel's top-right corner, inset by the top bar's padding, less two button
        // widths and the gap between them — and `corner_icon_button` lays it out the same way, so
        // this test and that function agree about where the button is by construction.
        let (mut app, _recorded, _session, _wake) = app_and_session();
        let ctx = egui::Context::default();
        crate::ui::fonts::ensure_icons(&ctx);
        let size = egui::vec2(708.0, 620.0);
        let button = f32::from(crate::ui::theme::ICON_BUTTON);
        let gear = egui::pos2(
            size.x - f32::from(crate::ui::theme::SHADOW_ROOM_SIDE)
                - f32::from(crate::ui::theme::PAD_TOP.right)
                - button * 1.5,
            f32::from(crate::ui::theme::SHADOW_ROOM_TOP + crate::ui::theme::PAD_TOP.top) + button / 2.0,
        );
        // Three passes, for the reason the picker's test spells out: egui hit-tests a press against
        // the rectangles the *previous* pass registered, so a press in the first pass reaches nothing
        // at all — which is how the first version of that test failed for the wrong reason.
        let plan: Vec<Vec<egui::Event>> = vec![
            vec![egui::Event::PointerMoved(gear)],
            vec![egui::Event::PointerButton {
                pos: gear,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
            vec![egui::Event::PointerButton {
                pos: gear,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        ];
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
        }
        assert!(app.state().settings_open, "the gear opened the page");
    }

    #[test]
    fn the_settings_page_replaces_the_conversation_rather_than_covering_it() {
        // The design's `setSettings` hides `q-main` and shows `q-settings`. That is the whole
        // structural claim of this view, and it is a claim about two things at once: the page's own
        // text is on screen, and the conversation's is gone. A dialog would satisfy the first and
        // fail the second, which is why both halves are asserted.
        let (mut app, recorded, session, _wake) = app_and_session();
        deliver(&session, &recorded, conversation_frame());
        let size = egui::vec2(708.0, 620.0);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        let opening = drawn_text(&mut app, size);
        assert!(
            visible(&opening, "帮我看看", screen),
            "the conversation is on screen to begin with",
        );

        let action = crate::ui::Action::OpenSettings;
        app.apply_card_action(action);
        let page = drawn_text(&mut app, size);
        assert!(visible(&page, "悬浮窗设置", screen), "the page says what it is");
        assert!(visible(&page, "返回对话", screen), "and offers the way back");
        assert!(visible(&page, "呼出快捷键", screen), "the hotkey is reported here");
        assert!(visible(&page, "外观", screen), "and the theme is chosen here");
        // Clipped to the panel, not merely absent from the shape tree. The first version of this
        // test asked whether the text was drawn at all, and passed even with both views drawn —
        // because the conversation was being painted *below the bottom edge*, where the settings
        // page had taken all the room. `drawn_text` reads shapes, and a shape outside the screen is
        // still a shape; what the user can read is the question, and clipping is what decides it.
        assert!(
            !visible(&page, "帮我看看", screen),
            "the conversation is not behind it: {:?}",
            on_screen(&page, screen),
        );
        assert!(
            recorded.marks().iter().any(|mark| mark == "settings open"),
            "the marker records it: {:?}",
            recorded.marks(),
        );
    }

    #[test]
    fn recording_starts_on_a_click_and_a_chord_becomes_the_accelerator() {
        // The whole flow the design's chip drives, from a real click to a real chord: the box *is* the
        // control, so clicking it must start listening and a key press while it listens must become the
        // accelerator. The key events go through egui's own input path rather than through a call to the
        // action, because the interesting part is that `Event::Key` reaches this row at all.
        let _serial = crate::runtime::hotkey::test_lock();
        let path = preferences_for_test("recorded");
        let (mut app, recorded, _session, _wake) = app_and_session_with(&path);
        app.hotkey = Hotkey::active_for_test("Alt+Space");
        app.apply_card_action(crate::ui::Action::OpenSettings);
        let ctx = egui::Context::default();
        crate::ui::fonts::ensure_icons(&ctx);
        let size = egui::vec2(708.0, 620.0);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        let run = |app: &mut App, events: Vec<egui::Event>| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    focused: true,
                    screen_rect: Some(screen),
                    ..Default::default()
                },
                |ui| app.draw(ui),
            );
            output.textures_delta.clear();
        };
        let press = |key: egui::Key, modifiers: egui::Modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };

        // Click the box, aimed at the accelerator it is drawing rather than at coordinates worked out
        // from the layout: the box's own text says where the box is, so the test cannot pass by landing
        // somewhere else that happens to be clickable.
        let before = drawn_text(&mut app, size);
        let chip = before
            .iter()
            .find(|(text, rect)| text.contains("Alt+Space") && screen.contains_rect(*rect))
            .map(|(_, rect)| rect.center())
            .unwrap_or_else(|| {
                panic!("the accelerator is on the page: {:?}", on_screen(&before, screen))
            });
        let click = |pressed: bool| egui::Event::PointerButton {
            pos: chip,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        run(&mut app, vec![egui::Event::PointerMoved(chip)]);
        run(&mut app, vec![egui::Event::PointerMoved(chip), click(true)]);
        run(&mut app, vec![click(false)]);
        assert!(app.state().recording, "the click started a recording");
        let listening = drawn_text(&mut app, size);
        assert!(
            visible(&listening, "按下组合键…", screen),
            "and the box says what it wants: {:?}",
            on_screen(&listening, screen),
        );

        // **A bare letter is refused, and the row says so.** This is the case that shipped wrong: the
        // first recorder accepted `M`, and a global hotkey is grabbed from the whole desktop, so the
        // letter M would have stopped reaching every other application. It was found by looking at the
        // panel — the chip showed "M" — which is why this test exists.
        run(&mut app, vec![press(egui::Key::M, egui::Modifiers::NONE)]);
        assert!(app.state().recording, "the row keeps listening after a refusal");
        assert!(
            app.state().recording_hint.is_some(),
            "and explains itself rather than swallowing the key press",
        );
        assert_eq!(app.hotkey.spec(), Some("Alt+Space"), "nothing was re-registered");
        let refused = drawn_text(&mut app, size);
        assert!(
            visible(&refused, "请按住修饰键", screen),
            "the reason is in the row: {:?}",
            on_screen(&refused, screen),
        );

        // A bare function key is allowed: nothing types `F13` by accident.
        run(&mut app, vec![press(egui::Key::F13, egui::Modifiers::NONE)]);
        assert_eq!(app.hotkey.spec(), Some("F13"), "a function key on its own is a shortcut");
        assert!(!app.state().recording, "and the row stopped listening");

        // Now a chord. `Cmd` (or `Ctrl`) + `Shift` + `K`.
        app.apply_card_action(crate::ui::Action::SetHotkey("Alt+Space".to_owned()));
        app.apply_card_action(crate::ui::Action::StartHotkeyRecording);
        let modifiers = egui::Modifiers {
            ctrl: !cfg!(target_os = "macos"),
            command: cfg!(target_os = "macos"),
            shift: true,
            ..egui::Modifiers::NONE
        };
        run(&mut app, vec![press(egui::Key::K, modifiers)]);
        let expected = if cfg!(target_os = "macos") { "Cmd+Shift+K" } else { "Ctrl+Shift+K" };
        assert_eq!(
            app.hotkey.spec(),
            Some(expected),
            "the chord became the accelerator, and nothing was typed",
        );
        assert!(!app.state().recording, "and the row stopped listening");
        assert!(
            recorded.marks().iter().any(|mark| mark.contains(expected)),
            "the attempt is on the record: {:?}",
            recorded.marks(),
        );
        // Remembered, like every other setting: a shortcut the user pressed is a preference.
        let written = std::fs::read_to_string(&path).expect("the choice reaches the disk");
        assert!(written.contains(expected), "{written}");
    }

    #[test]
    fn the_box_says_which_accelerator_did_not_work() {
        // The report this row owes the user, and the reason its rule is what it is: the box is the only
        // place the row can speak, so it shows **the accelerator that was asked for**, in the warning
        // colour when it is not held. Replacing the name with "not registered" would throw away the one
        // fact the user needs — *which* shortcut failed — which is exactly what happened when the text
        // field was removed and nothing else carried the value.
        let (mut app, _recorded, _session, _wake) = app_and_session();
        let size = egui::vec2(708.0, 620.0);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        // **Nothing was ever attempted**, which is the host's documented "no hotkey" escape hatch: no
        // spec at all, rather than a spec that failed. The helper starts the app with a *failed*
        // registration, so this has to be set up.
        app.hotkey = Hotkey::register("");
        app.apply_card_action(crate::ui::Action::OpenSettings);

        let none = drawn_text(&mut app, size);
        assert!(
            visible(&none, "未注册", screen),
            "no accelerator at all is reported as such: {:?}",
            on_screen(&none, screen),
        );

        // An accelerator was attempted and refused: it is named, in the warning colour.
        app.hotkey = Hotkey::unavailable_for_test("Alt+Space", "taken by another application");
        let refused = drawn_text_coloured(&mut app, size);
        assert!(
            refused
                .iter()
                .any(|(text, rect, _)| text.contains("Alt+Space") && screen.contains_rect(*rect)),
            "the refused accelerator is named: {:?}",
            on_screen(&drawn_text(&mut app, size), screen),
        );
        let named = refused
            .iter()
            .find(|(text, _, _)| text.contains("Alt+Space"))
            .expect("it is on the page");
        assert_eq!(
            named.2,
            crate::ui::theme::Palette::DARK.warn_text,
            "and drawn as a warning, not as a value that works",
        );

        // And a registered one is the plain value.
        app.hotkey = Hotkey::active_for_test("Alt+Space");
        let held = drawn_text_coloured(&mut app, size);
        let value = held
            .iter()
            .find(|(text, rect, _)| text.contains("Alt+Space") && screen.contains_rect(*rect))
            .expect("it is on the page");
        assert_eq!(
            value.2,
            crate::ui::theme::Palette::DARK.text,
            "a working accelerator is the panel's text colour",
        );
    }

    #[test]
    fn the_hotkey_row_has_two_faces_and_no_text_field() {
        // The design's shape: one box that reports the accelerator, and the same box asking for a new
        // one while it listens. Asserting that the box is the *only* control is the point — the two
        // buttons that used to sit beside it (录制 / 手动输入) are gone, so a row that grew them back
        // would be a row that stopped following the design.
        let (mut app, _recorded, _session, _wake) = app_and_session();
        app.hotkey = Hotkey::active_for_test("Alt+Space");
        app.apply_card_action(crate::ui::Action::OpenSettings);
        let size = egui::vec2(708.0, 620.0);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);

        let closed = drawn_text(&mut app, size);
        assert!(visible(&closed, "呼出快捷键", screen), "the row is there");
        assert!(visible(&closed, "Alt+Space", screen), "showing what is held");
        for gone in ["录制", "手动输入", "更改"] {
            assert!(
                !visible(&closed, gone, screen),
                "there is no {gone} button: the box is the control ({:?})",
                on_screen(&closed, screen),
            );
        }

        // Listening, the same box changes what it says.
        app.apply_card_action(crate::ui::Action::StartHotkeyRecording);
        let recording = drawn_text(&mut app, size);
        assert!(visible(&recording, "呼出快捷键", screen), "the row stays");
        assert!(
            visible(&recording, "按下组合键…", screen),
            "and the box asks for the chord: {:?}",
            on_screen(&recording, screen),
        );
        assert!(!visible(&recording, "Alt+Space", screen), "the old value gives way to the request");

        // Esc gives up, and the value comes back untouched.
        app.apply_card_action(crate::ui::Action::StopHotkeyRecording);
        let after = drawn_text(&mut app, size);
        assert!(visible(&after, "Alt+Space", screen), "the accelerator is shown again");
        assert_eq!(
            app.hotkey.spec(),
            Some("Alt+Space"),
            "and giving up costs nothing",
        );
    }

    #[test]
    fn the_hotkey_box_draws_its_text_in_the_panels_colour_not_a_dim_one() {
        // The user's report: in dark mode the accelerator in the box was hard to read, because the
        // text colour came from egui's theme rather than from the palette. Asserting on the *colour*
        // is the point: the string was always drawn, so a test that only asked "is it there" passed
        // both before and after the fix.
        let (mut app, _recorded, _session, _wake) = app_and_session();
        app.hotkey = Hotkey::active_for_test("Alt+Space");
        app.settings.theme = crate::ui::theme::Preference::Dark;
        app.apply_card_action(crate::ui::Action::OpenSettings);
        let size = egui::vec2(708.0, 620.0);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);

        let drawn = drawn_text_coloured(&mut app, size);
        let palette = crate::ui::theme::Palette::DARK;
        let accelerator = drawn
            .iter()
            .find(|(text, rect, _)| text.contains("Alt+Space") && screen.contains_rect(*rect))
            .unwrap_or_else(|| panic!("the accelerator is on the page: {}", drawn.len()));
        assert_eq!(
            accelerator.2, palette.text,
            "the box's accelerator is the panel's text colour, not a dim inactive grey",
        );
    }

    /// Put the host's answer into the session, the way a response frame would.
    ///
    /// @param session - the session to fill.
    /// @param recorded - for the frame the request writes.
    /// @returns nothing; the answer is applied in place.
    fn give_options(
        session: &Arc<Mutex<Session>>,
        recorded: &Recorded,
        result: serde_json::Value,
    ) {
        let mut sink = RecordingSink(recorded.clone());
        let mut guard = session.lock().expect("session");
        guard.request_options(&mut sink);
        let id = recorded
            .frames
            .lock()
            .expect("frames")
            .last()
            .expect("an options request")["id"]
            .clone();
        guard.on_frame(Inbound::Response { id, outcome: Ok(result) }, &mut sink);
    }

    #[test]
    fn the_footer_shows_the_statistics_the_host_reported() {
        // The figures only, and nothing when there are none: an empty strip is honest, and "0 tok/s"
        // reads as a stalled model rather than as "not measured yet".
        let (mut app, recorded, session, _wake) = app_and_session();
        attach_one(&mut app, &recorded, &session);
        let size = egui::vec2(708.0, 620.0);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);

        // Before the host says anything, the strip says nothing.
        let empty = drawn_text(&mut app, size);
        assert!(!visible(&empty, "轮", screen), "no counts before they are reported: {:?}", on_screen(&empty, screen));

        {
            let mut sink = RecordingSink(recorded.clone());
            session.lock().expect("session").on_frame(
                Inbound::Notification {
                    method: "session/stats".to_owned(),
                    params: Some(serde_json::json!({
                        "sessionId": "session-1",
                        "stats": {
                            "turns": 3, "steps": 7, "tokensPerSecond": 200.0,
                            "cacheHitPercent": 75, "contextTokens": 4000, "contextLimit": 128000,
                        },
                    })),
                },
                &mut sink,
            );
        }
        let drawn = drawn_text(&mut app, size);
        assert!(visible(&drawn, "3 轮 7 步", screen), "the counts: {:?}", on_screen(&drawn, screen));
        assert!(visible(&drawn, "200 tok/s", screen), "the output speed");
        assert!(visible(&drawn, "缓存命中 75%", screen), "the cache share");
        // 4000 of 128000 is 3%, rounded down: a share is what a person can act on, and the raw pair
        // is not.
        assert!(visible(&drawn, "上下文 3%", screen), "the context share");
    }

    #[test]
    fn the_reported_statistics_are_absent_rather_than_zero_when_unmeasurable() {
        let (mut app, recorded, session, _wake) = app_and_session();
        attach_one(&mut app, &recorded, &session);
        {
            let mut sink = RecordingSink(recorded.clone());
            session.lock().expect("session").on_frame(
                Inbound::Notification {
                    method: "session/stats".to_owned(),
                    params: Some(serde_json::json!({"sessionId": "session-1", "stats": {"turns": 0, "steps": 0}})),
                },
                &mut sink,
            );
        }
        let size = egui::vec2(708.0, 620.0);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        let drawn = drawn_text(&mut app, size);
        assert!(visible(&drawn, "0 轮 0 步", screen), "the counts are real figures");
        assert!(!visible(&drawn, "tok/s", screen), "but no speed was measured: {:?}", on_screen(&drawn, screen));
        assert!(!visible(&drawn, "缓存命中", screen));
    }

    #[test]
    fn the_footer_offers_the_models_and_permissions_the_host_listed() {
        let (mut app, recorded, session, _wake) = app_and_session();
        attach_one(&mut app, &recorded, &session);
        give_options(
            &session,
            &recorded,
            serde_json::json!({
                "groups": [{"id": "deepseek", "name": "DeepSeek", "models": [
                    {"id": "v41-flash", "name": "V4.1 Flash", "efforts": [
                        {"id": "low", "name": "低"}, {"id": "high", "name": "高"},
                    ], "defaultEffort": "low"},
                ]}],
                "current": {"provider": "deepseek", "model": "v41-flash", "reasoningEffort": "high"},
                "permissions": [
                    {"value": "read-only", "name": "只读"},
                    {"value": "workspace-write", "name": "工作区修改"},
                ],
                "permission": "read-only",
            }),
        );

        let size = egui::vec2(708.0, 620.0);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        let drawn = drawn_text(&mut app, size);
        // Each control shows the value in effect, which is what makes the strip a report rather than
        // a row of buttons.
        assert!(visible(&drawn, "V4.1 Flash", screen), "the model in use: {:?}", on_screen(&drawn, screen));
        assert!(visible(&drawn, "高", screen), "the effort in use");
        assert!(visible(&drawn, "只读", screen), "the permission in use");
    }

    #[test]
    fn the_footer_offers_nothing_it_was_not_given() {
        // The rule that keeps the strip honest: no options means no controls. A picker with an empty
        // menu reads as a failure, and a made-up model name reads as a fact.
        let (mut app, recorded, session, _wake) = app_and_session();
        attach_one(&mut app, &recorded, &session);
        let size = egui::vec2(708.0, 620.0);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        let drawn = drawn_text(&mut app, size);
        // The panel draws the words only when it has values; the fallback labels would be the
        // giveaway that a control was drawn without anything behind it.
        for absent in ["模型", "权限"] {
            assert!(
                !visible(&drawn, absent, screen),
                "no {absent} control without options: {:?}",
                on_screen(&drawn, screen),
            );
        }
    }

    /// Match the host projection already exercised by the footer tests, then render with the
    /// bundled CJK font: fallback Latin glyph widths cannot prove that Chinese labels fit.
    fn footer_state_for_layout() -> PanelState {
        let (mut app, recorded, session, _wake) = app_and_session();
        attach_one(&mut app, &recorded, &session);
        give_options(&session, &recorded, serde_json::json!({
            "groups": [{"provider": "deepseek-official", "name": "DeepSeek", "models": [
                {"id": "v41-flash", "name": "DeepSeek-V41-Flash", "efforts": [
                    {"id": "low", "name": "低"}, {"id": "high", "name": "高"}
                ]},
                {"id": "second", "name": "Second", "efforts": []}
            ]}],
            "current": {"provider": "deepseek-official", "model": "v41-flash", "reasoningEffort": "high"},
            "permissions": [{"value": "read-only", "name": "只读"}, {"value": "workspace-write", "name": "工作区修改"}],
            "permission": "workspace-write"
        }));
        let mut state = app.state();
        state.stats = Some(crate::app::session::Stats {
            turns: 3, steps: 9, tokens_per_second: Some(229.0), cache_hit_percent: Some(90),
            context_tokens: Some(10870), context_limit: None,
        });
        state
    }

    #[test]
    fn footer_rectangles_contain_real_font_text_even_with_long_values() {
        let ctx = egui::Context::default();
        crate::ui::fonts::ensure_icons(&ctx);
        let font = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts/NotoSansSC-VF.otf");
        assert!(matches!(crate::ui::fonts::install_from(&ctx, &font), crate::ui::fonts::FontStatus::Loaded { .. }));
        let mut state = footer_state_for_layout();
        for long in [false, true] {
            if long {
                let options = state.options.as_mut().unwrap();
                options.models[0].name = "长模型名称-".repeat(30);
                options.models[0].efforts[1].name = "很长的推理档位".repeat(20);
                options.permissions[1].name = "很长的权限预设名称".repeat(20);
                let stats = state.stats.as_mut().unwrap();
                stats.turns = u64::MAX;
                stats.steps = u64::MAX;
                stats.context_tokens = Some(u64::MAX);
            }
            for height in [318.0, 620.0] {
                let mut texts = Vec::new();
                let mut desired = 0.0;
                for _ in 0..3 {
                    let mut output = ctx.run_ui(egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(708.0, height))),
                        ..Default::default()
                    }, |ui| {
                        desired = crate::ui::draw(ui, &state, &mut String::new(), &mut None,
                            &mut egui_commonmark::CommonMarkCache::default()).desired_height;
                    });
                    texts.clear();
                    for shape in &output.shapes { collect_text(&shape.shape, &mut texts); }
                    output.textures_delta.clear();
                }
                let permission = texts.iter().find(|(text, _)| text.starts_with(if long { "很长的权限" } else { "工作区修改" })).expect("permission").1;
                let model = texts.iter().find(|(text, _)| text.starts_with(if long { "长模型名称" } else { "DeepSeek-V41-Flash" })).expect("model").1;
                let panel = egui::Rect::from_min_max(
                    egui::pos2(f32::from(crate::ui::theme::SHADOW_ROOM_SIDE), f32::from(crate::ui::theme::SHADOW_ROOM_TOP)),
                    egui::pos2(708.0 - f32::from(crate::ui::theme::SHADOW_ROOM_SIDE),
                        (height - f32::from(crate::ui::theme::SHADOW_ROOM_BOTTOM)).min(f32::from(crate::ui::theme::SHADOW_ROOM_TOP) + desired)),
                );
                let footer: Vec<_> = texts.iter().filter(|(_, rect)| rect.top() >= permission.top() - 3.0).collect();
                assert!(footer.len() >= 12, "check text and icons, not an empty set: {footer:?}");
                for (index, (text, rect)) in footer.iter().enumerate() {
                    assert!(panel.contains_rect(*rect), "{text:?} at {rect:?} outside {panel:?}");
                    for (other, other_rect) in &footer[index + 1..] {
                        assert!(!rect.intersects(*other_rect), "{text:?} overlaps {other:?}: {rect:?} / {other_rect:?}");
                    }
                }
                let counts = texts.iter().find(|(text, _)| text.contains("轮")).expect("counts").1;
                let close = texts.iter().find(|(text, _)| text == "关闭").expect("hints").1;
                assert!(permission.right() < model.left());
                assert!((permission.center().y - model.center().y).abs() < 2.0);
                assert!(model.bottom() < counts.top());
                assert!(close.right() < counts.left());
            }
        }
    }

    #[test]
    fn combined_footer_menu_still_selects_model_effort_and_permission() {
        fn frame(ctx: &egui::Context, state: &PanelState, events: Vec<egui::Event>) -> (Vec<(String, egui::Rect)>, Option<crate::ui::Action>) {
            let mut action = None;
            let mut output = ctx.run_ui(egui::RawInput {
                events, focused: true,
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(708.0, 620.0))),
                ..Default::default()
            }, |ui| { crate::ui::draw(ui, state, &mut String::new(), &mut action, &mut egui_commonmark::CommonMarkCache::default()); });
            let mut texts = Vec::new();
            for shape in &output.shapes { collect_text(&shape.shape, &mut texts); }
            output.textures_delta.clear();
            (texts, action)
        }
        fn click(ctx: &egui::Context, state: &PanelState, label: &str) -> Option<crate::ui::Action> {
            let _ = frame(ctx, state, vec![]);
            let (texts, _) = frame(ctx, state, vec![]);
            let pos = texts.iter().find(|(text, _)| text == label).unwrap_or_else(|| panic!("missing {label}: {texts:?}")).1.center();
            let _ = frame(ctx, state, vec![egui::Event::PointerMoved(pos)]);
            let mut chosen = None;
            for pressed in [true, false] {
                let (_, action) = frame(ctx, state, vec![egui::Event::PointerButton {
                    pos, pressed, button: egui::PointerButton::Primary, modifiers: egui::Modifiers::NONE,
                }]);
                if action.is_some() { chosen = action; }
            }
            chosen
        }
        let ctx = egui::Context::default();
        crate::ui::fonts::ensure_icons(&ctx);
        let state = footer_state_for_layout();
        click(&ctx, &state, "DeepSeek-V41-Flash");
        click(&ctx, &state, "推理等级");
        assert!(matches!(click(&ctx, &state, "低"), Some(crate::ui::Action::SelectEffort { effort }) if effort == "low"));
        click(&ctx, &state, "DeepSeek-V41-Flash");
        click(&ctx, &state, "模型");
        assert!(matches!(click(&ctx, &state, "Second"), Some(crate::ui::Action::SelectModel { provider, model, effort: None })
            if provider == "deepseek-official" && model == "second"));
        click(&ctx, &state, "工作区修改");
        assert!(matches!(click(&ctx, &state, "只读"), Some(crate::ui::Action::SetPermission { value }) if value == "read-only"));
    }

    #[test]
    fn the_footer_groups_do_not_print_over_each_other() {
        // The bug this exists for: the key hints and the statistics were drawn into the same
        // rectangle and printed on top of one another ("esc 关闭" through "缓存命中 90%"). It was
        // caught by looking at a screenshot after four attempts at the layout, so it is caught here
        // instead — from the rectangles, which is what "on top of each other" means.
        let (mut app, recorded, session, _wake) = app_and_session();
        attach_one(&mut app, &recorded, &session);
        give_options(
            &session,
            &recorded,
            serde_json::json!({
                "groups": [{"provider": "deepseek-official", "name": "DeepSeek",
                            "models": [{"id": "v41-flash", "name": "V4.1 Flash", "efforts": []}]}],
                "current": {"provider": "deepseek-official", "model": "v41-flash"},
                "permissions": [{"value": "read-only", "name": "只读"}],
                "permission": "read-only",
            }),
        );
        {
            let mut sink = RecordingSink(recorded.clone());
            session.lock().expect("session").on_frame(
                Inbound::Notification {
                    method: "session/stats".to_owned(),
                    params: Some(serde_json::json!({
                        "sessionId": "session-1",
                        "stats": {"turns": 3, "steps": 9, "tokensPerSecond": 229.0,
                                  "cacheHitPercent": 90, "contextTokens": 4000, "contextLimit": 128000},
                    })),
                },
                &mut sink,
            );
        }

        let size = egui::vec2(708.0, 620.0);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        let drawn = drawn_text(&mut app, size);
        // The strip is the bottom of the panel: everything below the last thread content. The footer's
        // own rows are the lowest ones drawn.
        let lowest = drawn
            .iter()
            .filter(|(_, rect)| screen.contains_rect(*rect))
            .map(|(_, rect)| rect.bottom())
            .fold(f32::MIN, f32::max);
        let strip = drawn.iter().filter(|(_, rect)| rect.bottom() > lowest - 60.0).collect::<Vec<_>>();
        let panel = egui::Rect::from_min_max(
            egui::pos2(f32::from(crate::ui::theme::SHADOW_ROOM_SIDE), 0.0),
            egui::pos2(size.x - f32::from(crate::ui::theme::SHADOW_ROOM_SIDE), size.y),
        );
        for (text, rect) in &strip {
            assert!(panel.contains_rect(*rect), "footer text {text:?} escaped {panel:?}: {rect:?}");
        }
        let permission = drawn.iter().find(|(text, _)| text == "只读").expect("permission").1;
        let model = drawn.iter().find(|(text, _)| text == "V4.1 Flash").expect("model").1;
        let stats = drawn.iter().find(|(text, _)| text.contains("3 轮 9 步")).expect("stats").1;
        assert!(permission.right() < model.left(), "permission left, model right");
        assert!((permission.center().y - model.center().y).abs() < 2.0, "settings share the upper row");
        assert!(model.bottom() < stats.top(), "statistics sit below the settings");

        // Two groups, and their text must not overlap horizontally: the hints' right edge is left of
        // the statistics' left edge.
        let hint = strip
            .iter()
            .find(|(text, _)| text.contains("发送"))
            .unwrap_or_else(|| panic!("the key hints are in the strip: {:?}", on_screen(&drawn, screen)));
        let counts = strip
            .iter()
            .find(|(text, _)| text.contains("轮"))
            .unwrap_or_else(|| panic!("the statistics are in the strip: {:?}", on_screen(&drawn, screen)));
        assert!(
            hint.1.right() <= counts.1.left(),
            "the hints end at {} and the statistics begin at {}: they are printed over each other",
            hint.1.right(),
            counts.1.left(),
        );
    }

    #[test]
    fn going_back_returns_to_the_conversation_it_left() {
        // Back must be a return and not a reset: the page never touched the conversation, and the
        // test that says so is that the same text is still there afterwards.
        let (mut app, recorded, session, _wake) = app_and_session();
        deliver(&session, &recorded, conversation_frame());
        let size = egui::vec2(708.0, 620.0);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        app.apply_card_action(crate::ui::Action::OpenSettings);
        let page = drawn_text(&mut app, size);
        assert!(visible(&page, "悬浮窗设置", screen));

        app.apply_card_action(crate::ui::Action::CloseSettings);
        let back = drawn_text(&mut app, size);
        assert!(
            visible(&back, "帮我看看", screen),
            "the conversation is back: {:?}",
            on_screen(&back, screen),
        );
        assert!(!visible(&back, "返回对话", screen), "and the page is not");
        assert!(
            recorded.marks().iter().any(|mark| mark == "settings closed"),
            "both transitions are on the record: {:?}",
            recorded.marks(),
        );
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

    /// Drawing a conversation with a frame this build cannot read must not panic.
    ///
    /// A panic inside the layout closure takes the panel down with no message the user can act
    /// on, and the code that draws blocks, tool output and notices is the code with the most ways
    /// Draw the panel at a given size and return every piece of text it produced, with where on
    /// screen it landed.
    ///
    /// Read from the painted shapes rather than from the widget tree: what matters in these tests
    /// is what a user could see, and a widget that exists but paints nothing is not that.
    fn drawn_text(app: &mut App, size: egui::Vec2) -> Vec<(String, egui::Rect)> {
        let ctx = egui::Context::default();
        // The way `main` does it: before the first frame, because egui builds its font atlas when a
        // pass starts, and a family named in the same frame it was added is not in it.
        crate::ui::fonts::ensure_icons(&ctx);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| app.draw(ui));
        // epaint refuses to drop a texture delta nobody applied, and a test has no renderer to
        // apply it to: saying so here is the difference between a helper and a panic.
        output.textures_delta.clear();
        let mut texts = Vec::new();
        for shape in output.shapes {
            collect_text(&shape.shape, &mut texts);
        }
        texts
    }

    /// Whether the user could read this text: drawn, and inside the panel.
    ///
    /// Containment rather than equality, because the panel draws a line at a time and a sentence may
    /// be one galley or several. And **clipped text does not count**: `drawn_text` reads shapes, and
    /// a shape below the bottom edge is still a shape. That distinction is not academic — it is the
    /// one that let a test for "the settings page replaces the conversation" pass while both were
    /// being painted, one of them off the end of the window.
    ///
    /// @param drawn - what [`drawn_text`] collected.
    /// @param needle - the text to look for.
    /// @param screen - the panel's own rectangle.
    /// @returns whether it is on screen where a user could see it.
    fn visible(drawn: &[(String, egui::Rect)], needle: &str, screen: egui::Rect) -> bool {
        drawn.iter().any(|(text, rect)| text.contains(needle) && screen.contains_rect(*rect))
    }

    /// What the user could actually read, for a failure message that says what was there instead.
    ///
    /// @param drawn - what [`drawn_text`] collected.
    /// @param screen - the panel's own rectangle.
    /// @returns the visible strings, in drawing order.
    fn on_screen(drawn: &[(String, egui::Rect)], screen: egui::Rect) -> Vec<&str> {
        drawn
            .iter()
            .filter(|(_, rect)| screen.contains_rect(*rect))
            .map(|(text, _)| text.as_str())
            .collect()
    }

    /// The same drawing, with the colour each run of text was painted in.
    ///
    /// A separate collector rather than a fourth tuple field everywhere, because only the tests about
    /// *how* something is drawn need it — and a report of the form "this text is the wrong colour in
    /// dark mode" cannot be checked against what was drawn, only against how. A green light on strings
    /// would pass whatever the palette said.
    ///
    /// @param app - the panel to draw.
    /// @param size - the window to draw it in.
    /// @returns one entry per drawn run of text, with its rectangle and colour.
    fn drawn_text_coloured(
        app: &mut App,
        size: egui::Vec2,
    ) -> Vec<(String, egui::Rect, egui::Color32)> {
        let ctx = egui::Context::default();
        crate::ui::fonts::ensure_icons(&ctx);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| app.draw(ui));
        output.textures_delta.clear();
        let mut texts = Vec::new();
        for shape in output.shapes {
            collect_text_coloured(&shape.shape, &mut texts);
        }
        texts
    }

    /// The recursive half of [`drawn_text_coloured`].
    ///
    /// @param shape - the shape to walk.
    /// @param out - where entries are collected.
    fn collect_text_coloured(
        shape: &egui::Shape,
        out: &mut Vec<(String, egui::Rect, egui::Color32)>,
    ) {
        match shape {
            egui::Shape::Text(text) => {
                // The colour a run of text is *really* painted in. `TextShape::fallback_color` is
                // only used for sections whose own colour is transparent — a label with an explicit
                // colour (which is most of this panel) carries it in the layout job instead. Reading
                // only the fallback would report `TRANSPARENT`-resolved defaults for exactly the text
                // whose colour is being questioned.
                let colour = text
                    .galley
                    .job
                    .sections
                    .first()
                    .map(|section| section.format.color)
                    .filter(|colour| *colour != egui::Color32::TRANSPARENT)
                    .unwrap_or(text.fallback_color);
                out.push((text.galley.text().to_owned(), text.visual_bounding_rect(), colour));
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect_text_coloured(shape, out);
                }
            }
            _ => {}
        }
    }

    /// Walk a shape tree, collecting every piece of text and the rectangle it occupies.
    fn collect_text(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
        match shape {
            egui::Shape::Text(text) => {
                out.push((text.galley.text().to_owned(), text.visual_bounding_rect()));
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect_text(shape, out);
                }
            }
            _ => {}
        }
    }


    /// Drawing a conversation with a frame this build cannot read must not panic.
    ///
    /// A panic inside the layout closure takes the panel down with no message the user can act on, and
    /// the code that draws blocks, tool output and notices is the code with the most ways to get an
    /// index wrong.
    #[test]
    fn drawing_a_conversation_does_not_panic() {
        let (mut app, recorded, session, _wake) = app_and_session();
        deliver(&session, &recorded, conversation_frame());
        let drawn = drawn_text(&mut app, egui::vec2(708.0, 620.0));
        assert!(!drawn.is_empty(), "the panel draws a conversation at all");

        // Deliberately a delta kind this build does not know, and an unterminated tool input:
        // the two shapes most likely to be mis-indexed by a drawer.
        deliver(
            &session,
            &recorded,
            Inbound::Notification {
                method: "session/stream".to_owned(),
                params: Some(serde_json::json!({
                    "sessionId": "session-1", "generation": 1,
                    "frame": {"type": "chunk", "revision": 9, "index": 4, "time": 5,
                              "chunk": {"type": "tool-input-delta", "index": 2, "delta": "{\"command\":"}},
                })),
            },
        );
        let after = drawn_text(&mut app, egui::vec2(708.0, 620.0));
        assert!(!after.is_empty(), "and it still draws with a frame it cannot interpret");
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
    fn a_send_with_no_conversation_starts_one() {
        // The panel used to refuse this, and the refusal was correct for the old design and wrong
        // for this one: with nothing pinned the panel is *meant* to start a conversation, and the
        // text waits in the session until the creation has finished.
        //
        // The whole three-step sequence — create, attach, then prompt — is proved at the session
        // layer, where the answers can be fed back in order. What this test is for is the wiring
        // above it: that a click in the panel reaches that path at all, that the ladder's workspace
        // goes out with the request, and that the user's text is taken out of the box because the
        // session is now holding it. The first version of this test sent its `hello` as a
        // notification and so never completed the handshake; it asserted against a panel that was
        // still waiting to say hello, and passed for that reason alone.
        let (mut app, recorded, session, _wake) = app_and_session();
        {
            let mut sink = RecordingSink(recorded.clone());
            session.lock().expect("session").start(&mut sink);
        }
        answer(
            &session,
            &recorded,
            "hello",
            serde_json::json!({"sessionId": "host-1", "hostVersion": "test"}),
        );
        app.logic();
        // The host's answer to the list the handshake asked for. It names one *old* conversation in
        // the workspace, which is what the ladder's last rung looks at: with nothing pinned and no
        // turn in flight, "the workspace with the newest conversation in it" is the only evidence of
        // where a new conversation should go.
        answer(
            &session,
            &recorded,
            "sessions/list",
            serde_json::json!({"items": [
                {"sessionId": "older", "updatedAt": 10, "cwd": "/work/project"},
            ]}),
        );
        // One more frame: the answer above released the one-request-at-a-time slot, and this is what
        // lets the panel's own loop ask for the workspace list it needs to climb that rung.
        {
            let mut sink = RecordingSink(recorded.clone());
            session.lock().expect("session").request_workspaces(&mut sink);
        }
        answer(
            &session,
            &recorded,
            "workspaces/list",
            serde_json::json!({"items": [{"workspaceId": "ws-1", "title": "project", "path": "/work/project"}]}),
        );

        app.draft = "你好".to_owned();
        apply(&mut app, crate::ui::Action::Send { text: "你好".to_owned() });

        let frames = recorded.frames.lock().expect("frames").clone();
        let create = frames
            .iter()
            .rev()
            .find(|frame| frame["method"] == "session/create")
            .unwrap_or_else(|| panic!("a conversation is created on submit: {frames:?}"));
        assert_eq!(
            create["params"]["workspaceId"], "ws-1",
            "in the workspace the ladder chose, and never an empty id",
        );
        assert!(
            !frames.iter().any(|frame| frame["method"] == "session/prompt"),
            "and the prompt waits for it: {frames:?}",
        );
        assert_eq!(app.draft, "", "the box is free again: the text is in the session's hands");
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
        // Attached, so the composer is in its normal state: its placeholder changes when there
        // is no conversation, and this test is about the layout rather than about that.
        attach_one(&mut app, &recorded, &session);
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
    /// Leaving a conversation is something the host has to hear about.
    ///
    /// The host decides who answers an approval, and it knows the panel's conversation from
    /// the attach it was asked for. A panel that quietly stopped showing one would go on
    /// claiming requests for it — so "start a new conversation" tells the host to detach.
    #[test]
    fn starting_a_new_conversation_tells_the_host_to_detach() {
        let (mut app, recorded, session, _wake) = app_and_session();
        attach_one(&mut app, &recorded, &session);
        apply(&mut app, crate::ui::Action::NewConversation);

        let frames = recorded.frames.lock().expect("frames").clone();
        let detach = frames
            .iter()
            .find(|frame| frame["method"] == "session/detach")
            .expect("the host is told");
        assert_eq!(detach["params"]["sessionId"], "session-1");
        assert!(detach.get("id").is_none(), "a fact, not a question: notifications carry no id");
    }

    /// "Start a new conversation" is a decision about the panel, not a request to the host:
    /// nothing is created until the user has something to say.
    #[test]
    fn starting_a_new_conversation_creates_nothing_yet() {
        let (mut app, recorded, session, _wake) = app_and_session();
        let path = std::env::temp_dir().join(format!("quorfloat-new-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        app.note_pinned_path(path.clone());
        attach_one(&mut app, &recorded, &session);
        apply(&mut app, crate::ui::Action::PinConversation { session_id: Some("session-1".to_owned()) });
        assert_eq!(session.lock().expect("session").pinned_conversation(), Some("session-1"));

        apply(&mut app, crate::ui::Action::NewConversation);
        let guard = session.lock().expect("session");
        assert_eq!(guard.pinned_conversation(), None, "nothing is pinned any more");
        assert_eq!(guard.follow().session_id(), None, "and nothing is attached");
        assert_eq!(guard.transcript().session_id(), None, "the transcript was let go too");
        drop(guard);
        assert!(!path.exists(), "and the remembered pin is gone from the file");

        let frames = recorded.frames.lock().expect("frames").clone();
        assert!(
            !frames.iter().any(|frame| frame["method"] == "session/create"),
            "no conversation is created until there is something to say: {frames:?}",
        );
        let _ = std::fs::remove_file(&path);
    }

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
    /// The whole way in: click the box, type, press Enter, and a prompt goes out.
    ///
    /// Written because "my input will not send" is a claim about a chain — the click reaching
    /// the editor, the editor taking focus, Enter not being taken as a newline, the send
    /// button's rule, and the session accepting the prompt — and every one of those links has
    /// its own test except the first two. This is the one that says the chain holds.
    #[test]
    fn typing_and_pressing_enter_sends_a_prompt() {
        let (mut app, recorded, session, _wake) = app_and_session();
        // A conversation to send to, attached the way the host attaches one. Without one the
        // panel refuses *with a reason*, which is the failure this test exists to tell apart
        // from a broken input path.
        attach_one(&mut app, &recorded, &session);
        assert_eq!(
            // The *attached* conversation, not `target_conversation`, which is only the
            // deliberate target and stays empty while the panel follows the newest one.
            session.lock().expect("session").follow().session_id(),
            Some("session-1"),
            "the panel is attached before anything is typed",
        );
        let ctx = egui::Context::default();
        crate::ui::fonts::ensure_icons(&ctx);
        let size = egui::vec2(708.0, 620.0);
        // Inside the composer's editor: past the search mark, below the top bar.
        let editor = egui::pos2(
            f32::from(crate::ui::theme::SHADOW_ROOM_SIDE + crate::ui::theme::PAD_COMPOSER.left) + 60.0,
            f32::from(crate::ui::theme::SHADOW_ROOM_TOP) + 95.0,
        );
        let plan: Vec<Vec<egui::Event>> = vec![
            vec![egui::Event::PointerMoved(editor)],
            vec![egui::Event::PointerButton {
                pos: editor,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
            vec![egui::Event::PointerButton {
                pos: editor,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
            vec![egui::Event::Text("你好".to_owned())],
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        ];
        for (index, events) in plan.into_iter().enumerate() {
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
            let _ = index;
        }

        let frames = recorded.frames.lock().expect("not poisoned").clone();
        let sent: Vec<&serde_json::Value> = frames
            .iter()
            .filter(|frame| frame["method"] == "session/prompt")
            .collect();
        assert_eq!(sent.len(), 1, "one prompt went out: {frames:?}");
        assert_eq!(sent[0]["params"]["text"], "你好");
        assert_eq!(app.draft, "", "and the box was cleared");
    }


    /// The invariant behind "the panel looks cut off at the bottom".
    ///
    /// The window is sized from one number — what the drawing layer says the content wants —
    /// so nothing may be drawn below it. The first version of the dynamic height broke this in
    /// the compact case: the conversation area *filled* the space it was offered while the
    /// height it reported was the height of its content, and the footer ended up 40 pixels
    /// past the bottom of the panel's own window.
    /// The three pieces of the design's thread: the question line, the separator between
    /// turns, and the answer bar with its copy control.
    #[test]
    fn a_turn_is_a_question_a_separator_and_an_answer_bar() {
        let (mut app, recorded, session, _wake) = app_and_session();
        attach_one(&mut app, &recorded, &session);
        deliver(&session, &recorded, conversation_frame());
        let size = egui::vec2(708.0, 620.0);

        let shown = drawn_text(&mut app, size);
        let all: String = shown.iter().map(|(text, _)| text.as_str()).collect::<Vec<_>>().join("\n");
        assert!(all.contains("你 · 帮我看看"), "the question is a signpost: {all}");
        assert!(all.contains("回答完成"), "and the answer says whether it is finished: {all}");
        assert!(all.contains("复制回答"), "with a copy control beside it: {all}");
        assert!(!all.contains("正在生成"), "which is not offered while nothing is running: {all}");
    }

    /// A finished answer is rendered as Markdown.
    ///
    /// The markers are what tell the two renderings apart: a plain-text draw shows `**粗体**`
    /// as written, and a Markdown draw shows the words without them.
    #[test]
    fn a_finished_answer_is_rendered_as_markdown() {
        let (mut app, recorded, session, _wake) = app_and_session();
        attach_one(&mut app, &recorded, &session);
        deliver(&session, &recorded, conversation_frame());
        let size = egui::vec2(708.0, 620.0);

        let shown = drawn_text(&mut app, size);
        let all: String = shown.iter().map(|(text, _)| text.as_str()).collect::<Vec<_>>().join("\n");
        assert!(all.contains("看到了"), "the answer is on screen: {all}");
        assert!(all.contains("重点"), "and its emphasis is rendered as text: {all}");
        assert!(
            !all.contains("**"),
            "and its Markdown markers are not, because they were rendered: {all}",
        );
        assert!(
            all.contains("let name"),
            "including the code block's contents: {all}",
        );
    }

    /// Reasoning is folded away by default, and one click opens it.
    ///
    /// The request came from using the panel: a chain of thought on screen for every answer
    /// buries the answer. It is kept rather than dropped, because a long silence with no visible
    /// work is how a slow model looks broken.
    #[test]
    fn reasoning_is_folded_away_until_it_is_asked_for() {
        let (mut app, recorded, session, _wake) = app_and_session();
        attach_one(&mut app, &recorded, &session);
        deliver(&session, &recorded, conversation_frame());
        let size = egui::vec2(708.0, 620.0);

        let folded = drawn_text(&mut app, size);
        assert!(
            folded.iter().any(|(text, _)| text.contains("思考")),
            "the fold is offered: {folded:#?}",
        );
        assert!(
            !folded.iter().any(|(text, _)| text.contains("想一下")),
            "and its contents are not on screen: {folded:#?}",
        );
        // The answer itself is there, which is the point of folding the working-out.
        assert!(folded.iter().any(|(text, _)| text.contains("看到了")), "{folded:#?}");
    }

    #[test]
    fn nothing_is_drawn_below_the_height_the_panel_asked_its_window_for() {
        let (mut app, _recorded, _session, _wake) = app_and_session();
        let ctx = egui::Context::default();
        crate::ui::fonts::ensure_icons(&ctx);
        let size = egui::vec2(708.0, 620.0);
        let mut lowest = f32::MIN;
        let mut allowed = 0.0;
        // A few passes: the first measures, the second draws with that answer, and the third
        // is there so that "it settled" is part of what is asserted.
        for _ in 0..3 {
            let mut layout = None;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    focused: true,
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                },
                |ui| layout = Some(app.draw_panel(ui)),
            );
            let layout = layout.expect("the panel was drawn");
            allowed = f32::from(crate::ui::theme::SHADOW_ROOM_TOP) + layout.desired_height;
            for clipped in &output.shapes {
                if let egui::Shape::Text(text) = &clipped.shape {
                    lowest = lowest.max(text.pos.y + text.galley.size().y);
                }
            }
            output.textures_delta.clear();
        }
        assert!(
            lowest <= allowed + 1.0,
            "the lowest text is at {lowest}, but the panel asked for {allowed}",
        );
    }

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
    ///
    /// Settings are remembered nowhere, so a test that is not about them leaves no trace.
    fn app_and_session() -> (App, Recorded, Arc<Mutex<Session>>, Sender<Wake>) {
        app_and_session_with(std::path::Path::new(""))
    }

    /// The same, remembering settings in a named file.
    ///
    /// @param preferences - where to remember them, or an empty path for nowhere.
    /// @returns the app, its frames, its session and its wake channel.
    fn app_and_session_with(
        preferences: &std::path::Path,
    ) -> (App, Recorded, Arc<Mutex<Session>>, Sender<Wake>) {
        let session = Arc::new(Mutex::new(Session::new(identity())));
        let recorded = Recorded::default();
        let sink = Arc::new(SharedSink::new(Box::new(RecordingSink(recorded.clone()))));
        let (tx, rx) = channel();
        let hotkey = Hotkey::unavailable_for_test("Alt+Space", "test");
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
            // Empty means nowhere to write.
            (!preferences.as_os_str().is_empty()).then(|| preferences.to_path_buf()),
        );
        (app, recorded, session, tx)
    }

    /// Take the panel through a handshake and onto one conversation.
    ///
    /// The decision in the middle is the user's: with the "follow the newest" rule gone, a
    /// conversation is attached because it was chosen — which is what the panel does when
    /// somebody picks one in the picker.
    ///
    /// @param app - the panel, whose follow loop is pumped the way `logic` pumps it.
    /// @param recorded - the frames sent so far.
    /// @param session - the session being driven.
    fn attach_one(app: &mut App, recorded: &Recorded, session: &Arc<Mutex<Session>>) {
        {
            let mut sink = RecordingSink(recorded.clone());
            session.lock().expect("session").start(&mut sink);
        }
        answer(session, recorded, "hello", serde_json::json!({"sessionId": "host-1", "hostVersion": "test"}));
        app.logic();
        answer(
            session,
            recorded,
            "sessions/list",
            serde_json::json!({"items": [{"sessionId": "session-1", "updatedAt": 1, "cwd": "/work/project"}]}),
        );
        {
            let mut sink = RecordingSink(recorded.clone());
            session.lock().expect("session").choose_conversation("session-1", &mut sink);
        }
        answer(session, recorded, "session/attach", serde_json::json!({"generation": 1}));
    }

    /// Answer the last request the panel made for one method, the way the host would.
    ///
    /// The id is read off the wire rather than assumed: request ids are the session's, and a
    /// test that guesses them is a test that stops testing anything the day they change.
    fn answer(
        session: &Arc<Mutex<Session>>,
        recorded: &Recorded,
        method: &str,
        result: serde_json::Value,
    ) {
        let id = recorded
            .frames
            .lock()
            .expect("frames")
            .iter()
            .rev()
            .find(|frame| frame["method"] == method)
            .unwrap_or_else(|| panic!("the panel asked for {method}"))["id"]
            .clone();
        deliver(session, recorded, Inbound::Response { id, outcome: Ok(result) });
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

    /// Long unbroken runs get break opportunities, and only for display.
    ///
    /// The panel's width is fixed, and egui never breaks inside a word unless it is truncating,
    /// so a code block holding one long JSON line painted past the panel's edge. The break
    /// opportunities are zero-width, and the text a copy takes is untouched.
    #[test]
    fn long_runs_are_given_something_to_break_on() {
        let source = "{\"key\":\"value\"}";
        let long = format!("```json\\n{}\\n```", source.repeat(6));
        let wrapped = crate::ui::soft_wrap_for_display(&long);
        assert!(wrapped.contains('\u{200b}'), "a break opportunity was added");
        assert!(
            wrapped.chars().filter(|c| *c != '\u{200b}').eq(long.chars()),
            "and nothing else changed: same characters, in the same order",
        );
        // Ordinary text is left alone, including the Chinese that already breaks per character.
        let prose = "这是一段普通的话，长度不足以需要断行。";
        assert_eq!(crate::ui::soft_wrap_for_display(prose), prose);
        assert_eq!(crate::ui::soft_wrap_for_display("a short line"), "a short line");
    }

    /// A conversation whose answer contains a table.
    fn table_frame() -> Inbound {
        Inbound::Notification {
            method: "session/snapshot".to_owned(),
            params: Some(serde_json::json!({
                "sessionId": "session-1", "generation": 1, "cursor": 1, "hasMore": false,
                "records": [
                    {"type": "event", "event": {"type": "user/message", "seq": 0, "time": 1,
                     "data": {"role": "user", "content": [{"type": "text", "text": "对比一下"}],
                              "source": {"kind": "user"}}}},
                    {"type": "event", "event": {"type": "assistant/message", "seq": 1, "time": 2,
                     "data": {"message": {"role": "assistant", "content": [{"type": "text",
                        "text": "两种做法：\n\n| 方案 | 做法 | 代价 |\n| --- | --- | --- |\n| A | 只给表格套滚动区 | 仍需手势滚动 |\n| B | 自绘表格，单元格内换行 | 需要自己实现 |\n"}]}}}},
                ],
            })),
        }
    }

    /// The bug this work started from: a table painted past the panel's edge.
    ///
    /// The viewer draws tables as an `egui::Grid`, which measures its columns from the content,
    /// so a wide cell asked for more width than the panel has. Read from the painted rectangles
    /// rather than from the layout's opinion of itself.
    #[test]
    fn a_table_stays_inside_the_panel() {
        let (mut app, recorded, session, _wake) = app_and_session();
        attach_one(&mut app, &recorded, &session);
        deliver(&session, &recorded, table_frame());
        let size = egui::vec2(708.0, 620.0);

        let drawn = drawn_text(&mut app, size);
        let texts: Vec<&str> = drawn.iter().map(|(text, _)| text.as_str()).collect();
        assert!(texts.iter().any(|text| text.contains("方案")), "the header: {texts:?}");
        assert!(texts.iter().any(|text| text.contains("单元格内换行")), "and its cells: {texts:?}");
        let widest = drawn.iter().map(|(_, rect)| rect.right()).fold(f32::MIN, f32::max);
        let panel_right = size.x - f32::from(crate::ui::theme::SHADOW_ROOM_SIDE);
        assert!(widest <= panel_right, "nothing paints past the panel: {widest} > {panel_right}");
    }

    /// A card is built from the panel's own tokens, and its two actions are one shape.
    ///
    /// The design draws exactly one filled control per view and leaves the rest as surfaces
    /// or text; the approval card used to have two filled buttons, a green and a red, which is
    /// a second vocabulary inside one panel. Asserted on the widgets rather than on a rendering:
    /// the introspection helper reports text galleys, not control rects, so button *geometry* is
    /// not something it can see — but the choices themselves are.
    #[test]
    fn a_card_is_built_from_the_panels_own_tokens() {
        let frame = crate::ui::theme::card_frame();
        assert_eq!(frame.fill, crate::ui::theme::soft(), "the surface every other one uses");
        assert_eq!(
            frame.corner_radius,
            egui::CornerRadius::same(crate::ui::theme::RADIUS_POPOVER),
            "a card is a surface inside the panel, like a popover",
        );
        assert_eq!(frame.inner_margin, crate::ui::theme::PAD_CARD, "its own padding, from the tokens");

        // The two actions are one shape because the same pair of helpers builds both, and they
        // are the only buttons a card constructs. `egui::Button`'s fields are private, so the
        // shape itself is verified by looking at the panel — which is what a real approval was
        // for (see `docs/progress.md` §38).
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

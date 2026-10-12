//! Following a conversation.
//!
//! The panel answers approvals for the conversation it is *showing*, which the host
//! decides by `session/attach`: an interaction is claimed only for an attached session.
//! With nothing attached the panel owns
//! nothing, so every approval is answered by the Harness window — and from the
//! outside that is indistinguishable from a panel that is broken.
//!
//! **Which** conversation to follow is decided by three rules, in order: the pin, the
//! choice made for this run, and the most recently updated conversation as a fallback
//! (see `Follow::target`).
//!
//! Discovery is a poll, which this project otherwise avoids, because the protocol
//! has no "a session appeared" notification. The cost is one small request every
//! [`POLL_INTERVAL_MS`], and it stops being necessary the moment the host can tell
//! the panel that a conversation started.
//!
//! Everything here is state and decisions, with time passed in, so the whole
//! sequence is testable without a host, a window, or a clock.

use serde_json::Value;

use crate::app::session::FrameSink;
use crate::ipc::protocol::error_code;

/// How often to look for a conversation that is more recent than the current one.
///
/// Short enough that a conversation started in the Harness window is picked up
/// before its first approval (a model turn takes seconds), long enough that the
/// request is noise. One small request per interval is the whole cost.
pub const POLL_INTERVAL_MS: i64 = 3000;

/// How long to wait for an answer before assuming the request was lost.
///
/// Without this a single dropped response would stop discovery for the life of the
/// process: the panel would sit there following nothing, looking exactly like a
/// panel with nothing to do. The page request uses the same budget for the same
/// reason — a busy "loading earlier" that never clears is a control the user cannot
/// use again.
pub const REQUEST_TIMEOUT_MS: i64 = 10_000;

/// How many consecutive list revisions must omit the pinned conversation before it is forfeit.
///
/// One is not evidence: a conversation the user just created, or just pinned, may not have
/// reached the host's list yet, and a panel that takes a pin away on a slow answer is a panel
/// that answers a question nobody asked. Two consecutive revisions is the conservative floor —
/// by then the list has been read twice, seconds apart, and the conversation is still not there.
const MISSING_REVISIONS_BEFORE_UNPIN: u32 = 2;

/// What the follow layer wants to ask the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outgoing {
    /// Ask which conversations exist.
    ListSessions,
    /// Ask which workspaces exist.
    ListWorkspaces,
    /// Ask the host to initialize (or resolve) its first-use default workspace.
    ///
    /// The host creates and registers it, so this is a command, not a lookup: it is how an
    /// empty list becomes a usable one without sending the user to the Harness window.
    DefaultWorkspace,
    /// Create a conversation, in a chosen workspace.
    CreateSession {
        /// The workspace to create it in, or `None` for the host's own default.
        workspace_id: Option<String>,
    },
    /// Ask the host to restart or stop this process.
    PanelLifecycle {
        /// Which of the two.
        action: PanelLifecycle,
    },
    /// Subscribe to one of them.
    Attach {
        /// Durable session identity.
        session_id: String,
    },
    /// Read one older page of the attached conversation's history.
    ///
    /// The one variant the follow layer does not decide: it exists so the page request
    /// travels on the same single-request path as everything else this process asks
    /// (one id, one answer, one timeout), and so the wire frame is built in one place.
    History {
        /// The conversation the page belongs to.
        session_id: String,
        /// The exclusive backwards cursor: ask for the page below this sequence. This is
        /// the Harness's own `beforeSeq` (`SessionPageRequest.beforeSeq`), and its value is
        /// the sequence of the oldest line the panel currently shows.
        before_seq: i64,
    },
}

/// A lifecycle action the panel asks the host to perform on this process.
///
/// The panel cannot do either of these itself: replacing or ending the process is the
/// supervisor's business, and an exit it reads as a crash would simply be restarted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelLifecycle {
    /// Replace this process with a fresh one.
    Restart,
    /// Stop this process on purpose, so no automatic restart follows.
    Stop,
}

impl PanelLifecycle {
    /// The wire spelling the host's router accepts.
    #[must_use]
    pub fn wire(self) -> &'static str {
        match self {
            Self::Restart => "restart",
            Self::Stop => "stop",
        }
    }
}

/// What a request was for, so its response can be read.
#[derive(Debug, Clone, PartialEq, Eq)]
enum InFlight {
    /// A `sessions/list` request, sent at this time.
    List { sent_at: i64 },
    /// A `session/attach` request for this session, sent at this time.
    Attach {
        /// Durable session identity.
        session_id: String,
        /// The workspace name that came with it, kept for the status line.
        label: Option<String>,
        /// When it was sent.
        sent_at: i64,
    },
    /// A `workspaces/list` request, sent at this time.
    Workspaces {
        /// When it was sent.
        sent_at: i64,
    },
    /// A `workspace/default` request, sent at this time.
    DefaultWorkspace {
        /// When it was sent.
        sent_at: i64,
    },
    /// A `session/create` request, sent at this time.
    Create {
        /// When it was sent.
        sent_at: i64,
    },
}

impl InFlight {
    /// When this request was sent.
    fn sent_at(&self) -> i64 {
        match self {
            Self::List { sent_at }
            | Self::Workspaces { sent_at }
            | Self::DefaultWorkspace { sent_at }
            | Self::Create { sent_at } => *sent_at,
            Self::Attach { sent_at, .. } => *sent_at,
        }
    }
}

/// One workspace the user may choose, as the host describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    /// Durable workspace identity.
    pub workspace_id: String,
    /// What the Harness calls it.
    pub title: String,
    /// Its directory, which is what tells two same-named workspaces apart.
    pub path: String,
    /// The conversations the registry accounts to it, in its own order.
    ///
    /// Membership, not the directory: a conversation can be moved out of a workspace without
    /// its `cwd` changing, and this list is the only account of which is which. It may name a
    /// conversation the session list no longer shows, so a consumer intersects it with the
    /// conversations it can actually see.
    pub session_ids: Vec<String>,
    /// When the workspace was registered, in epoch milliseconds; `0` when the host did not say.
    pub created_at: i64,
}

/// Why a half of the pin was forgotten, for the status line to explain.
///
/// A *reason*, never a sentence: the panel's copy lives in the frontend dictionaries, and a
/// user-facing string does not originate in this process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinNotice {
    /// The pinned conversation is not in the list any more.
    SessionGone,
    /// The pinned workspace is not in the list any more.
    WorkspaceGone,
}

impl PinNotice {
    /// The wire spelling the snapshot carries.
    #[must_use]
    pub fn wire(self) -> &'static str {
        match self {
            Self::SessionGone => "sessionGone",
            Self::WorkspaceGone => "workspaceGone",
        }
    }
}

/// What came of asking the host for its first-use default workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DefaultWorkspaceState {
    /// Not asked — the list has not been read yet, or it already had workspaces.
    #[default]
    Idle,
    /// Asked and answered: there is now a workspace to create in.
    Done,
    /// Asked and there is still no workspace to use: the host refused, failed, or declined.
    Failed,
}

impl DefaultWorkspaceState {
    /// The wire spelling the snapshot carries.
    #[must_use]
    pub fn wire(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }
}

/// The conversation this panel follows.
#[derive(Debug, Default)]
pub struct Follow {
    /// Set once the handshake has completed; discovery does not start before that.
    started: bool,
    /// The conversation currently followed, if any.
    session_id: Option<String>,
    /// Something human-readable about it — the workspace directory's name.
    label: Option<String>,
    /// The subscription generation the host reported for it.
    generation: Option<i64>,
    /// The request awaiting an answer, if any.
    in_flight: Option<InFlight>,
    /// Earliest time the next discovery request may be sent.
    next_poll_at: i64,
    /// Persistent events seen for the followed conversation.
    events: u64,
    /// Temporary assistant-stream frames seen for it.
    streams: u64,
    /// Times the host told us our view had a hole in it.
    resyncs: u64,
    /// Frames discarded because they belonged to a replaced subscription.
    stale: u64,
    /// Highest persistent sequence seen, for the gap check.
    last_seq: Option<i64>,
    /// The sequence whose jump was already reported, so one hole is one line.
    gap_reported_at: Option<i64>,
    /// The conversations the host last listed, newest first.
    sessions: Vec<SessionSummary>,
    /// The workspaces the host last listed.
    workspaces: Vec<Workspace>,
    /// The conversation the user picked for this run, if any.
    ///
    /// A *choice* outranks "the newest conversation" and is forgotten when the panel is
    /// reopened: it is what "switch to that one" means.
    choice: Option<String>,
    /// The conversation the user pinned, if any.
    ///
    /// A *pin* outranks both, and is kept across runs: it is what "always open this one"
    /// means. The two are separate because they answer different questions, and collapsing
    /// them would make switching conversations silently permanent.
    pinned: Option<String>,
    /// The workspace the user pinned as the default for a new conversation, if any.
    ///
    /// **Independent of [`Follow::pinned`].** The session pin decides which conversation opens;
    /// the workspace pin decides where a *new* one is created. A user gesture sets one at a
    /// time and never clears the other, and the program clears each half only when that half
    /// itself has been shown to be invalid.
    pinned_workspace: Option<String>,
    /// How many consecutive list revisions have omitted the pinned conversation.
    absent_revisions: u32,
    /// Whether a refused attach has already cost the pin during this visit.
    ///
    /// An attach that is refused is only evidence when the refusal is definitive, and even then
    /// it is worth one action per visit: the pin is gone after the first, and a later refusal
    /// in the same visit must not report the same loss again.
    refused_this_visit: bool,
    /// Why a half of the pin was forgotten, until the user summons again.
    pin_notice: Option<PinNotice>,
    /// Whether this layer forgot a half of the pin since the shell last wrote the pin file.
    pins_changed: bool,
    /// Whether the workspace list has been answered at least once.
    ///
    /// A list that has not answered is no evidence: an empty list before the first answer is
    /// "unknown", not "no workspaces", and clearing a pin on it would be reading silence as a
    /// fact.
    workspaces_loaded: bool,
    /// What came of asking for the host's first-use default workspace.
    default_workspace: DefaultWorkspaceState,
    /// Whether that ask was already made, so an empty list asks once.
    default_requested: bool,
    /// Whether the workspace list has been asked for yet.
    workspaces_asked: bool,
    /// Whether the next free poll slot should ask for the workspace list.
    ///
    /// Set by a summon: the workspace field's ladder (the pin, the most recent workspace, the
    /// host's default) is answered from that list, and a panel whose user just called it up has
    /// to be able to say where a new conversation would start. Deferred to the poll loop rather
    /// than sent from the summon itself because this layer runs one request at a time, and the
    /// summon is already sending the attach that returns to a pinned conversation.
    workspaces_wanted: bool,
    /// The workspace a `session/create` was asked for, kept for the log line that says
    /// where the conversation the user is now in was created.
    choosing_workspace: Option<String>,
    /// Why the last creation failed, until somebody reads it.
    create_failure: Option<String>,
    /// Whether the empty list has already been mentioned.
    said_empty: bool,
}

impl Follow {
    /// Ask for the workspace list, which the picker needs and discovery does not.
    ///
    /// On demand rather than at startup: a panel whose user never opens the picker should
    /// not spend a request on it, and workspaces change when someone adds one in the Harness
    /// rather than between two polls. The answer is then cached until asked for again.
    ///
    /// @param now - caller-local time in milliseconds.
    /// @returns the request to send, or `None` when one is already outstanding.
    pub fn request_workspaces(&mut self, now: i64) -> Option<Outgoing> {
        // One request at a time is the discipline this layer runs on, and the answer to
        // whatever is in flight may be more interesting than this list.
        if self.in_flight.is_some() {
            return None;
        }
        self.workspaces_asked = true;
        self.workspaces_wanted = false;
        self.in_flight = Some(InFlight::Workspaces { sent_at: now });
        Some(Outgoing::ListWorkspaces)
    }

    /// Give up the current attachment, without telling the host anything.
    ///
    /// Used when the conversation being followed has gone: the subscription is dead at the
    /// host's end anyway, and what matters here is that the panel stops behaving as if it were
    /// showing something.
    fn forget_attachment(&mut self) {
        self.session_id = None;
        self.generation = None;
        self.last_seq = None;
        self.gap_reported_at = None;
    }

    /// Take the reason the last creation failed, if there is one.
    ///
    /// Reading it clears it, so that one failure is reported once.
    ///
    /// @returns the host's message, if a creation failed since this was last read.
    pub fn take_create_failure(&mut self) -> Option<String> {
        self.create_failure.take()
    }

    /// The reason the last creation failed, without consuming it.
    ///
    /// The frontend's snapshot is a read: consuming a failure there would drop it
    /// before anything could show it. The settings page, which clears it, uses
    /// [`Follow::take_create_failure`].
    ///
    /// @returns the host's message, if a creation failed.
    #[must_use]
    pub fn create_failure(&self) -> Option<&str> {
        self.create_failure.as_deref()
    }

    /// Whether the workspace list has ever been asked for.
    #[must_use]
    pub fn workspaces_asked(&self) -> bool {
        self.workspaces_asked
    }

    /// Whether the workspace list has been answered at least once.
    #[must_use]
    pub fn workspaces_loaded(&self) -> bool {
        self.workspaces_loaded
    }

    /// What came of asking the host for its first-use default workspace.
    #[must_use]
    pub fn default_workspace(&self) -> DefaultWorkspaceState {
        self.default_workspace
    }

    /// The workspace pinned as the default for a new conversation, if any.
    #[must_use]
    pub fn pinned_workspace(&self) -> Option<&str> {
        self.pinned_workspace.as_deref()
    }

    /// Why a half of the pin was forgotten, while the explanation is still current.
    #[must_use]
    pub fn pin_notice(&self) -> Option<PinNotice> {
        self.pin_notice
    }

    /// Take the fact that a half of the pin was forgotten, so the shell can write the file.
    ///
    /// Reading clears it: one write per invalidation, not one per tick.
    ///
    /// @returns whether anything is waiting to be persisted.
    pub fn take_pins_changed(&mut self) -> bool {
        std::mem::take(&mut self.pins_changed)
    }

    /// Start a visit: a summon is a new visit, and an explanation of the last one is over.
    ///
    /// It also marks the workspace list as wanted, so the first free poll slot asks for it: the
    /// new-conversation ladder is answered from that list, and the panel has just been called up
    /// to show it.
    pub fn begin_visit(&mut self) {
        self.pin_notice = None;
        self.refused_this_visit = false;
        self.workspaces_wanted = true;
    }

    /// Pin (or unpin) the workspace a new conversation defaults to.
    ///
    /// The session half is not touched: the two pins answer different questions, and a gesture
    /// on one of them is not a statement about the other.
    ///
    /// @param workspace_id - which workspace, or `None` to stop pinning one.
    pub fn set_pinned_workspace(&mut self, workspace_id: Option<String>) {
        let next = workspace_id
            .map(|id| id.trim().to_owned())
            .filter(|id| !id.is_empty());
        if self.pinned_workspace != next {
            self.pins_changed = true;
        }
        self.pinned_workspace = next;
        self.pin_notice = None;
    }

    /// Ask for the host's first-use default workspace, when there is nothing else to use.
    ///
    /// Asked once, and only about a list that has actually been read and found empty: a list
    /// that has not answered, or one that already names a workspace, has nothing to fix.
    ///
    /// @param now - caller-local time in milliseconds.
    /// @returns the request to send, or `None` when it is not wanted.
    fn ask_for_default_workspace(&mut self, now: i64) -> Option<Outgoing> {
        if !self.workspaces_loaded || !self.workspaces.is_empty() || self.default_requested {
            return None;
        }
        self.default_requested = true;
        self.in_flight = Some(InFlight::DefaultWorkspace { sent_at: now });
        Some(Outgoing::DefaultWorkspace)
    }

    /// Record why a half of the pin was forgotten, keeping the first explanation of a visit.
    ///
    /// The snapshot carries one reason, and the session pin — the panel's *home* — is the one a
    /// user is most likely to be looking for, so the caller checks it first.
    fn note_pin_notice(&mut self, notice: PinNotice) {
        if self.pin_notice.is_none() {
            self.pin_notice = Some(notice);
        }
    }

    /// Forget the pinned conversation if two consecutive list revisions have omitted it.
    ///
    /// @param sink - where the explanation goes.
    fn reconcile_session_pin(&mut self, sink: &mut dyn FrameSink) {
        let Some(pinned) = self.pinned.clone() else {
            self.absent_revisions = 0;
            return;
        };
        if self.sessions.iter().any(|summary| summary.session_id == pinned) {
            self.absent_revisions = 0;
            return;
        }
        self.absent_revisions += 1;
        if self.absent_revisions < MISSING_REVISIONS_BEFORE_UNPIN {
            // Said out loud even when nothing is cleared: "the pin survived one silent list" is
            // the fact that explains why the panel is still showing something the host did not
            // mention.
            sink.mark(&format!("pin {pinned} absent from a list revision; keeping it"));
            return;
        }
        sink.log(&format!("the pinned conversation {pinned} is gone; forgetting the pin"));
        sink.mark(&format!("unpin {pinned} (absent from two list revisions)"));
        self.pinned = None;
        self.absent_revisions = 0;
        self.pins_changed = true;
        self.note_pin_notice(PinNotice::SessionGone);
        if self.session_id.as_deref() == Some(pinned.as_str()) {
            self.forget_attachment();
        }
    }

    /// Forget the pinned workspace if the list it was validated against no longer names it.
    ///
    /// @param sink - where the explanation goes.
    fn reconcile_workspace_pin(&mut self, sink: &mut dyn FrameSink) {
        let Some(pinned) = self.pinned_workspace.clone() else {
            return;
        };
        if self.workspaces.iter().any(|workspace| workspace.workspace_id == pinned) {
            return;
        }
        sink.log(&format!("the pinned workspace {pinned} is gone; forgetting the pin"));
        sink.mark(&format!("unpin workspace {pinned} (not listed)"));
        self.pinned_workspace = None;
        self.pins_changed = true;
        self.note_pin_notice(PinNotice::WorkspaceGone);
    }

    /// Start discovery, as soon as the channel can carry it.
    ///
    /// Called when the handshake completes rather than at construction: the host
    /// refuses session calls on a channel that has not said `hello`, and a request
    /// sent too early is a failure the panel would then have to retry through.
    pub fn begin(&mut self) {
        if self.started {
            return;
        }
        self.started = true;
        self.next_poll_at = 0;
    }

    /// Whether discovery has started.
    #[must_use]
    pub fn is_started(&self) -> bool {
        self.started
    }

    /// The conversation being followed, if any.
    #[must_use]
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// A human-readable name for the followed conversation.
    #[must_use]
    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    /// The subscription generation the host reported.
    #[must_use]
    pub fn generation(&self) -> Option<i64> {
        self.generation
    }

    /// How many persistent events have arrived for it.
    #[must_use]
    pub fn events(&self) -> u64 {
        self.events
    }

    /// How many temporary stream frames have arrived for it.
    #[must_use]
    pub fn streams(&self) -> u64 {
        self.streams
    }

    /// How many times the host reported a gap.
    #[must_use]
    pub fn resyncs(&self) -> u64 {
        self.resyncs
    }

    /// How many frames were discarded as belonging to a replaced subscription.
    #[must_use]
    pub fn stale(&self) -> u64 {
        self.stale
    }

    /// The next request to send, if one is due.
    ///
    /// A wanted workspace list wins the slot over the discovery poll: the discovery poll is a
    /// standing question, while the workspace list is what a panel that was just summoned has to
    /// answer to say where a new conversation would start. The request is sent from here rather
    /// than from the summon itself because this layer runs one request at a time, and the summon
    /// is already busy writing the attach that returns to a pinned conversation.
    ///
    /// @param now - caller-local time in milliseconds.
    /// @returns the request, or `None` when nothing is due or one is outstanding.
    pub fn next(&mut self, now: i64) -> Option<Outgoing> {
        if !self.started || self.in_flight.is_some() || now < self.next_poll_at {
            return None;
        }
        if self.workspaces_wanted && !self.workspaces_asked {
            self.workspaces_wanted = false;
            self.workspaces_asked = true;
            self.in_flight = Some(InFlight::Workspaces { sent_at: now });
            return Some(Outgoing::ListWorkspaces);
        }
        self.in_flight = Some(InFlight::List { sent_at: now });
        Some(Outgoing::ListSessions)
    }

    /// The conversations the host last listed, newest first.
    #[must_use]
    pub fn sessions(&self) -> &[SessionSummary] {
        &self.sessions
    }

    /// The workspaces the host last listed.
    #[must_use]
    pub fn workspaces(&self) -> &[Workspace] {
        &self.workspaces
    }

    /// The conversation the user pinned, if any.
    #[must_use]
    pub fn pinned(&self) -> Option<&str> {
        self.pinned.as_deref()
    }

    /// The conversation the user picked for this run, if any.
    #[must_use]
    pub fn chosen(&self) -> Option<&str> {
        self.choice.as_deref()
    }

    /// The conversation the panel should be attached to, whatever the list says.
    ///
    /// A pin outranks a choice, and a choice outranks "the newest".
    #[must_use]
    pub fn target(&self) -> Option<&str> {
        // The choice first, then the pin. The pin is only ever *adopted into* the choice, at
        // startup — and it clears the choice when it is (re)set, because a pin is the latest
        // decision. The other order is a panel that fights its user: picking another conversation
        // in the picker would last until the next poll, three seconds later, when the pin would
        // take the panel back. The pin means "where to open next time"; the choice means "where I
        // am now", and while the panel is open the second one wins.
        self.choice.as_deref().or(self.pinned.as_deref())
    }

    /// Open one conversation, now, and keep it until the panel is reopened.
    ///
    /// @param session_id - which one.
    /// @param now - caller-local time in milliseconds.
    /// @returns the request to send.
    pub fn choose(&mut self, session_id: &str, now: i64) -> Option<Outgoing> {
        self.choice = Some(session_id.to_owned());
        self.attach_now(session_id, now)
    }

    /// Keep opening one conversation, across restarts.
    ///
    /// @param session_id - which one, or `None` to stop pinning anything.
    /// @param now - caller-local time in milliseconds.
    /// @returns the request to send, when pinning means switching to it.
    pub fn set_pinned(&mut self, session_id: Option<String>, now: i64) -> Option<Outgoing> {
        // Only a *change* is worth writing down. A summon re-attaches the pinned conversation
        // through this method, and a pin that already says what it said before is not news.
        if self.pinned != session_id {
            self.pins_changed = true;
        }
        self.pinned = session_id;
        // A new decision is a fresh count: whatever the list said about the old pin is no
        // longer evidence about this one.
        self.absent_revisions = 0;
        self.pin_notice = None;
        if self.pinned.is_some() {
            // A pin is the user's *latest* decision about where the panel belongs, so it
            // supersedes the run-time choice: left in place, the choice would win at the next
            // poll (see `target`) and pull the panel back off the just-pinned conversation —
            // both for a picker pin and for the summon that returns to the pin.
            self.choice = None;
        }
        // Pinning what is already on screen changes nothing about the subscription.
        match self.pinned.clone() {
            Some(pinned) if self.session_id.as_deref() != Some(pinned.as_str()) => self.attach_now(&pinned, now),
            _ => None,
        }
    }

    /// Create a conversation, and be in it.
    ///
    /// @param workspace_id - where to create it, or `None` for the host's default.
    /// @param now - caller-local time in milliseconds.
    /// @returns the request to send.
    pub fn create(&mut self, workspace_id: Option<&str>, now: i64) -> Option<Outgoing> {
        if self.in_flight.is_some() {
            // One request at a time is the discipline this whole layer runs on; the user can
            // press the button again when the first answer is in.
            return None;
        }
        self.choosing_workspace = workspace_id.map(str::to_owned);
        self.in_flight = Some(InFlight::Create { sent_at: now });
        Some(Outgoing::CreateSession { workspace_id: workspace_id.map(str::to_owned) })
    }

    /// Take a pin that was read from disk, without asking the host for anything.
    ///
    /// The pin decides which conversation the *next* discovery answer attaches, so it has to
    /// be in place before that answer arrives — which is why this is separate from
    /// [`Follow::set_pinned`], whose whole job is to switch to the pinned one right now.
    ///
    /// @param session_id - the pinned conversation, if one was remembered.
    /// @param workspace_id - the pinned workspace, if one was remembered. It is a separate
    ///   fact, read from the same file but never derived from the conversation: either half
    ///   can be remembered without the other.
    pub fn adopt_pinned(&mut self, session_id: Option<String>, workspace_id: Option<String>) {
        self.pinned = session_id.clone();
        // Opened *into* the choice, which is what makes the pin a starting point rather than a
        // standing order (see `target`).
        self.choice = session_id;
        self.pinned_workspace = workspace_id;
        self.absent_revisions = 0;
    }

    /// Go back to "new conversation" mode: nothing pinned, nothing chosen, nothing attached.
    ///
    /// This is what "start a new conversation" means under the design — the conversation
    /// itself is created when the user submits, so this only clears the decision that was
    /// keeping the panel on an existing one. Clearing the pin is a *user gesture* here (the
    /// panel's own "new conversation" action), which is why it is marked for the shell to
    /// write down: a pin the user let go of must not come back on the next run.
    pub fn start_new(&mut self) {
        self.choice = None;
        if self.pinned.is_some() {
            self.pinned = None;
            self.pins_changed = true;
        }
        self.absent_revisions = 0;
        self.forget_attachment();
    }

    /// Forget a choice, so the newest conversation is followed again.
    ///
    /// Used when a pinned conversation is unpinned: going back to "newest" is the only
    /// sensible reading of that.
    pub fn clear_choice(&mut self) {
        self.choice = None;
    }

    /// Subscribe to one conversation, as a deliberate act.
    fn attach_now(&mut self, session_id: &str, now: i64) -> Option<Outgoing> {
        let label = self
            .sessions
            .iter()
            .find(|summary| summary.session_id == session_id)
            .and_then(|summary| summary.label.clone());
        self.in_flight = Some(InFlight::Attach {
            session_id: session_id.to_owned(),
            label,
            sent_at: now,
        });
        Some(Outgoing::Attach { session_id: session_id.to_owned() })
    }

    /// Re-subscribe to the conversation already being followed.
    ///
    /// For recovery, not for discovery: when the host reports that the event stream had
    /// a gap, what is on screen is known-incomplete, and the fix is a fresh subscription
    /// to *this* conversation. Polling for the newest one instead would silently switch
    /// the panel to a different conversation while the user was reading this one.
    ///
    /// The conversation is passed in rather than read from `self` because the transcript
    /// can know it before discovery has recorded it — a snapshot only arrives for an
    /// attached subscription, so the two agree in practice, but a gap is no less real
    /// when the bookkeeping lags behind it.
    ///
    /// @param session_id - the conversation to re-subscribe to.
    /// @param now - caller-local time in milliseconds.
    /// @returns the request to send.
    pub fn resubscribe(&mut self, session_id: &str, now: i64) -> Option<Outgoing> {
        // Following this conversation from now on, so the status line and the generation
        // checks describe the subscription that actually exists.
        if self.session_id.as_deref() != Some(session_id) {
            self.session_id = Some(session_id.to_owned());
            self.generation = None;
        }
        self.in_flight = Some(InFlight::Attach {
            session_id: session_id.to_owned(),
            label: self.label.clone(),
            sent_at: now,
        });
        Some(Outgoing::Attach { session_id: session_id.to_owned() })
    }

    /// Give up on an outstanding request, so the next pass can retry.
    ///
    /// Used both when the answer never came and when the frame could not be written.
    ///
    /// @param now - caller-local time in milliseconds.
    pub fn abandon(&mut self, now: i64) {
        self.in_flight = None;
        self.next_poll_at = now + POLL_INTERVAL_MS;
    }

    /// Whether an answer is still awaited, and how long it has been.
    ///
    /// @param now - caller-local time in milliseconds.
    /// @returns whether the outstanding request should be treated as lost.
    #[must_use]
    pub fn expired(&self, now: i64) -> bool {
        self.in_flight
            .as_ref()
            .is_some_and(|pending| now - pending.sent_at() > REQUEST_TIMEOUT_MS)
    }

    /// Read the answer to the outstanding request.
    ///
    /// @param outcome - the response, or the error that came back instead.
    /// @param now - caller-local time in milliseconds.
    /// @param sink - where log lines go.
    /// @returns the follow-up request, when the answer calls for one.
    pub fn resolve(
        &mut self,
        outcome: Result<Value, crate::ipc::rpc::RpcError>,
        now: i64,
        sink: &mut dyn FrameSink,
    ) -> Option<Outgoing> {
        let Some(pending) = self.in_flight.take() else {
            return None;
        };
        self.next_poll_at = now + POLL_INTERVAL_MS;
        match (pending, outcome) {
            (InFlight::List { .. }, Ok(result)) => {
                // The whole list is kept, not just its newest entry: this is the same answer
                // the picker shows, and asking twice for one fact is how two views of it
                // start to disagree.
                self.sessions = parse_sessions(&result);
                // The count and how many carry a name, because "the list shows session ids" is a
                // question about what the host sent, and this is the record that answers it.
                sink.mark(&format!(
                    "sessions {} named {}",
                    self.sessions.len(),
                    self.sessions.iter().filter(|entry| entry.title.is_some()).count(),
                ));
                // A pinned conversation the host no longer lists is a pin that cannot be
                // honoured. Keeping it would leave the panel attached to something that is not
                // there — and looking, from the outside, exactly like a panel with nothing to
                // say. Forgetting it puts the panel back into "new conversation" mode, which
                // is what the design asks for — but only once the absence is evidence, which
                // two consecutive revisions are and one is not (see `reconcile_session_pin`).
                self.reconcile_session_pin(sink);
                // Who to attach to is a decision the user made — pinned, or chosen for this
                // run. The newest conversation is *not* a decision: following it is how a panel
                // ends up talking about whatever the harness happened to touch last.
                match self.target().map(str::to_owned) {
                    Some(target) if self.session_id.as_deref() == Some(target.as_str()) => None,
                    Some(target) => {
                        let label = self
                            .sessions
                            .iter()
                            .find(|summary| summary.session_id == target)
                            .and_then(|summary| summary.label.clone());
                        self.in_flight = Some(InFlight::Attach {
                            session_id: target.clone(),
                            label,
                            sent_at: now,
                        });
                        Some(Outgoing::Attach { session_id: target })
                    }
                    None => {
                        if self.sessions.is_empty() && !self.said_empty {
                            // Once: the harness may have no conversations at all, which is
                            // normal on a fresh install, and this is what explains a panel
                            // that is waiting to be given something to say.
                            self.said_empty = true;
                            sink.log("no conversations yet; sending will start one");
                        }
                        None
                    }
                }
            }
            (InFlight::Workspaces { .. }, Ok(result)) => {
                self.workspaces = workspaces(&result);
                // The answer is now a fact. Before it, an empty list meant "not asked yet".
                self.workspaces_loaded = true;
                // A mark, not a log: the host swallows the peer's stderr, so the marker is the
                // only place this is visible after the fact — and "which workspace was chosen,
                // and why" is a question about exactly this list.
                sink.mark(&format!("workspaces {}", self.workspaces.len()));
                // A pinned workspace the host no longer lists cannot be created in: the list is
                // the authority the pin was validated against, so the pin goes — and only that
                // half of it.
                self.reconcile_workspace_pin(sink);
                // An empty list is not an answer the user can act on. The host can create its
                // first-use workspace itself, and when it will not, the panel says so rather
                // than showing a picker with nothing in it.
                self.ask_for_default_workspace(now)
            }
            (InFlight::Workspaces { .. }, Err(error)) => {
                // Asked for once. A host that cannot answer leaves the picker without
                // workspaces, which is a smaller problem than asking every three seconds.
                // The list stays *unloaded*: a request that failed is not a list that is empty.
                sink.mark(&format!("workspaces failed: {}", error.message));
                None
            }
            (InFlight::DefaultWorkspace { .. }, Ok(result)) => {
                let defaults = workspaces(&result);
                if defaults.is_empty() {
                    // The host answered and there is still nothing to create in. Ineligible
                    // first-use initialization (a host with sessions but no workspaces) looks
                    // exactly like this, and both mean the user has to make one in the Harness.
                    self.default_workspace = DefaultWorkspaceState::Failed;
                    sink.mark("workspace/default: none");
                    return None;
                }
                // The returned rows are authoritative registry projections, so they are merged
                // like any other list answer — the panel does not have to ask again to see the
                // workspace it is now going to use.
                for workspace in defaults {
                    match self.workspaces.iter_mut().find(|known| known.workspace_id == workspace.workspace_id) {
                        Some(known) => *known = workspace,
                        None => self.workspaces.push(workspace),
                    }
                }
                self.default_workspace = DefaultWorkspaceState::Done;
                sink.mark(&format!("workspace/default: {}", self.workspaces.len()));
                None
            }
            (InFlight::DefaultWorkspace { .. }, Err(error)) => {
                self.default_workspace = DefaultWorkspaceState::Failed;
                sink.mark(&format!("workspace/default failed: {}", error.message));
                None
            }
            (InFlight::Create { .. }, Ok(result)) => {
                let Some(session_id) = result.get("sessionId").and_then(Value::as_str) else {
                    sink.log("session/create answered without a session id");
                    return None;
                };
                // Creating a conversation means being in it: the newest-conversation rule
                // would do the same, but only after the next poll, and the user is looking
                // at the panel *now*.
                self.choice = Some(session_id.to_owned());
                sink.log(&format!("created conversation {session_id}"));
                sink.mark(&format!("create {session_id}"));
                self.in_flight = Some(InFlight::Attach {
                    session_id: session_id.to_owned(),
                    label: None,
                    sent_at: now,
                });
                Some(Outgoing::Attach { session_id: session_id.to_owned() })
            }
            (InFlight::Create { .. }, Err(error)) => {
                sink.log(&format!("could not create a conversation: {}", error.message));
                // Recorded rather than only logged: a prompt is waiting behind this creation,
                // and its sender is the only one who can tell the user why nothing was sent.
                self.create_failure = Some(error.message.clone());
                None
            }
            (InFlight::Attach { session_id, label, .. }, Ok(result)) => {
                self.session_id = Some(session_id.clone());
                self.label = label;
                self.generation = result.get("generation").and_then(Value::as_i64);
                self.last_seq = None;
                sink.log(&format!(
                    "following conversation {session_id} (generation {})",
                    self.generation.map_or_else(|| "unknown".to_owned(), |value| value.to_string()),
                ));
                // The durable record of the one fact that decides whether an approval
                // can reach this panel at all. Without it, "the card did not appear"
                // has no explanation that survives the run.
                sink.mark(&format!(
                    "follow {session_id} generation={}",
                    self.generation.map_or_else(|| "unknown".to_owned(), |value| value.to_string()),
                ));
                None
            }
            (InFlight::Attach { session_id, .. }, Err(error)) => {
                sink.log(&format!(
                    "could not follow {session_id}: [{code}] {message}",
                    code = error.code,
                    message = error.message,
                ));
                // The pin goes only when the refusal is *definitive*: `stale` is the host saying
                // the identity itself is no longer valid, which is the one answer that cannot be
                // better on a retry. A transient refusal — an unavailable service, a timeout, a
                // closed channel — is a statement about the moment, and a panel that throws a
                // pin away for it is a panel that blames the user for a restart. At most one
                // such loss is recorded per visit: after the first, the pin is already gone.
                if self.pinned.as_deref() == Some(session_id.as_str()) {
                    if error.code == error_code::STALE && !self.refused_this_visit {
                        self.refused_this_visit = true;
                        sink.mark(&format!("unpin {session_id} (refused: stale)"));
                        self.pinned = None;
                        self.pins_changed = true;
                        self.note_pin_notice(PinNotice::SessionGone);
                    } else if error.code != error_code::STALE {
                        sink.mark(&format!("kept pin {session_id} (transient refusal {})", error.code));
                    }
                }
                // The run-time choice is not a pin: it is where this visit happened to be, and
                // a conversation the host will not attach to is not a place to stay. Dropping it
                // puts the panel back to waiting, which is what "start a new one" means.
                if self.choice.as_deref() == Some(session_id.as_str()) {
                    self.choice = None;
                }
                None
            }
            (InFlight::List { .. }, Err(error)) => {
                sink.log(&format!(
                    "could not list conversations: [{code}] {message}",
                    code = error.code,
                    message = error.message,
                ));
                None
            }
        }
    }

    /// Apply one pushed workspace change.
    ///
    /// The host follows the Harness's own workspace stream and forwards every frame, so the list
    /// the picker draws stops being one snapshot taken on demand. The frames land in the same
    /// `workspaces` vector a `workspaces/list` answer fills, which is what makes the snapshot the
    /// frontend already re-renders from carry the change — no second list, and no request.
    ///
    /// `archived` and `pinned` are deliberately not applied: they are the Harness's *session* sets,
    /// and the panel's conversation list is not built from them yet. The `pinned` frame in
    /// particular is the Harness's session pin, not this panel's own workspace pin, and treating
    /// one as the other would silently move where a new conversation is created.
    ///
    /// @param frame - one `workspaces/changed` payload.
    /// @param sink - where the marker for an applied change goes.
    pub fn apply_workspace_frame(&mut self, frame: &Value, sink: &mut dyn FrameSink) {
        match frame.get("type").and_then(Value::as_str) {
            Some("baseline") => {
                let Some(value) = frame.get("value") else {
                    sink.log("workspaces/changed baseline carried no value");
                    return;
                };
                // A baseline is a complete list, so it replaces whatever was held — including a
                // list this process never asked for. `workspaces_asked` deliberately stays as it
                // was: it records that *this* process sent a request, and both the picker's
                // "list pending" state and the first-use default-workspace ladder key off an
                // answer to that request rather than off the list's arrival.
                self.workspaces = workspaces(value);
                self.workspaces_loaded = true;
                sink.mark(&format!("workspaces changed: baseline {}", self.workspaces.len()));
                // The list is the authority the workspace pin was validated against, and a
                // baseline is the list.
                self.reconcile_workspace_pin(sink);
            }
            Some("upsert") => {
                let Some(workspace) = frame.get("workspace").and_then(read_workspace) else {
                    sink.log("workspaces/changed upsert carried no usable workspace");
                    return;
                };
                let workspace_id = workspace.workspace_id.clone();
                // Merge by identity, in place when it is already known: the frame is the whole
                // workspace, so replacing the row is the update, and appending a new one keeps it
                // until an `order` frame says otherwise.
                match self.workspaces.iter_mut().find(|known| known.workspace_id == workspace_id) {
                    Some(known) => *known = workspace,
                    None => self.workspaces.push(workspace),
                }
                sink.mark(&format!("workspaces changed: upsert {workspace_id}"));
            }
            Some("remove") => {
                let Some(workspace_id) = frame.get("workspaceId").and_then(Value::as_str) else {
                    sink.log("workspaces/changed remove carried no workspace id");
                    return;
                };
                self.workspaces.retain(|workspace| workspace.workspace_id != workspace_id);
                sink.mark(&format!("workspaces changed: removed {workspace_id}"));
                // A removal is the list saying the pin's workspace is gone, exactly as a list
                // answer would, so the same reconciliation applies.
                self.reconcile_workspace_pin(sink);
            }
            Some("order") => {
                let Some(ids) = frame.get("workspaceIds").and_then(Value::as_array) else {
                    sink.log("workspaces/changed order carried no workspace ids");
                    return;
                };
                let mut known = std::mem::take(&mut self.workspaces);
                let mut reordered = Vec::with_capacity(known.len());
                for id in ids.iter().filter_map(Value::as_str) {
                    if let Some(index) = known.iter().position(|workspace| workspace.workspace_id == id) {
                        reordered.push(known.remove(index));
                    }
                }
                // Anything the frame did not name is kept at the end rather than dropped: an
                // `order` frame is an ordering, not a census, and a list that lost a workspace
                // to it would be one the user cannot explain.
                reordered.append(&mut known);
                self.workspaces = reordered;
                sink.mark(&format!("workspaces changed: order {}", self.workspaces.len()));
            }
            Some("archived") | Some("pinned") => {}
            other => sink.log(&format!(
                "ignoring a workspaces/changed frame this build does not use: {other:?}",
            )),
        }
    }

    /// Record one frame of session traffic.
    ///
    /// Everything here is counted rather than stored — but *counted*, not dropped on the
    /// floor: the status line is how a user tells "following and receiving" from "following
    /// and getting nothing", and those look the same from every other angle.
    ///
    /// Nothing is logged per frame: a turn produces hundreds, and each line would
    /// cost stderr that the host forwards and truncates, drowning the lines that
    /// matter.
    ///
    /// @param method - the notification method.
    /// @param params - its payload.
    /// @param sink - where the rare log line goes.
    /// @returns whether the frame belonged to the followed conversation.
    pub fn record(&mut self, method: &str, params: Option<&Value>, sink: &mut dyn FrameSink) -> bool {
        let Some(params) = params else { return false };
        let belongs = params
            .get("sessionId")
            .and_then(Value::as_str)
            .zip(self.session_id.as_deref())
            .is_some_and(|(frame, followed)| frame == followed);
        if !belongs {
            return false;
        }
        // A frame from a subscription the host has already replaced describes a
        // conversation state that nobody is following any more. Counting it would
        // make the status line lie.
        if let Some(generation) = params.get("generation").and_then(Value::as_i64) {
            if self.generation.is_some_and(|current| current != generation) {
                self.stale += 1;
                return true;
            }
        }
        match method {
            "session/snapshot" => {
                let records = params.get("records").and_then(Value::as_array).map_or(0, Vec::len);
                self.last_seq = params.get("cursor").and_then(Value::as_i64);
                sink.log(&format!(
                    "the conversation opened at cursor {} with {records} record(s)",
                    self.last_seq.map_or_else(|| "unknown".to_owned(), |value| value.to_string()),
                ));
            }
            "session/event" => {
                self.events += 1;
                let seq = params.get("seq").and_then(Value::as_i64);
                if let (Some(seq), Some(last)) = (seq, self.last_seq) {
                    if seq != last + 1 && self.gaps_are_new(seq) {
                        // A hole in the persistent stream is the host's business to
                        // repair with `session/resync`; noting it here is what makes
                        // the repair visible if it never comes.
                        sink.log(&format!("the event stream jumped from {last} to {seq}"));
                    }
                }
                if let Some(seq) = seq {
                    self.last_seq = Some(seq);
                }
            }
            "session/stream" => self.streams += 1,
            "session/resync" => {
                self.resyncs += 1;
                sink.log("the host reported a gap in the event stream");
            }
            _ => return false,
        }
        true
    }

    /// Whether a jump to this sequence is worth a log line.
    ///
    /// Only the first jump after a clean run is reported: a stream that has already
    /// been declared broken would otherwise produce one line per event until the host
    /// repaired it.
    ///
    /// @param seq - the sequence that arrived.
    /// @returns whether this is a new discontinuity.
    fn gaps_are_new(&mut self, seq: i64) -> bool {
        let is_new = self.gap_reported_at != Some(seq);
        self.gap_reported_at = Some(seq);
        is_new
    }
}

/// One conversation as `sessions/list` describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSummary {
    /// Durable session identity.
    pub session_id: String,
    /// When the Harness last touched it, in epoch milliseconds.
    pub updated_at: i64,
    /// What the Harness calls this conversation, when it has a name for it.
    ///
    /// The list is what the picker draws, and a row of session ids is a list nobody can read; the
    /// host is the only side that knows the titles of conversations this panel has never attached
    /// to.
    pub title: Option<String>,
    /// The workspace directory's name, when the host reported one.
    pub label: Option<String>,
    /// That directory in full, which is how a conversation is matched to the workspace it
    /// belongs to: the session list carries no workspace id, and the paths are what both
    /// lists agree on.
    pub cwd: Option<String>,
    /// Whether a turn is in flight there right now.
    pub running: bool,
    /// Whether it has no history yet — a conversation that was created and not used.
    pub blank: bool,
}

/// Read every conversation out of a `sessions/list` result, newest first.
///
/// Ordered by `updatedAt` rather than by the array's order, which the protocol does
/// not promise to be meaningful. Entries without a usable id are skipped: attaching
/// to one would be a request the host must refuse, and offering it in the picker would
/// be offering something that cannot work.
///
/// @param result - the response value.
/// @returns the conversations, newest first.
pub(crate) fn parse_sessions(result: &Value) -> Vec<SessionSummary> {
    summaries(result)
}

fn summaries(result: &Value) -> Vec<SessionSummary> {
    let Some(items) = result.get("items").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut found: Vec<SessionSummary> = items
        .iter()
        .filter_map(|item| {
            let session_id = item.get("sessionId").and_then(Value::as_str)?;
            if session_id.is_empty() {
                return None;
            }
            Some(SessionSummary {
                session_id: session_id.to_owned(),
                updated_at: item.get("updatedAt").and_then(Value::as_i64).unwrap_or(0),
                title: item
                    .get("title")
                    .and_then(Value::as_str)
                    .filter(|title| !title.trim().is_empty())
                    .map(str::to_owned),
                cwd: item.get("cwd").and_then(Value::as_str).map(str::to_owned),
                label: item
                    .get("cwd")
                    .and_then(Value::as_str)
                    .and_then(|cwd| std::path::Path::new(cwd).file_name().map(|name| name.to_string_lossy().into_owned())),
                running: item.get("running").and_then(Value::as_bool).unwrap_or(false),
                blank: item.get("blank").and_then(Value::as_bool).unwrap_or(false),
            })
        })
        .collect();
    found.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    found
}

/// Read every workspace out of a `workspaces/list` (or `workspace/default`) result.
///
/// The item shape is the host's `WorkspaceView`, and the two halves this layer needs beside the
/// name and the directory are the membership account (`sessionIds`) and the registration time
/// (`createdAt`, epoch milliseconds). Both are read defensively: a host that does not send them
/// leaves a workspace with no members and no time, which is a workspace that simply never wins
/// "most recent" — not a workspace that disappears.
///
/// @param result - the response value.
/// @returns the workspaces, in the order the host listed them.
fn workspaces(result: &Value) -> Vec<Workspace> {
    let Some(items) = result.get("items").and_then(Value::as_array) else {
        return Vec::new();
    };
    items.iter().filter_map(read_workspace).collect()
}

/// Read one workspace out of a single `WorkspaceView` object.
///
/// The same reading `workspaces` applies to a list's rows, extracted because the pushed
/// `upsert` frame carries one workspace rather than a list: two readers of the same shape would
/// drift, and the field that would drift first is `createdAt` — the registry's ISO string becomes
/// epoch milliseconds on the host side, and a reader that stopped doing that conversion would
/// quietly score every workspace as "registered at the epoch" in the new-conversation ladder.
///
/// @param item - one workspace object.
/// @returns the workspace, or `None` when it has no usable identity.
fn read_workspace(item: &Value) -> Option<Workspace> {
    let workspace_id = item.get("workspaceId").and_then(Value::as_str)?;
    if workspace_id.is_empty() {
        return None;
    }
    Some(Workspace {
        workspace_id: workspace_id.to_owned(),
        title: item.get("title").and_then(Value::as_str).unwrap_or_default().to_owned(),
        path: item.get("path").and_then(Value::as_str).unwrap_or_default().to_owned(),
        session_ids: item
            .get("sessionIds")
            .and_then(Value::as_array)
            .map(|ids| {
                ids.iter()
                    .filter_map(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        created_at: item
            .get("createdAt")
            .and_then(Value::as_i64)
            .filter(|value| *value >= 0)
            .unwrap_or(0),
    })
}

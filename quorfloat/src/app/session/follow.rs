//! Following a conversation.
//!
//! The panel answers approvals for the conversation it is *showing*, and the host
//! decides that by `session/attach` (see `docs/dsh-quorfloat.md` §6): an interaction is
//! claimed only for an attached session. With nothing attached the panel owns
//! nothing, so every approval is answered by the Harness window — and from the
//! outside that is indistinguishable from a panel that is broken.
//!
//! **Which** conversation to follow is, for now, the simplest rule that makes any of
//! this observable: the most recently updated one. The panel has no conversation UI
//! yet, so there is nothing a user could pick with; when there is, this rule becomes
//! a default rather than the whole policy.
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
/// panel with nothing to do.
const REQUEST_TIMEOUT_MS: i64 = 10_000;

/// What the follow layer wants to ask the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outgoing {
    /// Ask which conversations exist.
    ListSessions,
    /// Ask which workspaces exist.
    ListWorkspaces,
    /// Create a conversation, in a chosen workspace.
    CreateSession {
        /// The workspace to create it in, or `None` for the host's own default.
        workspace_id: Option<String>,
    },
    /// Subscribe to one of them.
    Attach {
        /// Durable session identity.
        session_id: String,
    },
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
            Self::List { sent_at } | Self::Workspaces { sent_at } | Self::Create { sent_at } => *sent_at,
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
    /// Whether the workspace list has been asked for yet.
    workspaces_asked: bool,
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

    /// Set the subscription generation, for tests whose subject is what happens at a known
    /// generation rather than how it was learned.
    #[cfg(test)]
    fn set_generation_for_tests(&mut self, generation: i64) {
        self.generation = Some(generation);
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
    /// @param now - caller-local time in milliseconds.
    /// @returns the request, or `None` when nothing is due or one is outstanding.
    pub fn next(&mut self, now: i64) -> Option<Outgoing> {
        if !self.started || self.in_flight.is_some() || now < self.next_poll_at {
            return None;
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
        self.pinned = session_id;
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
    pub fn adopt_pinned(&mut self, session_id: Option<String>) {
        self.pinned = session_id.clone();
        // Opened *into* the choice, which is what makes the pin a starting point rather than a
        // standing order (see `target`).
        self.choice = session_id;
    }

    /// Go back to "new conversation" mode: nothing pinned, nothing chosen, nothing attached.
    ///
    /// This is what "start a new conversation" means under the design — the conversation
    /// itself is created when the user submits, so this only clears the decision that was
    /// keeping the panel on an existing one.
    pub fn start_new(&mut self) {
        self.choice = None;
        self.pinned = None;
        self.forget_attachment();
    }

    /// Forget a choice, so the newest conversation is followed again.
    ///
    /// Used when a pinned conversation is unpinned: going back to "newest" is the only
    /// sensible reading of that, and it is what the panel did before pins existed.
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
                // is what the design asks for.
                if let Some(pinned) = self.pinned.clone() {
                    if !self.sessions.iter().any(|summary| summary.session_id == pinned) {
                        sink.log(&format!("the pinned conversation {pinned} is gone; forgetting the pin"));
                        sink.mark(&format!("unpin {pinned} (no longer listed)"));
                        self.pinned = None;
                        if self.session_id.as_deref() == Some(pinned.as_str()) {
                            self.forget_attachment();
                        }
                    }
                }
                // Who to attach to is a decision the user made — pinned, or chosen for this
                // run. The newest conversation is *not* a decision: following it was what this
                // layer did before the panel could be told what to show, and it is how a panel
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
                // A mark, not a log: the host swallows the peer's stderr, so the marker is the
                // only place this is visible after the fact — and "which workspace was chosen,
                // and why" is a question about exactly this list.
                sink.mark(&format!("workspaces {}", self.workspaces.len()));
                None
            }
            (InFlight::Workspaces { .. }, Err(error)) => {
                // Asked for once. A host that cannot answer leaves the picker without
                // workspaces, which is a smaller problem than asking every three seconds.
                sink.mark(&format!("workspaces failed: {}", error.message));
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
                // And the decision goes with it. Keeping a pin or a choice that the host has
                // just refused would retry it every few seconds for the rest of the run, and
                // the panel would sit there looking attached to nothing. The design's answer
                // to "the conversation is not available" is a new one, made on submit.
                if self.pinned.as_deref() == Some(session_id.as_str()) {
                    sink.mark(&format!("unpin {session_id} (refused)"));
                    self.pinned = None;
                }
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

    /// Record one frame of session traffic.
    ///
    /// The panel does not display conversations yet, so everything here is counted
    /// rather than stored — but it is *counted*, not dropped on the floor: the status
    /// line is how a user tells "following and receiving" from "following and getting
    /// nothing", and those look the same from every other angle.
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

/// Read every workspace out of a `workspaces/list` result.
///
/// @param result - the response value.
/// @returns the workspaces, in the order the host listed them.
fn workspaces(result: &Value) -> Vec<Workspace> {
    let Some(items) = result.get("items").and_then(Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let workspace_id = item.get("workspaceId").and_then(Value::as_str)?;
            if workspace_id.is_empty() {
                return None;
            }
            Some(Workspace {
                workspace_id: workspace_id.to_owned(),
                title: item.get("title").and_then(Value::as_str).unwrap_or_default().to_owned(),
                path: item.get("path").and_then(Value::as_str).unwrap_or_default().to_owned(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A sink that keeps what was said instead of writing it.
    #[derive(Default)]
    struct Recorded {
        logs: Vec<String>,
        marks: Vec<String>,
    }

    impl FrameSink for Recorded {
        fn send(&mut self, _frame: &Value) -> std::io::Result<()> {
            Ok(())
        }

        fn log(&mut self, line: &str) {
            self.logs.push(line.to_owned());
        }

        fn mark(&mut self, line: &str) {
            self.marks.push(line.to_owned());
        }
    }

    impl Recorded {
        fn joined(&self) -> String {
            self.logs.join("\n")
        }
    }

    /// A follow layer that has already started, as it is after the handshake.
    /// Attach to one conversation, the way the design says a panel gets attached: the user
    /// pinned it or chose it. Following the newest one is gone, so every test that needs an
    /// attachment has to make that decision — which is the point of the change.
    ///
    /// @param follow - the layer.
    /// @param sink - where the breadcrumbs go.
    /// @param id - the conversation to attach to.
    /// @param now - caller-local time.
    fn attach(follow: &mut Follow, sink: &mut Recorded, id: &str, now: i64) {
        assert_eq!(
            follow.choose(id, now),
            Some(Outgoing::Attach { session_id: id.to_owned() }),
            "choosing a conversation asks to attach to it",
        );
        let _ = follow.resolve(Ok(json!({"sessionId": id, "generation": 1})), now, sink);
        assert_eq!(follow.session_id(), Some(id), "and the attachment is recorded");
    }

    /// A follow layer that has completed its handshake.
    fn started() -> Follow {
        let mut follow = Follow::default();
        follow.begin();
        follow
    }

    /// A `sessions/list` answer for the given `(id, updatedAt)` pairs.
    fn sessions(entries: &[(&str, i64)]) -> Value {
        json!({
            "items": entries
                .iter()
                .map(|(id, updated)| json!({"sessionId": id, "updatedAt": updated, "cwd": "/work/project"}))
                .collect::<Vec<_>>(),
        })
    }

    /// The workspace list is asked for when the picker needs it, kept, and never asked for
    /// on top of a request that is already outstanding.
    #[test]
    fn the_workspace_list_is_asked_for_on_demand_and_then_cached() {
        let mut follow = started();
        let mut sink = Recorded::default();
        assert_eq!(follow.request_workspaces(0), Some(Outgoing::ListWorkspaces));
        assert!(follow.workspaces_asked());
        // While that is in flight, a second ask waits rather than colliding with it.
        assert_eq!(follow.request_workspaces(1), None, "one request at a time");

        let result = json!({"items": [
            {"workspaceId": "ws-1", "title": "Quorvox", "path": "/work/quorvox"},
            {"workspaceId": "ws-2", "title": "Notes", "path": "/work/notes"},
        ]});
        assert_eq!(follow.resolve(Ok(result), 10, &mut sink), None, "listing asks for nothing further");
        assert_eq!(follow.workspaces().len(), 2);
        assert_eq!(follow.workspaces()[0].title, "Quorvox");
        assert_eq!(sink.marks, vec!["workspaces 2".to_owned()], "the count is on the record");
    }

    /// A failure to list workspaces is reported and leaves the picker empty; it does not
    /// stop conversations from being discovered.
    #[test]
    fn a_workspace_list_that_fails_is_reported_and_discovery_continues() {
        let mut follow = started();
        let mut sink = Recorded::default();
        assert_eq!(follow.request_workspaces(0), Some(Outgoing::ListWorkspaces));
        let error = crate::ipc::rpc::RpcError {
            code: -32_000,
            message: "no harness".to_owned(),
            data: None,
        };
        assert_eq!(follow.resolve(Err(error), 10, &mut sink), None);
        // On the record, not in the log: the host swallows the peer's stderr, so a failure that
        // only reached a log line would be invisible after the fact.
        assert!(
            sink.marks.iter().any(|mark| mark.starts_with("workspaces failed")),
            "{:?}",
            sink.marks,
        );
        assert!(sink.joined().contains("workspaces: 2") || follow.workspaces().is_empty());
        assert_eq!(follow.next(POLL_INTERVAL_MS * 2), Some(Outgoing::ListSessions), "discovery is unaffected");
    }

    /// The list is kept whole, because the picker draws it — not just its newest entry.
    #[test]
    fn the_whole_list_is_kept_for_the_picker_newest_first() {
        let mut follow = started();
        let mut sink = Recorded::default();
        assert_eq!(follow.next(0), Some(Outgoing::ListSessions));
        let result = json!({"items": [
            {"sessionId": "old", "updatedAt": 10, "cwd": "/work/project", "running": false, "blank": false},
            {"sessionId": "busy", "updatedAt": 30, "cwd": "/work/other", "running": true, "blank": false},
            {"sessionId": "new-blank", "updatedAt": 20, "cwd": "/work/project", "running": false, "blank": true},
        ]});
        let _ = follow.resolve(Ok(result), 10, &mut sink);
        let listed: Vec<&str> = follow.sessions().iter().map(|entry| entry.session_id.as_str()).collect();
        assert_eq!(listed, vec!["busy", "new-blank", "old"], "newest first");
        assert!(follow.sessions()[0].running, "a turn in flight is visible in the list");
        assert!(follow.sessions()[1].blank, "and so is a conversation with no history");
        assert_eq!(follow.sessions()[1].label.as_deref(), Some("project"), "the workspace directory's name");
    }

    /// Choosing a conversation is a decision, and the newest-conversation rule has to stop
    /// at it — otherwise the panel would switch away from what the user just picked.
    #[test]
    fn a_chosen_conversation_is_not_replaced_by_a_newer_one() {
        let mut follow = started();
        let mut sink = Recorded::default();
        assert_eq!(follow.next(0), Some(Outgoing::ListSessions));
        let _ = follow.resolve(Ok(sessions(&[("first", 10)])), 10, &mut sink);
        // The user picks the older one.
        assert_eq!(follow.choose("second", 20), Some(Outgoing::Attach { session_id: "second".to_owned() }));
        assert_eq!(follow.target(), Some("second"));
        let _ = follow.resolve(Ok(json!({})), 20, &mut sink);

        // A newer conversation appears. A choice outranks it.
        assert_eq!(follow.next(POLL_INTERVAL_MS * 2), Some(Outgoing::ListSessions));
        let _ = follow.resolve(Ok(sessions(&[("newest", 99), ("second", 20)])), 30, &mut sink);
        assert_eq!(follow.session_id(), Some("second"), "the choice holds");
        assert_eq!(follow.next(POLL_INTERVAL_MS * 4), Some(Outgoing::ListSessions), "and discovery keeps refreshing the list");
    }

    /// The pin is where the panel opens, and the choice is where it goes next.
    ///
    /// The other order — pin always first — is a panel that fights its user: picking another
    /// conversation lasted until the next poll, three seconds later, when the pin took the
    /// panel back. Reported from using it, and this is the rule that came out of it.
    #[test]
    fn a_pin_decides_where_the_panel_opens_and_the_choice_decides_where_it_goes() {
        let mut follow = started();
        let mut sink = Recorded::default();

        // Opened with a pin: the pin is adopted as the choice, so the panel attaches to it.
        follow.adopt_pinned(Some("session-1".to_owned()));
        assert_eq!(follow.target(), Some("session-1"));

        // And it is still the preference for the next launch.
        assert_eq!(follow.pinned(), Some("session-1"));

        // The user picks another conversation: the panel goes there, and the pin does not
        // take it back on the next poll.
        follow.next(0);
        let _ = follow.resolve(Ok(sessions(&[("session-1", 10), ("session-2", 20)])), 1, &mut sink);
        follow.choose("session-2", 2);
        assert_eq!(follow.target(), Some("session-2"));
        assert_eq!(follow.pinned(), Some("session-1"), "the preference is untouched");
        follow.next(2 + POLL_INTERVAL_MS);
        let _ = follow.resolve(Ok(sessions(&[("session-1", 10), ("session-2", 20)])), 3 + POLL_INTERVAL_MS, &mut sink);
        assert_eq!(follow.target(), Some("session-2"), "and the poll leaves it alone");
    }

    /// Pinning is the latest decision: it supersedes the run-time choice, or the next poll
    /// would bounce the panel back off the just-pinned conversation.
    #[test]
    fn pinning_supersedes_the_run_time_choice() {
        let mut follow = started();
        follow.choose("session-2", 0);
        assert_eq!(follow.target(), Some("session-2"));
        follow.set_pinned(Some("session-1".to_owned()), 0);
        assert_eq!(follow.target(), Some("session-1"), "the pin is the latest decision");
        // The superseded choice is gone, not parked: unpinning now means nothing is
        // pinned and nothing chosen — the new-conversation state.
        follow.set_pinned(None, 0);
        assert_eq!(follow.target(), None, "nothing is pinned or chosen after the pin is off");
    }

    /// Unpinning *without* ever pinning leaves the run-time choice in place.
    #[test]
    fn unpinning_without_a_pin_keeps_the_run_time_choice() {
        let mut follow = started();
        follow.choose("session-2", 0);
        follow.set_pinned(None, 0);
        assert_eq!(follow.target(), Some("session-2"));
    }

    /// Creating a conversation means being in it, without waiting for the next poll.
    #[test]
    fn creating_a_conversation_attaches_it_immediately() {
        let mut follow = started();
        let mut sink = Recorded::default();
        assert_eq!(
            follow.create(Some("ws-1"), 10),
            Some(Outgoing::CreateSession { workspace_id: Some("ws-1".to_owned()) }),
        );
        assert_eq!(
            follow.resolve(Ok(json!({"sessionId": "brand-new"})), 20, &mut sink),
            Some(Outgoing::Attach { session_id: "brand-new".to_owned() }),
        );
        assert_eq!(follow.chosen(), Some("brand-new"));
        assert_eq!(sink.marks, vec!["create brand-new".to_owned()], "the durable record names it");
        // And the newest-conversation rule cannot take it away before the user has typed.
        assert_eq!(follow.target(), Some("brand-new"));
    }

    #[test]
    fn discovery_waits_for_the_handshake() {
        // Session calls are refused on a channel that has not said `hello`, so asking
        // early would be a failure the panel had to retry through.
        let mut follow = Follow::default();
        let mut sink = Recorded::default();
        assert!(!follow.is_started());
        assert_eq!(follow.next(0), None);
        follow.begin();
        assert!(follow.is_started());
        assert_eq!(follow.next(0), Some(Outgoing::ListSessions));
        // Starting twice is not two polls.
        follow.begin();
        assert_eq!(follow.next(POLL_INTERVAL_MS * 10), None, "a request is already in flight");
        let _ = &mut sink;
    }

    #[test]
    fn one_request_is_outstanding_at_a_time() {
        let mut follow = started();
        let mut sink = Recorded::default();
        assert_eq!(follow.next(0), Some(Outgoing::ListSessions));
        assert_eq!(follow.next(POLL_INTERVAL_MS * 10), None, "the first answer is still owed");
        let _ = follow.resolve(Ok(sessions(&[("a", 1)])), 10, &mut sink);
        // Answered, so the slot is free — but not before the poll interval has passed.
        assert_eq!(follow.next(11), None, "too soon after an answer");
        assert_eq!(follow.next(10 + POLL_INTERVAL_MS), Some(Outgoing::ListSessions));
    }

    #[test]
    fn nothing_is_attached_until_the_user_pins_or_chooses() {
        // The list arrives, several conversations are in it, and the panel attaches to none of
        // them: it is in "new conversation" mode, and the conversation it will use is created
        // when the user submits — not picked from whatever the harness touched last.
        let mut follow = started();
        let mut sink = Recorded::default();
        follow.next(0);
        let outgoing = follow.resolve(Ok(sessions(&[("old", 10), ("newest", 90), ("middle", 50)])), 5, &mut sink);
        assert_eq!(outgoing, None, "no attachment without a decision");
        assert_eq!(follow.session_id(), None);
        // The list itself is still kept: the picker draws it.
        assert_eq!(follow.sessions().len(), 3);
        assert_eq!(follow.sessions()[0].session_id, "newest", "and it is ordered by `updatedAt`");
    }

    #[test]
    fn an_attach_is_recorded_with_its_generation_and_a_breadcrumb() {
        let mut follow = started();
        let mut sink = Recorded::default();
        follow.next(0);
        let _ = follow.resolve(Ok(sessions(&[("session-1", 10)])), 5, &mut sink);
        // The choice carries the label the list gave, and the answer carries the generation.
        follow.choose("session-1", 6);
        follow.resolve(Ok(json!({"sessionId": "session-1", "generation": 4, "replaced": false})), 6, &mut sink);
        assert_eq!(follow.session_id(), Some("session-1"));
        assert_eq!(follow.generation(), Some(4));
        assert_eq!(follow.label(), Some("project"));
        assert!(sink.joined().contains("following conversation session-1"));
        // The one fact that decides whether an approval can reach the panel must survive the run.
        // The list's own mark comes first and belongs here: "which conversations did the host
        // offer" has to stay answerable afterwards, because the decision to attach was made against
        // exactly that list.
        assert_eq!(
            sink.marks,
            vec!["sessions 1 named 0".to_owned(), "follow session-1 generation=4".to_owned()],
        );
    }

    #[test]
    fn an_empty_list_is_not_an_error_and_is_retried_later() {
        // A fresh environment has no conversations at all. That is normal, must not be logged
        // every few seconds, and must not stop discovery.
        let mut follow = started();
        let mut sink = Recorded::default();
        follow.next(0);
        assert_eq!(follow.resolve(Ok(json!({"items": []})), 10, &mut sink), None);
        assert!(sink.joined().contains("no conversations yet"));
        assert_eq!(follow.session_id(), None);
        assert_eq!(follow.next(10 + POLL_INTERVAL_MS - 1), None, "too early");
        assert_eq!(follow.next(10 + POLL_INTERVAL_MS), Some(Outgoing::ListSessions));
        // Said once: repeating it every three seconds is noise.
        let _ = follow.resolve(Ok(json!({"items": []})), 10 + POLL_INTERVAL_MS + 1, &mut sink);
        assert_eq!(sink.joined().matches("no conversations yet").count(), 1);
    }

    #[test]
    fn the_same_conversation_is_not_re_attached() {
        // The steady state runs every few seconds; re-attaching would churn the subscription
        // and log a line each time.
        let mut follow = started();
        let mut sink = Recorded::default();
        follow.next(0);
        let _ = follow.resolve(Ok(sessions(&[("session-1", 10)])), 1, &mut sink);
        attach(&mut follow, &mut sink, "session-1", 2);
        let before = sink.logs.len();
        follow.next(2 + POLL_INTERVAL_MS);
        assert_eq!(
            follow.resolve(Ok(sessions(&[("session-1", 10)])), 3 + POLL_INTERVAL_MS, &mut sink),
            None,
        );
        assert_eq!(sink.logs.len(), before, "nothing new to say");
        assert_eq!(follow.session_id(), Some("session-1"));
    }

    #[test]
    fn a_newer_conversation_does_not_take_over() {
        // The rule this replaced was "follow the newest", and it is the reason a panel could
        // end up talking about whatever the harness touched last. An attachment now lasts
        // until the user says otherwise.
        let mut follow = started();
        let mut sink = Recorded::default();
        follow.next(0);
        let _ = follow.resolve(Ok(sessions(&[("session-1", 10)])), 1, &mut sink);
        attach(&mut follow, &mut sink, "session-1", 2);

        follow.next(2 + POLL_INTERVAL_MS);
        let outgoing = follow.resolve(Ok(sessions(&[("session-1", 10), ("session-2", 99)])), 3, &mut sink);
        assert_eq!(outgoing, None, "a busier conversation elsewhere changes nothing");
        assert_eq!(follow.session_id(), Some("session-1"));
        // But it is in the list, so the picker offers it.
        assert_eq!(follow.sessions()[0].session_id, "session-2");
    }

    #[test]
    fn a_refused_attach_keeps_looking() {
        // The host can refuse: the conversation may have been deleted between the list
        // and the attach. Giving up would leave the panel following nothing, which
        // looks exactly like having nothing to follow.
        let mut follow = started();
        let mut sink = Recorded::default();
        follow.next(0);
        let _ = follow.resolve(Ok(sessions(&[("session-1", 10)])), 1, &mut sink);
        follow.choose("session-1", 2);
        let refused = follow.resolve(
            Err(crate::ipc::rpc::RpcError { code: -32004, message: "no such session".to_owned(), data: None }),
            2,
            &mut sink,
        );
        assert_eq!(refused, None);
        assert_eq!(follow.session_id(), None);
        assert!(sink.joined().contains("could not follow session-1"));
        // And the decision is dropped: retrying a refused conversation every three seconds
        // would be a panel that looks attached and never is.
        assert_eq!(follow.chosen(), None);
        assert_eq!(follow.target(), None);
        assert_eq!(follow.next(2 + POLL_INTERVAL_MS), Some(Outgoing::ListSessions));
    }

    #[test]
    fn a_lost_answer_is_noticed_and_retried() {
        // Without this one dropped response stops discovery for the life of the
        // process, and the panel silently owns nothing for the rest of the session.
        let mut follow = started();
        follow.next(1_000);
        assert!(!follow.expired(1_000 + REQUEST_TIMEOUT_MS - 1));
        assert!(follow.expired(1_000 + REQUEST_TIMEOUT_MS + 1));
        follow.abandon(2_000);
        assert!(!follow.expired(2_100));
        assert_eq!(follow.next(2_000 + POLL_INTERVAL_MS), Some(Outgoing::ListSessions));
    }

    /// The titles the picker draws: the host carries them in the list.
    #[test]
    fn a_listed_conversation_carries_the_name_the_host_gave_it() {
        let mut follow = started();
        let mut sink = Recorded::default();
        follow.next(0);
        let _ = follow.resolve(
            Ok(json!({"items": [
                {"sessionId": "session-1", "updatedAt": 10, "title": "随机测试对话"},
                {"sessionId": "session-2", "updatedAt": 9, "title": "   "},
                {"sessionId": "session-3", "updatedAt": 8},
            ]})),
            1,
            &mut sink,
        );
        let titles: Vec<Option<&str>> = follow
            .sessions()
            .iter()
            .map(|entry| entry.title.as_deref())
            .collect();
        assert_eq!(titles, vec![Some("随机测试对话"), None, None], "blank and missing are no name");
        assert!(
            sink.marks.iter().any(|mark| mark == "sessions 3 named 1"),
            "and the record says how many arrived named: {:?}",
            sink.marks,
        );
    }

    #[test]
    fn entries_without_a_usable_id_are_skipped() {
        let mut follow = started();
        let mut sink = Recorded::default();
        follow.next(0);
        let outgoing = follow.resolve(
            Ok(json!({"items": [{"updatedAt": 99}, {"sessionId": "", "updatedAt": 98}, {"sessionId": "ok", "updatedAt": 1}]})),
            1,
            &mut sink,
        );
        assert_eq!(outgoing, None, "nothing is attached without a decision");
        // The picker offers what can actually be attached to — the other two entries cannot.
        let listed: Vec<&str> = follow.sessions().iter().map(|entry| entry.session_id.as_str()).collect();
        assert_eq!(listed, vec!["ok"]);
    }

    #[test]
    fn a_list_error_is_reported_and_retried() {
        let mut follow = started();
        let mut sink = Recorded::default();
        follow.next(0);
        follow.resolve(
            Err(crate::ipc::rpc::RpcError { code: -32601, message: "unsupported".to_owned(), data: None }),
            1,
            &mut sink,
        );
        assert!(sink.joined().contains("could not list conversations"));
        assert_eq!(follow.next(1 + POLL_INTERVAL_MS), Some(Outgoing::ListSessions));
    }

    /// A follow layer attached to `session-1`, having been told to be.
    ///
    /// Most of these tests are about what happens to frames once attached, so this does the
    /// two steps that get there — discovery, then a decision — and says so once.
    fn following() -> Follow {
        let mut follow = started();
        let mut sink = Recorded::default();
        follow.next(0);
        let _ = follow.resolve(Ok(sessions(&[("session-1", 10)])), 1, &mut sink);
        attach(&mut follow, &mut sink, "session-1", 2);
        follow.set_generation_for_tests(3);
        follow
    }

    #[test]
    fn session_traffic_is_counted_rather_than_logged() {
        // A turn produces hundreds of frames. Logging each one would spend the stderr
        // the host forwards and truncates, drowning the lines that matter — and the
        // counts are what makes "following and receiving" visible in the status line.
        let mut follow = following();
        let mut sink = Recorded::default();
        follow.record(
            "session/snapshot",
            Some(&json!({"sessionId": "session-1", "generation": 3, "cursor": 7, "records": [{}, {}]})),
            &mut sink,
        );
        assert!(sink.joined().contains("opened at cursor 7 with 2 record(s)"));
        let after_snapshot = sink.logs.len();
        for seq in 8..40 {
            assert!(follow.record(
                "session/event",
                Some(&json!({"sessionId": "session-1", "generation": 3, "seq": seq, "type": "assistant/delta"})),
                &mut sink,
            ));
        }
        assert_eq!(follow.events(), 32);
        assert_eq!(sink.logs.len(), after_snapshot, "no per-frame logging");
        assert!(follow.record("session/stream", Some(&json!({"sessionId": "session-1", "generation": 3})), &mut sink));
        assert_eq!(follow.streams(), 1);
    }

    #[test]
    fn traffic_for_another_conversation_is_ignored() {
        // A subscription that has been replaced can still have frames in flight, and
        // counting them would make the status line describe something nobody follows.
        let mut follow = following();
        let mut sink = Recorded::default();
        assert!(!follow.record("session/event", Some(&json!({"sessionId": "other", "seq": 1})), &mut sink));
        assert!(
            follow.record(
                "session/event",
                Some(&json!({"sessionId": "session-1", "generation": 2, "seq": 1})),
                &mut sink,
            ),
            "a frame from a replaced subscription is still this conversation's, and is handled by discarding it",
        );
        assert_eq!(follow.events(), 0, "but it does not count as an event of the live subscription");
        assert_eq!(follow.stale(), 1, "the replaced generation is counted, not silently dropped");
        assert!(follow.record(
            "session/event",
            Some(&json!({"sessionId": "session-1", "generation": 3, "seq": 1})),
            &mut sink,
        ));
        assert_eq!(follow.events(), 1);
    }

    #[test]
    fn a_hole_in_the_stream_is_reported_and_a_repeat_is_not_a_second_hole() {
        let mut follow = following();
        let mut sink = Recorded::default();
        let jumps = |sink: &Recorded| sink.logs.iter().filter(|line| line.contains("jumped from")).count();
        follow.record("session/snapshot", Some(&json!({"sessionId": "session-1", "generation": 3, "cursor": 10})), &mut sink);
        // Sequence 12 arrives where 11 was expected.
        follow.record("session/event", Some(&json!({"sessionId": "session-1", "generation": 3, "seq": 12})), &mut sink);
        assert_eq!(jumps(&sink), 1, "the hole is reported");
        assert!(sink.joined().contains("jumped from 10 to 12"));
        // Contiguous from there: nothing more to say.
        follow.record("session/event", Some(&json!({"sessionId": "session-1", "generation": 3, "seq": 13})), &mut sink);
        assert_eq!(jumps(&sink), 1);
        // The same frame again is a re-delivery, not a new hole.
        follow.record("session/event", Some(&json!({"sessionId": "session-1", "generation": 3, "seq": 12})), &mut sink);
        assert_eq!(jumps(&sink), 1, "one hole is one line");
        // A different one is a different hole, and worth saying.
        follow.record("session/event", Some(&json!({"sessionId": "session-1", "generation": 3, "seq": 20})), &mut sink);
        assert_eq!(jumps(&sink), 2);
        follow.record("session/resync", Some(&json!({"sessionId": "session-1", "generation": 3, "reason": "gap"})), &mut sink);
        assert_eq!(follow.resyncs(), 1);
        assert!(sink.joined().contains("reported a gap"));
        // The host repaired it: the new baseline is followed without complaint.
        follow.record("session/snapshot", Some(&json!({"sessionId": "session-1", "generation": 3, "cursor": 40})), &mut sink);
        follow.record("session/event", Some(&json!({"sessionId": "session-1", "generation": 3, "seq": 41})), &mut sink);
        assert_eq!(jumps(&sink), 2);
    }

    #[test]
    fn unrelated_notifications_are_not_claimed() {
        let mut follow = following();
        let mut sink = Recorded::default();
        assert!(!follow.record("host/heartbeat", Some(&json!({})), &mut sink));
        assert!(!follow.record("session/event", None, &mut sink));
    }
}

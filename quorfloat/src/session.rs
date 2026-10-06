//! The session: handshake, liveness, and shutdown.
//!
//! This module owns the *state* of a connection to the Harness host and knows
//! nothing about processes or sockets. Frames arrive through [`FrameSource`] and
//! leave through [`FrameSink`], both of which are small enough to implement with
//! a vector in a test. That is deliberate: the handshake order and the answers to
//! liveness traffic are the parts most likely to be wrong, and they must be
//! verifiable without a host, a model, or a spawned process.
//!
//! Two rules from `docs/protocol.md` shape everything here:
//!
//! - **stdout carries protocol frames and nothing else.** Log lines go to the
//!   sink's `log`, which is stderr. A stray `println!` would corrupt the stream
//!   the host is parsing, so no code in this crate writes to stdout directly.
//! - **The host judges liveness by `ping`, not by `host/heartbeat`.** A peer that
//!   forwarded heartbeats but never answered `ping` would be declared dead while
//!   happily talking, so `ping` is answered before anything else is considered.

use serde_json::{Value, json};
use std::collections::BTreeMap;

use crate::dump::Dump;
use crate::follow::{Follow, Outgoing};
use crate::ipc::protocol::{CAPABILITIES, PROTOCOL_VERSION, error_code};
use crate::ipc::rpc::{self, Inbound, Outcome, RpcError, Router};
use crate::transcript::Transcript;

/// Where outbound frames go.
pub trait FrameSink {
    /// Write one frame. Implementations must not buffer it indefinitely: the host
    /// treats silence as a stalled peer.
    ///
    /// @param frame - the message to encode and send.
    /// @returns an error when the peer is gone, which ends the session.
    fn send(&mut self, frame: &Value) -> std::io::Result<()>;

    /// Write one log line. Separate from [`FrameSink::send`] because it must not
    /// reach the protocol stream.
    ///
    /// @param line - text to record.
    fn log(&mut self, line: &str);

    /// Record one durable breadcrumb, if the deployment asked for any.
    ///
    /// A third destination, distinct from the other two on purpose: frames are the
    /// protocol, `log` goes to stderr **which the host captures and never shows**,
    /// and this goes to a file that a developer or a test can read afterwards. The
    /// default is a no-op, so a sink that has nowhere to put it needs no code and a
    /// shipped build writes nothing.
    ///
    /// @param line - text to record, without a trailing newline.
    fn mark(&mut self, line: &str) {
        let _ = line;
    }
}

/// Where inbound frames come from.
pub trait FrameSource {
    /// Read one decoded frame.
    ///
    /// @returns `None` at end of stream, which means the host closed stdin.
    fn next_frame(&mut self) -> Option<Inbound>;
}

/// What the host asked the window to do.
///
/// Parsed here and executed by the window layer, so the decision about *what* was
/// asked is testable without a window and the decision about *how* to do it stays
/// with the code that owns the viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowCommand {
    /// Show the panel.
    Show,
    /// Hide the panel.
    Hide,
    /// Flip whichever state the panel is in.
    ///
    /// The hotkey is handled locally by this process, so this exists for a host
    /// that wants the same behaviour from its own surface.
    Toggle,
}

impl WindowCommand {
    /// Read one `window/visibility` payload.
    ///
    /// Three spellings are accepted, and the tolerance is deliberate rather than
    /// accidental: this method is a *request* from the peer's point of view and a
    /// *notification* from the host's, so both `{visible}` and the command object
    /// the host's own router validates can arrive on the same wire. Accepting both
    /// costs one match arm; refusing one would make the two ends disagree about a
    /// method they both already implement.
    ///
    /// @param params - the notification parameters.
    /// @returns the command, or `None` when the payload asks for nothing
    ///   recognisable — which is ignored rather than treated as an error, because
    ///   a future host may extend this payload and an older peer must survive it.
    #[must_use]
    pub fn parse(params: Option<&Value>) -> Option<Self> {
        let params = params?;
        if let Some(visible) = params.get("visible").and_then(Value::as_bool) {
            return Some(if visible { Self::Show } else { Self::Hide });
        }
        match params.as_str() {
            Some("show") => Some(Self::Show),
            Some("hide") => Some(Self::Hide),
            Some("toggle") => Some(Self::Toggle),
            _ => None,
        }
    }
}

/// Why the session ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionExit {
    /// The host asked this process to stop. Exit cleanly and promptly: the host
    /// escalates to `SIGTERM` and then `SIGKILL`, and a clean exit is the
    /// evidence it looks for.
    ShutdownRequested,
    /// The host closed the channel, or it broke. Either way nobody is listening.
    PeerClosed,
}

/// Which waterfall an interaction came from.
///
/// The two are not variations of one thing: an approval is answered with a word
/// from a closed vocabulary, while a question is answered with selected option ids.
/// That difference is why this build can answer the first and can only *report* the
/// second, and why the kind travels on the wire instead of being inferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionKind {
    /// A tool call is asking permission — in practice, to escalate its sandbox.
    Approval,
    /// The model is asking the user something.
    Question,
}

impl InteractionKind {
    /// Read the kind from a payload.
    ///
    /// @param value - the `kind` field, if any.
    /// @returns the kind, or `None` for a spelling this build does not know —
    ///   which is ignored rather than guessed at, because guessing would put a
    ///   card with the wrong buttons in front of the user.
    #[must_use]
    pub fn parse(value: Option<&Value>) -> Option<Self> {
        match value.and_then(Value::as_str) {
            Some("approval") => Some(Self::Approval),
            Some("question") => Some(Self::Question),
            _ => None,
        }
    }

    /// The wire spelling, as the host writes it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Approval => "approval",
            Self::Question => "question",
        }
    }
}

/// What the user decided about an approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalVerdict {
    /// Grant this one call. The host's vocabulary calls it `allowed-once` because
    /// the grant is deliberately not remembered: the next call asks again.
    AllowOnce,
    /// Refuse. The upstream contract expects the tool to stop and explain rather
    /// than work around the refusal.
    Reject,
}

impl ApprovalVerdict {
    /// The wire spelling, as the host's answer validator expects it.
    #[must_use]
    pub fn wire(self) -> &'static str {
        match self {
            Self::AllowOnce => "allowed-once",
            Self::Reject => "rejected",
        }
    }
}

/// Where one interaction stands, from this panel's point of view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InteractionState {
    /// On screen and waiting for the user.
    Pending,
    /// The answer was sent; the host has not said what it did with it yet.
    Submitting {
        /// What was sent, kept so the card can still say what it asked for.
        verdict: ApprovalVerdict,
    },
    /// The host applied the answer. The end of the line: the grant is one-shot.
    Applied {
        /// What was applied.
        verdict: ApprovalVerdict,
    },
    /// The host refused the answer — another surface answered first, the asker
    /// withdrew, or the claim expired. **Not an error and not retryable**: the
    /// request is gone upstream, so a second attempt would be a guess. The reason
    /// is shown so the user is not left wondering whether the click registered.
    Refused {
        /// The host's explanation, or a stand-in when it gave none.
        reason: String,
    },
}

/// One request the host has handed to this panel to answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interaction {
    id: String,
    session_id: String,
    kind: InteractionKind,
    payload: Value,
    state: InteractionState,
}

/// How many characters of peer-supplied text the panel will render.
///
/// A frame may be up to a megabyte, and the asker's text comes from a model or a
/// hook. The bound is here rather than in the drawing code so that no path can hand
/// the window an unbounded string to lay out.
const MAX_TEXT_CHARS: usize = 600;

/// How many questions of one request are summarised for display.
const MAX_QUESTIONS_SHOWN: usize = 3;

/// How many pending cards are kept.
///
/// Not a threat model — the host is the process that spawned this one — but a peer
/// that publishes without ever resolving must not grow this process without bound.
const MAX_INTERACTIONS: usize = 16;

impl Interaction {
    /// The interaction id this panel answers with.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The session the request belongs to. Not used for routing — the host decided
    /// that already — but it is what makes a log line answer "which conversation".
    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Which waterfall this came from.
    #[must_use]
    pub fn kind(&self) -> InteractionKind {
        self.kind
    }

    /// Where the card stands.
    #[must_use]
    pub fn state(&self) -> &InteractionState {
        &self.state
    }

    /// Whether the user can still act on this card.
    #[must_use]
    pub fn is_actionable(&self) -> bool {
        self.kind == InteractionKind::Approval && self.state == InteractionState::Pending
    }

    /// The tool whose call is asking for permission.
    #[must_use]
    pub fn tool_name(&self) -> Option<&str> {
        self.payload.get("toolName").and_then(Value::as_str)
    }

    /// The most human-readable explanation the asker supplied.
    ///
    /// Preference order, and the reason for it:
    ///
    /// 1. `displayReason[locale]` — written *to be shown* to a user, localized by
    ///    the asker (upstream sandbox escalation sends `zh` and `en`).
    /// 2. `displayReason.en` — same, when the requested locale is absent.
    /// 3. `reason` — the audit string, written for a log. Still readable, and much
    ///    better than showing the user nothing.
    ///
    /// @param locale - the locale to prefer, e.g. `zh`.
    /// @returns the text, or `None` when the asker supplied none.
    #[must_use]
    pub fn detail(&self, locale: &str) -> Option<String> {
        if let Some(display) = self.payload.get("displayReason").and_then(Value::as_object) {
            for key in [locale, "en"] {
                if let Some(text) = display.get(key).and_then(Value::as_str) {
                    let text = text.trim();
                    if !text.is_empty() {
                        return Some(truncate(text));
                    }
                }
            }
        }
        let reason = self.payload.get("reason").and_then(Value::as_str)?.trim();
        if reason.is_empty() {
            return None;
        }
        Some(truncate(reason))
    }

    /// Up to [`MAX_QUESTIONS_SHOWN`] of the questions being asked, as display text.
    ///
    /// This build has no way to *answer* a question (see [`Session::answer_interaction`]),
    /// but it can still tell the user enough to decide whether to switch to the
    /// Harness window — which is the entire reason the host announces questions to
    /// the panel instead of staying silent.
    ///
    /// @returns `(header, text)` pairs, skipping entries without readable text.
    #[must_use]
    pub fn questions(&self) -> Vec<(Option<String>, String)> {
        let Some(items) = self.payload.get("questions").and_then(Value::as_array) else {
            return Vec::new();
        };
        items
            .iter()
            .filter_map(|item| {
                let text = item.get("question").and_then(Value::as_str)?.trim();
                if text.is_empty() {
                    return None;
                }
                let header = item
                    .get("header")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(truncate);
                Some((header, truncate(text)))
            })
            .take(MAX_QUESTIONS_SHOWN)
            .collect()
    }

    /// How many questions the request contains, including ones not shown.
    #[must_use]
    pub fn question_count(&self) -> usize {
        self.payload.get("questions").and_then(Value::as_array).map_or(0, Vec::len)
    }
}

/// A request the host routed elsewhere instead of to the panel.
///
/// Advisory, and deliberately not an [`Interaction`]: it has no id and cannot be
/// answered. It exists because the user is looking at the panel at that moment, so
/// the panel is the only surface that can tell them where the request went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handoff {
    kind: InteractionKind,
    reason: String,
    surfaces: Vec<String>,
}

impl Handoff {
    /// Which kind of request was handed on.
    #[must_use]
    pub fn kind(&self) -> InteractionKind {
        self.kind
    }

    /// Why the host routed it elsewhere, as the host spells it.
    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// Harness surfaces whose presence the host considered fresh.
    ///
    /// This is what decides whether "go and answer it there" is advice the panel can
    /// honestly give: with no live surface there is no window to send the user to.
    #[must_use]
    pub fn surfaces(&self) -> &[String] {
        &self.surfaces
    }
}

/// Cut peer-supplied text to what the panel will lay out.
///
/// Counts characters rather than bytes so a multi-byte string is never cut inside a
/// code point, and says so with an ellipsis so a truncated value is not mistaken for
/// the whole message.
///
/// @param text - the text to bound.
/// @returns the text, possibly shortened.
#[must_use]
fn truncate(text: &str) -> String {
    if text.chars().count() <= MAX_TEXT_CHARS {
        return text.to_owned();
    }
    let mut shortened: String = text.chars().take(MAX_TEXT_CHARS).collect();
    shortened.push('…');
    shortened
}

/// Facts this process reports about itself in `hello`.
#[derive(Debug, Clone)]
pub struct Identity {
    /// Version of this subproject, as built.
    pub quorfloat_version: String,
    /// The platform in the host's vocabulary (`platform::host_platform`).
    pub platform: String,
    /// The architecture in the host's vocabulary (`platform::host_arch`).
    pub arch: String,
    /// The hotkey this process wants, and whether it managed to register it.
    ///
    /// Reported in `hello` rather than after the fact so the host can show a
    /// conflict in its settings surface instead of the user discovering that the
    /// key does nothing.
    pub hotkey: HotkeyReport,
}

/// The hotkey this process asked for and what happened to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyReport {
    /// The accelerator as requested, e.g. `Alt+Space`.
    pub requested: String,
    /// Whether registration succeeded. `false` is a normal outcome on a desktop
    /// where something else already owns the key.
    pub registered: bool,
}

impl HotkeyReport {
    /// Read the requested accelerator from the environment.
    ///
    /// The host passes the configured hotkey as `DSH_QUORFLOAT_HOTKEY` when it
    /// spawns this process, so a user changing the setting in the Harness
    /// settings surface does not require this side to read the host's config
    /// files. Registration itself needs a window system and arrives with P1; the
    /// name is reported regardless, because it is the requested accelerator and
    /// not the achieved one.
    ///
    /// @returns the report, with `registered: false` until P1 wires a real grab.
    #[must_use]
    pub fn requested_from_env() -> Self {
        let requested = std::env::var("DSH_QUORFLOAT_HOTKEY")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "Alt+Space".to_owned());
        Self { requested, registered: false }
    }
}

/// What the host sent in `ready` after a successful handshake.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HostConfig {
    /// The channel identity for this connection generation.
    pub session_id: Option<String>,
    /// The host's own version, for diagnostics.
    pub host_version: Option<String>,
    /// Window geometry and appearance the host wants this process to use.
    pub window: Option<Value>,
    /// The hotkey the host considers effective.
    pub hotkey: Option<String>,
    /// Effective `heartbeatMs`, for logging only: the host drives the beat.
    pub heartbeat_ms: Option<i64>,
}

/// The connection state machine.
pub struct Session {
    identity: Identity,
    router: Router,
    host: HostConfig,
    /// Window commands the host has sent but the window layer has not applied.
    ///
    /// Queued rather than applied on the spot because this type has no window:
    /// keeping the two apart is what lets the whole command path be tested without
    /// a viewport.
    commands: Vec<WindowCommand>,
    /// Requests the host has handed to this panel, oldest first.
    ///
    /// Owned here rather than by the window for the same reason [`Session::commands`]
    /// is: what the host asked for and how it is presented are separate decisions,
    /// and only the first one needs a window to be wrong about.
    interactions: Vec<Interaction>,
    /// Which interaction each `interaction/answer` request in flight belongs to.
    ///
    /// Keyed by request id because that is all the host's response carries back;
    /// without it an answer arriving after two sends could not be attributed.
    answers_in_flight: BTreeMap<i64, String>,
    /// The last hand-off the host announced, until the user dismisses it.
    handoff: Option<Handoff>,
    /// The conversation this panel follows, and why that matters for ownership.
    follow: Follow,
    /// The conversation, folded from the records and streams the host sent.
    transcript: Transcript,
    /// Where raw conversation frames are captured, when a capture was asked for.
    ///
    /// Owned here rather than read from the environment in [`Session::new`] so tests
    /// stay hermetic and the one place that reads the environment stays in `main`.
    dump: Dump,
    /// The id and send time of the outstanding follow request, if any.
    follow_request: Option<(i64, i64)>,
    next_request_id: i64,
    handshake_sent: bool,
    handshake_done: bool,
}

impl Session {
    /// Create a session that has not yet spoken.
    ///
    /// @param identity - what to report about this process.
    #[must_use]
    pub fn new(identity: Identity) -> Self {
        Self {
            identity,
            router: Router,
            host: HostConfig::default(),
            commands: Vec::new(),
            interactions: Vec::new(),
            answers_in_flight: BTreeMap::new(),
            handoff: None,
            follow: Follow::default(),
            transcript: Transcript::new(),
            dump: Dump::default(),
            follow_request: None,
            next_request_id: 1,
            handshake_sent: false,
            handshake_done: false,
        }
    }

    /// Install a raw-frame capture.
    ///
    /// @param dump - where conversation notifications are appended verbatim.
    pub fn set_dump(&mut self, dump: Dump) {
        self.dump = dump;
    }

    /// What the host sent in `ready`, once it has.
    #[must_use]
    pub fn host_config(&self) -> &HostConfig {
        &self.host
    }

    /// Whether the handshake completed.
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.handshake_done
    }

    /// Run the handshake, then service frames until the host lets go.
    ///
    /// @param source - inbound frames; `None` ends the session.
    /// @param sink - outbound frames and logging.
    /// @returns why the session ended.
    /// @throws Never: a dead peer is a normal ending, not an error.
    pub fn run(&mut self, source: &mut dyn FrameSource, sink: &mut dyn FrameSink) -> SessionExit {
        self.send_hello(sink);
        loop {
            let Some(frame) = source.next_frame() else {
                sink.log("host closed the channel; exiting");
                return SessionExit::PeerClosed;
            };
            if let Some(exit) = self.on_frame(frame, sink) {
                return exit;
            }
        }
    }

    /// Send the opening `hello` request.
    ///
    /// The host waits for this before it considers the process started, so it is
    /// the first thing written — before the window exists, before any device is
    /// opened. A slow handshake is indistinguishable from a broken binary.
    fn send_hello(&mut self, sink: &mut dyn FrameSink) {
        if self.handshake_sent {
            return;
        }
        self.handshake_sent = true;
        let params = json!({
            "protocol": PROTOCOL_VERSION,
            "quorfloatVersion": self.identity.quorfloat_version,
            "platform": self.identity.platform,
            "arch": self.identity.arch,
            "capabilities": CAPABILITIES,
            "hotkey": {
                "requested": self.identity.hotkey.requested,
                "registered": self.identity.hotkey.registered,
            },
        });
        let id = self.take_request_id();
        if let Err(error) = sink.send(&rpc::request(id, "hello", params)) {
            sink.log(&format!("could not send hello: {error}"));
        }
    }

    /// Send the opening `hello` request, once.
    ///
    /// Public because the GUI owns the event loop: it must write the handshake
    /// before the window exists, then keep servicing frames from inside its own
    /// update callback rather than from a blocking loop.
    ///
    /// @param sink - where the frame goes.
    pub fn start(&mut self, sink: &mut dyn FrameSink) {
        self.send_hello(sink);
    }

    /// Handle one inbound frame.
    ///
    /// @param frame - a classified frame from the transport.
    /// @param sink - where answers go.
    /// @returns `Some(exit)` when the session is over.
    pub fn on_frame(&mut self, frame: Inbound, sink: &mut dyn FrameSink) -> Option<SessionExit> {
        match frame {
            Inbound::Request { id, method, params } => {
                let outcome = self.router.handle(&method, params.as_ref());
                // The host is waiting on this answer to decide whether it has to
                // escalate to a signal, so it is flushed before the loop ends.
                let shutdown = method == "shutdown";
                let replied = self.reply(&id, outcome, sink);
                if shutdown {
                    sink.log("shutdown requested by the host; exiting");
                    return Some(SessionExit::ShutdownRequested);
                }
                replied
            }
            Inbound::Notification { method, params } => {
                self.on_notification(&method, params, sink);
                None
            }
            Inbound::Response { id, outcome } => {
                self.on_response(&id, outcome, sink);
                None
            }
            Inbound::Malformed => {
                // The host's own rule for this case: count it and keep serving.
                sink.log("ignoring a frame that is not a JSON-RPC message");
                None
            }
        }
    }

    /// Send the answer to one request.
    fn reply(&mut self, id: &Value, outcome: Outcome, sink: &mut dyn FrameSink) -> Option<SessionExit> {
        let frame = match outcome {
            Outcome::Ok(result) => rpc::success(id, result),
            Outcome::Err(error) => rpc::failure(id, &error),
            // A method that ran but produced nothing to say. Only `shutdown`
            // reaches this today, and it answers with an empty result so the host
            // sees its request acknowledged rather than ignored.
            Outcome::Accepted => rpc::success(id, json!({})),
        };
        if let Err(error) = sink.send(&frame) {
            sink.log(&format!("could not answer a request: {error}"));
        }
        None
    }

    /// React to one notification.
    fn on_notification(&mut self, method: &str, params: Option<Value>, sink: &mut dyn FrameSink) {
        match method {
            // Liveness, in the host's direction. Nothing to do: the host judges
            // this process by its `ping` answers, and a peer that echoed
            // heartbeats back would add traffic without adding evidence.
            "host/heartbeat" => {}
            "host/config" => {
                // The same shape as `ready`'s `config`, for changes that do not
                // need a restart. Applied through the same path so there is one
                // interpretation of it rather than two that drift.
                self.absorb_config(params.as_ref());
                sink.log("host pushed configuration");
            }
            "ready" => {
                // The post-handshake payload: the effective configuration. It
                // arrives as a notification, so nothing is expected back.
                self.absorb_config(params.as_ref().and_then(|value| value.get("config")));
                sink.log("host sent the effective configuration");
            }
            "interaction/open" => self.admit_interaction(params.as_ref(), sink),
            "window/visibility" => match WindowCommand::parse(params.as_ref()) {
                Some(command) => self.commands.push(command),
                None => sink.log("ignoring an unrecognised window/visibility payload"),
            },
            "interaction/hint" => self.note_handoff(params.as_ref(), sink),
            // Traffic for the conversation this panel follows. Counted rather than
            // rendered — but counted on purpose: the status line is how a user tells
            // "following and receiving" from "following and getting nothing", and the
            // two are identical from every other angle.
            // Dumped before it is interpreted, and for every session rather than only
            // the followed one: the point is to see what the host sends, and filtering
            // to what this build happens to understand would hide the kinds it does not.
            "session/snapshot" => {
                self.dump.record(method, params.as_ref());
                if let Some(params) = params.as_ref() {
                    self.transcript.apply_snapshot(params);
                }
                self.follow.record(method, params.as_ref(), sink);
                // What the panel now holds, on the record. "The conversation is empty"
                // and "the conversation never arrived" look identical on screen, and
                // this is the line that tells them apart.
                let title = self
                    .transcript
                    .title()
                    .map_or_else(String::new, |title| format!(" title={title:?}"));
                sink.mark(&format!(
                    "transcript {} entries cursor={}{title}",
                    self.transcript.entries().len(),
                    self.transcript.cursor(),
                ));
                if let Some(skipped) = self.transcript.skipped().describe() {
                    sink.mark(&skipped);
                }
            }
            "session/event" => {
                self.dump.record(method, params.as_ref());
                if let Some(params) = params.as_ref() {
                    self.transcript.apply_event(params);
                }
                self.follow.record(method, params.as_ref(), sink);
            }
            "session/stream" => {
                self.dump.record(method, params.as_ref());
                if let Some(params) = params.as_ref() {
                    self.transcript.apply_stream(params);
                }
                self.follow.record(method, params.as_ref(), sink);
            }
            "session/resync" => {
                self.dump.record(method, params.as_ref());
                let detail = params
                    .as_ref()
                    .and_then(|params| self.transcript.apply_resync(params));
                self.follow.record(method, params.as_ref(), sink);
                // The gap cannot be repaired from here: what arrived is known to be
                // incomplete, and re-subscribing is the only way to get a state that is
                // not a guess. Recorded *and* acted on, because a resync that changes
                // nothing on screen is indistinguishable from a lost conversation.
                if let Some(detail) = detail {
                    sink.mark(&format!("transcript resync {detail}"));
                    self.resubscribe(sink);
                }
            }
            other => sink.log(&format!("ignoring a notification this build does not use: {other}")),
        }
    }

    /// Record what the handshake produced, or the host's verdict on an answer.
    fn on_response(&mut self, id: &Value, outcome: Result<Value, RpcError>, sink: &mut dyn FrameSink) {
        // In-flight answers are checked before the handshake id: they are the only
        // responses this process solicits after `hello`, and a mismatch here would
        // leave a card stuck on "sending…" with nothing to explain why.
        if let Some(interaction_id) = id.as_i64().and_then(|value| self.answers_in_flight.remove(&value)) {
            self.resolve_answer(&interaction_id, outcome, sink);
            return;
        }
        let request_id = id.as_i64();
        if let Some((pending, _)) = self.follow_request {
            if request_id == Some(pending) {
                self.follow_request = None;
                let now = rpc::now_millis();
                if let Some(outgoing) = self.follow.resolve(outcome, now, sink) {
                    self.send_follow(outgoing, now, sink);
                }
                return;
            }
        }
        if id.as_i64() != Some(1) {
            sink.log(&format!("ignoring a response to an unknown request id: {id}"));
            return;
        }
        match outcome {
            Ok(result) => {
                self.host.session_id = result.get("sessionId").and_then(Value::as_str).map(str::to_owned);
                self.host.host_version = result.get("hostVersion").and_then(Value::as_str).map(str::to_owned);
                self.handshake_done = true;
                // Only now can session calls be made: the host refuses them on a
                // channel that has not said `hello`, and a request sent too early is a
                // failure the panel would have to retry through.
                self.follow.begin();
                sink.log(&format!(
                    "handshake complete: host={} session={}",
                    self.host.host_version.as_deref().unwrap_or("unknown"),
                    self.host.session_id.as_deref().unwrap_or("unknown"),
                ));
            }
            Err(error) => {
                // A version mismatch is not a bug to retry through: the host will
                // reject every frame until the two halves agree, so say which
                // version it wanted and stop.
                sink.log(&format!(
                    "handshake refused: [{code}] {message}{detail}",
                    code = error.code,
                    message = error.message,
                    detail = match &error.data {
                        Some(data) => format!(" (data: {data})"),
                        None => String::new(),
                    },
                ));
                if error.code == error_code::PROTOCOL_MISMATCH {
                    sink.log(&format!("this build speaks {PROTOCOL_VERSION}"));
                }
            }
        }
    }

    /// Report whether the panel is on screen, and what this process can actually do.
    ///
    /// The host routes approvals by which surface the user is looking at, and this
    /// notification is how it learns that the panel is one of them. It is sent on
    /// every change including the first (hidden), because a panel that never
    /// reports is indistinguishable from one that is not running.
    ///
    /// `capabilities` is additive and optional: the host's validator reads
    /// `visible` and ignores the rest, so an older host keeps working. Sending it
    /// here rather than only in `hello` is what makes it a *measurement*: `hello`
    /// is written before any window exists, so a window capability in it would be a
    /// claim, whereas this is sent once creation has actually succeeded.
    ///
    /// @param visible - whether the window is currently shown.
    /// @param capabilities - what this process has established it can do.
    /// @param sink - where the notification goes.
    pub fn report_visibility(
        &self,
        visible: bool,
        capabilities: &[&str],
        sink: &mut dyn FrameSink,
    ) {
        let frame = rpc::notification(
            "window/visibility",
            json!({ "visible": visible, "capabilities": capabilities }),
        );
        if let Err(error) = sink.send(&frame) {
            sink.log(&format!("could not report window visibility: {error}"));
        }
    }

    /// Take every window command the host has sent since the last call.
    ///
    /// Draining rather than peeking: the caller applies them in order, and leaving
    /// them in place would make a command run again on every pass.
    ///
    /// @returns the commands, oldest first.
    #[must_use]
    pub fn take_window_commands(&mut self) -> Vec<WindowCommand> {
        std::mem::take(&mut self.commands)
    }

    /// Every interaction this panel is holding, oldest first.
    #[must_use]
    pub fn interactions(&self) -> &[Interaction] {
        &self.interactions
    }

    /// The conversation this panel follows.
    ///
    /// Load-bearing rather than informational: an interaction is claimed only for a
    /// session that has been attached, so this is what decides whether an approval can
    /// reach the panel at all.
    #[must_use]
    pub fn follow(&self) -> &Follow {
        &self.follow
    }

    /// Keep the followed conversation current.
    ///
    /// Called from the window's pass, which runs whether or not the panel is on
    /// screen. Nothing here blocks: a request goes out, and its answer arrives through
    /// the ordinary response path — so a panel sitting idle still discovers the
    /// conversation that was just started next to it.
    ///
    /// @param sink - where the request and any complaint go.
    pub fn pump_follow(&mut self, sink: &mut dyn FrameSink) {
        let now = rpc::now_millis();
        self.pump_follow_at(now, sink);
    }

    /// The same, with the clock supplied.
    ///
    /// Time is a parameter rather than read inside so that "the answer never came" is
    /// testable without waiting ten seconds for it.
    ///
    /// @param now - caller-local time in milliseconds.
    /// @param sink - where the request and any complaint go.
    pub fn pump_follow_at(&mut self, now: i64, sink: &mut dyn FrameSink) {
        if !self.follow.is_started() {
            return;
        }
        if let Some((_, sent_at)) = self.follow_request {
            if !self.follow.expired(now) {
                return;
            }
            // The answer never came. Retrying is the only recovery: discovery is what
            // makes the panel own anything, and giving up would leave it silently
            // answering nothing for the rest of the session.
            self.follow_request = None;
            sink.log(&format!(
                "no answer to a conversation request after {}ms; it will be retried",
                now - sent_at,
            ));
            self.follow.abandon(now);
        }
        let Some(outgoing) = self.follow.next(now) else { return };
        self.send_follow(outgoing, now, sink);
    }

    /// Send one conversation request, and remember that its answer is expected.
    ///
    /// @param outgoing - what to ask for.
    /// @param now - caller-local time, recorded so a lost answer can be noticed.
    /// @param sink - where the frame goes.
    fn send_follow(&mut self, outgoing: Outgoing, now: i64, sink: &mut dyn FrameSink) {
        let id = self.take_request_id();
        let frame = match &outgoing {
            Outgoing::ListSessions => rpc::request(id, "sessions/list", json!({})),
            Outgoing::Attach { session_id } => {
                rpc::request(id, "session/attach", json!({ "sessionId": session_id }))
            }
        };
        self.follow_request = Some((id, now));
        if let Err(error) = sink.send(&frame) {
            self.follow_request = None;
            self.follow.abandon(now);
            sink.log(&format!("could not ask about conversations: {error}"));
        }
    }

    /// Ask the host for a fresh subscription to the conversation being followed.
    ///
    /// Used after a reported gap. A failure here is logged and otherwise ignored: the
    /// transcript has already recorded that its contents are incomplete, and a panel
    /// that says so is better than one that retries in silence.
    fn resubscribe(&mut self, sink: &mut dyn FrameSink) {
        let target = self
            .follow
            .session_id()
            .or_else(|| self.transcript.session_id())
            .map(str::to_owned);
        let Some(target) = target else {
            sink.log("a resync arrived but no conversation is being followed");
            return;
        };
        let now = rpc::now_millis();
        if let Some(outgoing) = self.follow.resubscribe(&target, now) {
            self.send_follow(outgoing, now, sink);
        }
    }

    /// The conversation, as far as this panel knows it.
    #[must_use]
    pub fn transcript(&self) -> &Transcript {
        &self.transcript
    }

    /// The last hand-off the host announced, if the user has not dismissed it.
    #[must_use]
    pub fn handoff(&self) -> Option<&Handoff> {
        self.handoff.as_ref()
    }

    /// Forget the announced hand-off.
    ///
    /// Purely local: the host neither knows nor needs to know that the panel stopped
    /// showing a banner.
    pub fn dismiss_handoff(&mut self) {
        self.handoff = None;
    }

    /// Answer one approval.
    ///
    /// Refuses in four cases, each deliberately rather than by omission:
    ///
    /// - the id is not held (a stale card, or one from a previous process);
    /// - the request is already in flight or already resolved — the grant is
    ///   one-shot, so a second send would be a second decision for one question;
    /// - the request is a **question**, whose answer shape is a list of selected
    ///   option ids (`{answers:[{id,selected}]}`). This build has no widget for that
    ///   yet, and inventing an answer would tell the model the user chose something
    ///   they never saw;
    /// - the frame could not be written, in which case the state is put back so the
    ///   card does not claim a send that did not happen.
    ///
    /// @param interaction_id - which request to answer.
    /// @param verdict - what the user decided.
    /// @param sink - where the answer goes.
    /// @returns whether an answer was sent.
    pub fn answer_interaction(
        &mut self,
        interaction_id: &str,
        verdict: ApprovalVerdict,
        sink: &mut dyn FrameSink,
    ) -> bool {
        let Some(index) = self.interactions.iter().position(|held| held.id() == interaction_id) else {
            sink.log(&format!("not answering {interaction_id}: this panel is not holding it"));
            return false;
        };
        match self.interactions[index].state() {
            InteractionState::Pending => {}
            InteractionState::Submitting { .. } => {
                sink.log(&format!("not answering {interaction_id}: an answer is already in flight"));
                return false;
            }
            InteractionState::Applied { .. } | InteractionState::Refused { .. } => {
                sink.log(&format!("not answering {interaction_id}: it is already resolved"));
                return false;
            }
        }
        if self.interactions[index].kind() != InteractionKind::Approval {
            sink.log(&format!(
                "not answering {interaction_id}: this build cannot answer a question, only report it",
            ));
            return false;
        }
        let request_id = self.take_request_id();
        let frame = rpc::request(
            request_id,
            "interaction/answer",
            json!({
                "interactionId": interaction_id,
                "answer": { "outcome": verdict.wire() },
            }),
        );
        self.interactions[index].state = InteractionState::Submitting { verdict };
        self.answers_in_flight.insert(request_id, interaction_id.to_owned());
        sink.mark(&format!("interaction {interaction_id} answering {}", verdict.wire()));
        if let Err(error) = sink.send(&frame) {
            self.interactions[index].state = InteractionState::Pending;
            self.answers_in_flight.remove(&request_id);
            sink.log(&format!("could not send an answer for {interaction_id}: {error}"));
            return false;
        }
        true
    }

    /// Drop one interaction from the panel.
    ///
    /// Local only, and honest about what it does not do: the host still holds the
    /// request until its own deadline. It exists for cards that cannot be acted on
    /// (a reported question) and for resolved ones the user has read.
    ///
    /// @param interaction_id - which card to drop.
    /// @returns whether a card was dropped.
    pub fn dismiss_interaction(&mut self, interaction_id: &str) -> bool {
        let before = self.interactions.len();
        self.interactions.retain(|held| held.id() != interaction_id);
        self.interactions.len() != before
    }

    /// Take one `interaction/open` payload into the pending list.
    ///
    /// Three refusals, all logged rather than swallowed:
    ///
    /// - no id or an unknown `kind`, because a card without either would offer the
    ///   user buttons that cannot mean anything;
    /// - an id already held, because the host may re-publish (a restart re-delivers
    ///   what is still pending) and two cards for one decision is worse than one;
    /// - more than [`MAX_INTERACTIONS`] already pending.
    ///
    /// @param params - the notification payload.
    /// @param sink - where the breadcrumb and any complaint go.
    fn admit_interaction(&mut self, params: Option<&Value>, sink: &mut dyn FrameSink) {
        let Some(params) = params else {
            sink.log("ignoring an interaction/open without parameters");
            return;
        };
        let Some(id) = params.get("interactionId").and_then(Value::as_str) else {
            sink.log("ignoring an interaction/open without an interactionId");
            return;
        };
        let Some(kind) = InteractionKind::parse(params.get("kind")) else {
            sink.log(&format!("ignoring interaction {id}: unknown kind"));
            return;
        };
        if self.interactions.iter().any(|held| held.id() == id) {
            sink.log(&format!("ignoring a re-published interaction {id}: already holding it"));
            return;
        }
        if self.interactions.len() >= MAX_INTERACTIONS {
            sink.log(&format!("ignoring interaction {id}: {MAX_INTERACTIONS} are already pending"));
            return;
        }
        self.interactions.push(Interaction {
            id: id.to_owned(),
            session_id: params.get("sessionId").and_then(Value::as_str).unwrap_or_default().to_owned(),
            kind,
            payload: params.get("payload").cloned().unwrap_or(Value::Null),
            state: InteractionState::Pending,
        });
        // Breadcrumbed at the moment of arrival, before any window is involved: this
        // is the line that answers "did the approval reach the panel at all".
        sink.mark(&format!("interaction {id} {} arrived", kind.as_str()));
    }

    /// Record the host's announcement that a request went to another surface.
    ///
    /// Advisory only, so a payload this build cannot read is dropped rather than
    /// guessed at — an unreadable reason is still worth a banner, but an unreadable
    /// *kind* would decide which sentence the user reads, and that must not be a
    /// guess.
    ///
    /// @param params - the notification payload.
    /// @param sink - where the breadcrumb and any complaint go.
    fn note_handoff(&mut self, params: Option<&Value>, sink: &mut dyn FrameSink) {
        let Some(params) = params else {
            sink.log("ignoring an interaction/hint without parameters");
            return;
        };
        let Some(kind) = InteractionKind::parse(params.get("kind")) else {
            sink.log("ignoring an interaction/hint with an unknown kind");
            return;
        };
        let reason = params.get("reason").and_then(Value::as_str).unwrap_or("unspecified").to_owned();
        let surfaces = params
            .get("surfaces")
            .and_then(Value::as_array)
            .map(|entries| {
                entries.iter().filter_map(Value::as_str).map(str::to_owned).collect::<Vec<String>>()
            })
            .unwrap_or_default();
        sink.mark(&format!(
            "handoff {} {reason} surfaces={}",
            kind.as_str(),
            surfaces.len(),
        ));
        self.handoff = Some(Handoff { kind, reason, surfaces });
    }

    /// Apply the host's verdict on an answer this panel sent.
    ///
    /// @param interaction_id - the interaction the response belongs to.
    /// @param outcome - the response, or the error that came back instead.
    /// @param sink - where the breadcrumb and any complaint go.
    fn resolve_answer(
        &mut self,
        interaction_id: &str,
        outcome: Result<Value, RpcError>,
        sink: &mut dyn FrameSink,
    ) {
        let Some(index) = self.interactions.iter().position(|held| held.id() == interaction_id) else {
            sink.log(&format!("a response arrived for interaction {interaction_id}, which is not held"));
            return;
        };
        let InteractionState::Submitting { verdict } = self.interactions[index].state else {
            sink.log(&format!("a response arrived for interaction {interaction_id}, which is not in flight"));
            return;
        };
        match outcome {
            Ok(result) if result.get("accepted").and_then(Value::as_bool) == Some(true) => {
                self.interactions[index].state = InteractionState::Applied { verdict };
                sink.mark(&format!("interaction {interaction_id} applied {}", verdict.wire()));
            }
            Ok(result) => {
                // `accepted:false` is a normal answer, not a failure: another surface
                // answered first, the asker withdrew, or the claim expired. The host
                // usually explains which.
                let reason = result
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("the host did not say why")
                    .to_owned();
                self.interactions[index].state = InteractionState::Refused { reason: reason.clone() };
                sink.mark(&format!("interaction {interaction_id} refused: {reason}"));
            }
            Err(error) => {
                let reason = format!("[{}] {}", error.code, error.message);
                self.interactions[index].state = InteractionState::Refused { reason: reason.clone() };
                sink.mark(&format!("interaction {interaction_id} refused: {reason}"));
            }
        }
    }

    /// Fold the host's configuration into the stored view.
    fn absorb_config(&mut self, config: Option<&Value>) {
        let Some(config) = config else { return };
        self.host.window = config.get("window").cloned().or(self.host.window.take());
        self.host.hotkey = config
            .get("hotkey")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or(self.host.hotkey.take());
        self.host.heartbeat_ms = config.get("heartbeatMs").and_then(Value::as_i64).or(self.host.heartbeat_ms);
    }

    /// Allocate the next request id.
    fn take_request_id(&mut self) -> i64 {
        let id = self.next_request_id;
        self.next_request_id += 1;
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// A sink that keeps frames in memory instead of writing them.
    #[derive(Default)]
    struct RecordingSink {
        frames: Vec<Value>,
        logs: Vec<String>,
        marks: Vec<String>,
    }

    impl FrameSink for RecordingSink {
        fn send(&mut self, frame: &Value) -> std::io::Result<()> {
            self.frames.push(frame.clone());
            Ok(())
        }

        fn log(&mut self, line: &str) {
            self.logs.push(line.to_owned());
        }

        fn mark(&mut self, line: &str) {
            self.marks.push(line.to_owned());
        }
    }

    /// A source that hands out prepared frames and then reports end of stream.
    struct ScriptedSource {
        frames: VecDeque<Inbound>,
    }

    impl ScriptedSource {
        fn new(frames: Vec<Inbound>) -> Self {
            Self { frames: frames.into() }
        }
    }

    impl FrameSource for ScriptedSource {
        fn next_frame(&mut self) -> Option<Inbound> {
            self.frames.pop_front()
        }
    }

    fn identity() -> Identity {
        Identity {
            quorfloat_version: "0.0.1".to_owned(),
            platform: "darwin".to_owned(),
            arch: "arm64".to_owned(),
            hotkey: HotkeyReport { requested: "Alt+Space".to_owned(), registered: true },
        }
    }

    fn request(id: i64, method: &str) -> Inbound {
        Inbound::Request { id: json!(id), method: method.to_owned(), params: None }
    }

    fn notification(method: &str, params: Value) -> Inbound {
        Inbound::Notification { method: method.to_owned(), params: Some(params) }
    }

    /// The host's answer to a good `hello`, trimmed to the fields this side reads.
    fn hello_ok() -> Inbound {
        Inbound::Response {
            id: json!(1),
            outcome: Ok(json!({"protocol": "quorfloat/1", "hostVersion": "0.2.0-rc.2",
                               "sessionId": "quorfloat-1-abc"})),
        }
    }

    #[test]
    fn the_first_frame_is_hello_before_anything_else() {
        // The host's startup budget starts when the process does. A peer that
        // initialised a window first would be killed mid-handshake.
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        let mut source = ScriptedSource::new(vec![]);
        assert_eq!(session.run(&mut source, &mut sink), SessionExit::PeerClosed);
        assert_eq!(sink.frames.len(), 1);
        let hello = &sink.frames[0];
        assert_eq!(hello["method"], "hello");
        assert_eq!(hello["jsonrpc"], "2.0");
        assert_eq!(hello["id"], 1);
    }

    #[test]
    fn hello_carries_exactly_the_facts_the_host_records() {
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        let mut source = ScriptedSource::new(vec![]);
        session.run(&mut source, &mut sink);
        let params = &sink.frames[0]["params"];
        assert_eq!(params["protocol"], PROTOCOL_VERSION);
        assert_eq!(params["quorfloatVersion"], "0.0.1");
        assert_eq!(params["platform"], "darwin");
        assert_eq!(params["arch"], "arm64");
        assert_eq!(params["capabilities"], json!(["window", "hotkey", "egui", "approval"]));
        assert_eq!(params["hotkey"]["requested"], "Alt+Space");
        assert_eq!(params["hotkey"]["registered"], true);
    }

    #[test]
    fn a_good_handshake_records_the_host_facts() {
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        let mut source = ScriptedSource::new(vec![hello_ok()]);
        session.run(&mut source, &mut sink);
        assert!(session.is_ready());
        assert_eq!(session.host_config().host_version.as_deref(), Some("0.2.0-rc.2"));
        assert_eq!(session.host_config().session_id.as_deref(), Some("quorfloat-1-abc"));
    }

    #[test]
    fn ping_is_answered_even_when_no_handshake_has_succeeded() {
        // The host's liveness verdict is exactly "did `ping` come back". Treating
        // an early ping as out of order would have it declare this process dead
        // while the process is healthy.
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        let mut source = ScriptedSource::new(vec![request(7, "ping")]);
        session.run(&mut source, &mut sink);
        let answer = sink.frames.iter().find(|frame| frame["id"] == 7).expect("ping was answered");
        assert_eq!(answer["jsonrpc"], "2.0");
        assert!(answer.get("result").is_some());
        assert!(answer.get("error").is_none());
    }

    #[test]
    fn an_unknown_method_is_refused_rather_than_answered_empty() {
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        let mut source = ScriptedSource::new(vec![request(3, "session/create")]);
        session.run(&mut source, &mut sink);
        let answer = sink.frames.iter().find(|frame| frame["id"] == 3).expect("answered");
        assert_eq!(answer["error"]["code"], error_code::METHOD_NOT_FOUND);
    }

    #[test]
    fn a_shutdown_request_is_answered_and_then_ends_the_session() {
        // Both halves matter: the answer is what stops the host escalating, and
        // the exit is what makes the cleanup evidence it accepts.
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        let mut source = ScriptedSource::new(vec![request(11, "shutdown"), request(12, "ping")]);
        let exit = session.run(&mut source, &mut sink);
        assert_eq!(exit, SessionExit::ShutdownRequested);
        let answer = sink.frames.iter().find(|frame| frame["id"] == 11).expect("shutdown was answered");
        assert!(answer.get("result").is_some(), "answered with a result, not an error");
        assert!(
            !sink.frames.iter().any(|frame| frame["id"] == 12),
            "nothing after shutdown is served",
        );
    }

    #[test]
    fn a_notification_is_never_answered() {
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        let mut source = ScriptedSource::new(vec![notification("host/heartbeat", json!({"t": 1}))]);
        session.run(&mut source, &mut sink);
        assert_eq!(sink.frames.len(), 1, "only the opening hello was written");
    }

    #[test]
    fn a_protocol_mismatch_is_reported_with_the_supported_versions() {
        // Retrying would be pointless and hiding it would leave a process that
        // looks alive while every frame is rejected.
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        let mut source = ScriptedSource::new(vec![Inbound::Response {
            id: json!(1),
            outcome: Err(RpcError {
                code: error_code::PROTOCOL_MISMATCH,
                message: "unsupported protocol version: quorfloat/2".to_owned(),
                data: Some(json!({"supported": [PROTOCOL_VERSION]})),
            }),
        }]);
        session.run(&mut source, &mut sink);
        assert!(!session.is_ready());
        let joined = sink.logs.join("\n");
        assert!(joined.contains("handshake refused"));
        assert!(joined.contains("quorfloat/1"), "the version this build speaks is stated");
    }

    #[test]
    fn end_of_stream_ends_the_session_without_error() {
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        let mut source = ScriptedSource::new(vec![]);
        assert_eq!(session.run(&mut source, &mut sink), SessionExit::PeerClosed);
    }

    #[test]
    fn a_malformed_frame_is_logged_and_the_session_continues() {
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        let mut source = ScriptedSource::new(vec![Inbound::Malformed, request(5, "ping")]);
        session.run(&mut source, &mut sink);
        assert!(sink.logs.iter().any(|line| line.contains("not a JSON-RPC message")));
        assert!(sink.frames.iter().any(|frame| frame["id"] == 5), "the ping after it was served");
    }

    #[test]
    fn a_response_to_an_unknown_request_id_is_ignored_not_crashed_on() {
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        let mut source = ScriptedSource::new(vec![Inbound::Response {
            id: json!(99),
            outcome: Ok(json!({})),
        }]);
        session.run(&mut source, &mut sink);
        assert!(!session.is_ready());
        assert!(sink.logs.iter().any(|line| line.contains("unknown request id")));
    }

    #[test]
    fn interaction_traffic_never_answers_itself() {
        // The failure this prevents: answering for an interaction the user never saw,
        // which would grant an escalation with no human in the loop. *Receiving* one
        // is not answering one, so cards appear and nothing goes out.
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        let mut source = ScriptedSource::new(vec![
            notification("interaction/open", json!({"interactionId": "q", "kind": "approval", "payload": {}})),
            notification("interaction/open", json!({"interactionId": "w", "kind": "question", "payload": {}})),
        ]);
        session.run(&mut source, &mut sink);
        assert_eq!(sink.frames.len(), 1, "only the handshake went out");
        assert_eq!(session.interactions().len(), 2, "both were kept for the user to see");
        // Breadcrumbed before any window exists, which is exactly the situation in
        // which "did the approval reach the panel at all?" gets asked.
        assert_eq!(sink.marks.len(), 2);
    }

    #[test]
    fn ready_carries_the_effective_configuration() {
        // The host's answer to `hello` says what it accepted; `ready` says what it
        // wants this process to look like. Reading it is what makes the window the
        // configured size on its first frame rather than after a resize.
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        let mut source = ScriptedSource::new(vec![notification(
            "ready",
            json!({"protocol": "quorfloat/1", "config": {
                "window": {"width": 480, "maxHeight": 300},
                "hotkey": "Alt+Space",
                "heartbeatMs": 5000,
            }}),
        )]);
        session.run(&mut source, &mut sink);
        let config = session.host_config();
        assert_eq!(config.window.as_ref().unwrap()["width"], 480);
        assert_eq!(config.hotkey.as_deref(), Some("Alt+Space"));
        assert_eq!(config.heartbeat_ms, Some(5000));
    }

    #[test]
    fn a_config_push_changes_only_the_fields_it_carries() {
        // A partial push must not wipe the fields it omits, or a later heartbeat
        // interval would silently reset the window geometry to nothing.
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        let mut source = ScriptedSource::new(vec![
            notification("ready", json!({"config": {"window": {"width": 480}, "heartbeatMs": 5000}})),
            notification("host/config", json!({"hotkey": "Cmd+Shift+K"})),
        ]);
        session.run(&mut source, &mut sink);
        let config = session.host_config();
        assert_eq!(config.hotkey.as_deref(), Some("Cmd+Shift+K"));
        assert_eq!(config.window.as_ref().unwrap()["width"], 480, "the window config survived");
        assert_eq!(config.heartbeat_ms, Some(5000), "and so did the heartbeat interval");
    }

    #[test]
    fn the_host_can_ask_for_the_window_to_change() {
        // The missing direction: the peer reports where its window is, this is the
        // host asking for it to move. Accepting both spellings matters because the
        // host's router validates `{visible}` while a command object is the more
        // natural thing for a host surface to send.
        assert_eq!(WindowCommand::parse(Some(&json!({"visible": true}))), Some(WindowCommand::Show));
        assert_eq!(WindowCommand::parse(Some(&json!({"visible": false}))), Some(WindowCommand::Hide));
        assert_eq!(WindowCommand::parse(Some(&json!("show"))), Some(WindowCommand::Show));
        assert_eq!(WindowCommand::parse(Some(&json!("toggle"))), Some(WindowCommand::Toggle));
    }

    #[test]
    fn an_unrecognised_window_payload_is_ignored_rather_than_fatal() {
        // A future host may extend this payload; an older peer must keep running
        // rather than drop the channel over a field it does not know.
        assert_eq!(WindowCommand::parse(Some(&json!({"opacity": 0.5}))), None);
        assert_eq!(WindowCommand::parse(None), None);
    }

    #[test]
    fn window_commands_are_queued_and_drained_in_order() {
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        let mut source = ScriptedSource::new(vec![
            notification("window/visibility", json!({"visible": true})),
            notification("window/visibility", json!({"visible": false})),
            notification("window/visibility", json!({"visible": true})),
        ]);
        session.run(&mut source, &mut sink);
        assert_eq!(
            session.take_window_commands(),
            vec![WindowCommand::Show, WindowCommand::Hide, WindowCommand::Show],
        );
        assert!(session.take_window_commands().is_empty(), "draining leaves nothing behind");
    }

    #[test]
    fn panel_visibility_is_reported_as_a_notification() {
        // A notification, not a request: the host handles it in its notification
        // path and the peer has nothing to wait for. It must also be sent when the
        // panel is hidden, since "hidden" is a fact the host routes on.
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        session.report_visibility(false, &["window", "hotkey"], &mut sink);
        session.report_visibility(true, &["window", "hotkey"], &mut sink);
        assert_eq!(sink.frames.len(), 2);
        assert_eq!(sink.frames[0]["method"], "window/visibility");
        assert_eq!(sink.frames[0]["params"]["visible"], false);
        assert!(sink.frames[0].get("id").is_none(), "a notification carries no id");
        assert_eq!(sink.frames[1]["params"]["visible"], true);
    }

    #[test]
    fn the_report_carries_what_the_process_can_actually_do() {
        // `hello` is written before a window exists, so a capability there is a
        // claim. This is the measurement, and it rides along with a notification the
        // host already parses — one field it ignores, no new method.
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        session.report_visibility(false, &["window", "hotkey"], &mut sink);
        assert_eq!(sink.frames[0]["params"]["capabilities"], json!(["window", "hotkey"]));
    }

    /// A snapshot carrying one user message and one assistant message.
    fn conversation_snapshot() -> serde_json::Value {
        json!({
            "sessionId": "session-1", "generation": 1, "cursor": 1, "hasMore": false,
            "records": [
                // The two wrappers are the real asymmetry: `user/message` carries its
                // content directly, the assistant's is inside `message`.
                {"type": "event", "event": {"type": "user/message", "seq": 0, "time": 1,
                 "data": {"role": "user", "content": [{"type": "text", "text": "帮我看看"}]}}},
                {"type": "event", "event": {"type": "assistant/message", "seq": 1, "time": 2,
                 "data": {"message": {"role": "assistant",
                    "content": [{"type": "text", "text": "看到了"}]}}}},
            ],
        })
    }

    #[test]
    fn the_conversation_notifications_reach_the_transcript() {
        // The wiring, not the model: `transcript.rs` tests what a snapshot means, this
        // one tests that a snapshot notification is actually fed to it.
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        let mut source = ScriptedSource::new(vec![notification("session/snapshot", conversation_snapshot())]);
        session.run(&mut source, &mut sink);

        let entries = session.transcript().entries();
        assert_eq!(entries.len(), 2, "{entries:?}");
        assert_eq!(entries[0], crate::transcript::Entry::User { text: "帮我看看".to_owned() });
        assert_eq!(session.transcript().session_id(), Some("session-1"));
    }

    #[test]
    fn an_event_arriving_after_the_snapshot_is_appended() {
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        let mut source = ScriptedSource::new(vec![
            notification("session/snapshot", conversation_snapshot()),
            notification("session/event", json!({
                "sessionId": "session-1", "generation": 1, "seq": 2, "type": "user/message", "time": 3,
                "data": {"role": "user", "content": [{"type": "text", "text": "还有一件事"}]},
            })),
        ]);
        session.run(&mut source, &mut sink);

        let entries = session.transcript().entries();
        assert_eq!(entries.len(), 3, "{entries:?}");
        assert_eq!(entries[2], crate::transcript::Entry::User { text: "还有一件事".to_owned() });
    }

    #[test]
    fn a_reported_gap_is_recorded_and_repaired_by_re_subscribing() {
        // A resync that only wrote a log line would leave the panel showing a
        // conversation with a hole in it and no way to tell.
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        let mut source = ScriptedSource::new(vec![
            notification("session/snapshot", conversation_snapshot()),
            notification("session/resync", json!({"sessionId": "session-1", "generation": 1,
                "reason": "sequence-gap", "expected": 2, "received": 5})),
        ]);
        session.run(&mut source, &mut sink);

        let attached = sink
            .frames
            .iter()
            .filter(|frame| frame["method"] == "session/attach")
            .count();
        assert_eq!(attached, 1, "the gap is repaired by asking again: {:?}", sink.frames);
        assert!(
            sink.marks.iter().any(|mark| mark.contains("resync")),
            "and it is on the record: {:?}",
            sink.marks,
        );
        assert!(session.transcript().entries().iter().any(|entry| matches!(
            entry,
            crate::transcript::Entry::Notice { kind, .. } if kind == "session/resync"
        )));
    }

    #[test]
    fn a_hint_is_recorded_as_a_hand_off_rather_than_an_error() {
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        let mut source = ScriptedSource::new(vec![notification(
            "interaction/hint",
            json!({"kind": "question", "reason": "harness-not-visible", "surfaces": ["desktop"]}),
        )]);
        session.run(&mut source, &mut sink);
        let handoff = session.handoff().expect("the hint is remembered");
        assert_eq!(handoff.kind(), InteractionKind::Question);
        assert_eq!(handoff.reason(), "harness-not-visible");
        assert_eq!(handoff.surfaces(), ["desktop"], "the live surfaces decide what the banner may claim");
        // Advisory: nothing is sent back, and no card is created. A hint has no id
        // and answering one is refused by the host.
        assert!(session.interactions().is_empty());
        assert_eq!(sink.frames.len(), 1, "only the handshake went out");
        assert_eq!(sink.marks, vec!["handoff question harness-not-visible surfaces=1".to_owned()]);
    }

    #[test]
    fn a_hand_off_without_a_live_surface_says_so() {
        // The difference between "go and answer it there" and "nothing can answer it"
        // is one field, and the panel must not have to guess which sentence is true.
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        let mut source = ScriptedSource::new(vec![
            notification("interaction/hint", json!({"kind": "question", "reason": "harness-not-visible"})),
            // A payload whose surface list is not a list of strings is reported as
            // empty rather than as a surface nobody can name.
            notification(
                "interaction/hint",
                json!({"kind": "approval", "reason": "nobody-looking", "surfaces": [7, "web"]}),
            ),
        ]);
        session.run(&mut source, &mut sink);
        let handoff = session.handoff().expect("the second hint replaced the first");
        assert_eq!(handoff.kind(), InteractionKind::Approval);
        assert_eq!(handoff.surfaces(), ["web"]);
        assert!(
            sink.marks.iter().any(|line| line.ends_with("surfaces=0")),
            "the first hint recorded no live surface: {:?}",
            sink.marks,
        );
    }

    #[test]
    fn a_hint_with_an_unknown_kind_is_ignored_rather_than_guessed_at() {
        // The kind decides which sentence the user reads. Guessing it would tell them
        // to confirm an approval that does not exist, or to go and type an answer to
        // a question nobody asked.
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        let mut source = ScriptedSource::new(vec![notification(
            "interaction/hint",
            json!({"kind": "telepathy", "reason": "harness-visible"}),
        )]);
        session.run(&mut source, &mut sink);
        assert!(session.handoff().is_none());
        assert!(sink.logs.join("\n").contains("unknown kind"));
    }

    #[test]
    fn a_hand_off_can_be_dismissed() {
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        session
            .on_frame(notification("interaction/hint", json!({"kind": "approval", "reason": "harness-open-but-idle"})), &mut sink);
        assert!(session.handoff().is_some());
        session.dismiss_handoff();
        assert!(session.handoff().is_none());
    }

    /// The host's `interaction/open` for one approval.
    fn approval_open(id: &str) -> Inbound {
        notification(
            "interaction/open",
            json!({
                "interactionId": id,
                "sessionId": "session-1",
                "kind": "approval",
                "payload": {
                    "toolName": "bash",
                    "callId": "call-1",
                    "reason": "escalate sandbox to danger-full-access: write the file",
                },
            }),
        )
    }

    /// Deliver frames without the handshake, which these tests do not exercise.
    fn deliver(session: &mut Session, frames: Vec<Inbound>, sink: &mut RecordingSink) {
        for frame in frames {
            session.on_frame(frame, sink);
        }
    }

    #[test]
    fn an_approval_becomes_a_card_that_can_be_answered() {
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        deliver(&mut session, vec![approval_open("approval-1")], &mut sink);

        let cards = session.interactions();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].id(), "approval-1");
        assert_eq!(cards[0].session_id(), "session-1");
        assert_eq!(cards[0].kind(), InteractionKind::Approval);
        assert_eq!(cards[0].tool_name(), Some("bash"));
        assert!(cards[0].is_actionable());
        assert_eq!(sink.marks, vec!["interaction approval-1 approval arrived".to_owned()]);

        assert!(session.answer_interaction("approval-1", ApprovalVerdict::AllowOnce, &mut sink));
        let answer = sink.frames.last().expect("an answer was sent");
        assert_eq!(answer["method"], "interaction/answer");
        assert_eq!(answer["params"]["interactionId"], "approval-1");
        assert_eq!(answer["params"]["answer"]["outcome"], "allowed-once");
        assert!(answer["id"].is_number(), "an answer is a request, so it carries an id");
        assert!(sink.marks.iter().any(|line| line == "interaction approval-1 answering allowed-once"));
    }

    #[test]
    fn one_decision_is_sent_once() {
        // The grant is one-shot: a second send would be a second decision for one
        // question, and the host refuses it anyway. Refusing it here keeps the two
        // ends from disagreeing about what the user did.
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        deliver(&mut session, vec![approval_open("approval-1")], &mut sink);
        assert!(session.answer_interaction("approval-1", ApprovalVerdict::Reject, &mut sink));
        let sent = sink.frames.len();
        assert!(!session.answer_interaction("approval-1", ApprovalVerdict::AllowOnce, &mut sink));
        assert_eq!(sink.frames.len(), sent, "no second answer went out");
        assert!(sink.logs.join("\n").contains("already in flight"));
    }

    #[test]
    fn answering_something_this_panel_does_not_hold_is_refused() {
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        assert!(!session.answer_interaction("approval-9", ApprovalVerdict::AllowOnce, &mut sink));
        assert!(sink.frames.is_empty());
        assert!(sink.logs.join("\n").contains("not holding it"));
    }

    #[test]
    fn a_re_published_interaction_does_not_become_a_second_card() {
        // The host re-delivers what is still pending after a restart. Two cards for
        // one request would show the user two buttons for one decision, and the second
        // would be refused by the host anyway.
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        deliver(&mut session, vec![approval_open("approval-1"), approval_open("approval-1")], &mut sink);
        assert_eq!(session.interactions().len(), 1);
        assert!(sink.logs.join("\n").contains("already holding it"));
        assert_eq!(sink.marks.len(), 1, "one arrival, one breadcrumb");
    }

    #[test]
    fn an_unreadable_open_is_refused_rather_than_half_renderable() {
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        deliver(
            &mut session,
            vec![
                notification("interaction/open", json!({"kind": "approval"})),
                notification("interaction/open", json!({"interactionId": "a-2", "kind": "telepathy"})),
                Inbound::Notification { method: "interaction/open".to_owned(), params: None },
            ],
            &mut sink,
        );
        assert!(session.interactions().is_empty());
        let logs = sink.logs.join("\n");
        assert!(logs.contains("without an interactionId"), "{logs}");
        assert!(logs.contains("unknown kind"), "{logs}");
        assert!(logs.contains("without parameters"), "{logs}");
    }

    #[test]
    fn the_hosts_acceptance_settles_the_card() {
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        deliver(&mut session, vec![approval_open("approval-1")], &mut sink);
        session.answer_interaction("approval-1", ApprovalVerdict::AllowOnce, &mut sink);
        let id = sink.frames.last().expect("the answer")["id"].clone();
        session.on_frame(
            Inbound::Response { id, outcome: Ok(json!({"accepted": true})) },
            &mut sink,
        );
        assert_eq!(
            session.interactions()[0].state(),
            &InteractionState::Applied { verdict: ApprovalVerdict::AllowOnce }
        );
        assert!(!session.interactions()[0].is_actionable());
        assert!(sink.marks.iter().any(|line| line == "interaction approval-1 applied allowed-once"));
    }

    #[test]
    fn a_refusal_keeps_the_hosts_reason_and_cannot_be_retried() {
        // `accepted:false` is a normal answer, not a failure: another surface answered
        // first, or the claim expired. The user is owed the reason, and a retry would
        // be a guess — the request is gone upstream.
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        deliver(&mut session, vec![approval_open("approval-1")], &mut sink);
        session.answer_interaction("approval-1", ApprovalVerdict::Reject, &mut sink);
        let id = sink.frames.last().expect("the answer")["id"].clone();
        session.on_frame(
            Inbound::Response {
                id,
                outcome: Ok(json!({"accepted": false, "reason": "another surface answered first"})),
            },
            &mut sink,
        );
        assert_eq!(
            session.interactions()[0].state(),
            &InteractionState::Refused { reason: "another surface answered first".to_owned() }
        );
        let sent = sink.frames.len();
        assert!(!session.answer_interaction("approval-1", ApprovalVerdict::AllowOnce, &mut sink));
        assert_eq!(sink.frames.len(), sent);
        assert!(sink.logs.join("\n").contains("already resolved"));
    }

    #[test]
    fn a_refusal_without_a_reason_still_says_something() {
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        deliver(&mut session, vec![approval_open("approval-1")], &mut sink);
        session.answer_interaction("approval-1", ApprovalVerdict::AllowOnce, &mut sink);
        let id = sink.frames.last().expect("the answer")["id"].clone();
        session.on_frame(Inbound::Response { id, outcome: Ok(json!({"accepted": false})) }, &mut sink);
        assert_eq!(
            session.interactions()[0].state(),
            &InteractionState::Refused { reason: "the host did not say why".to_owned() }
        );
    }

    #[test]
    fn an_error_response_also_settles_the_card() {
        // An older host answers `unknown_method`, or the handler throws. Either way the
        // card must stop claiming an answer is on its way.
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        deliver(&mut session, vec![approval_open("approval-1")], &mut sink);
        session.answer_interaction("approval-1", ApprovalVerdict::AllowOnce, &mut sink);
        let id = sink.frames.last().expect("the answer")["id"].clone();
        session.on_frame(
            Inbound::Response {
                id,
                outcome: Err(RpcError {
                    code: error_code::METHOD_NOT_FOUND,
                    message: "unsupported method: interaction/answer".to_owned(),
                    data: None,
                }),
            },
            &mut sink,
        );
        let InteractionState::Refused { reason } = session.interactions()[0].state() else {
            panic!("expected a refusal, got {:?}", session.interactions()[0].state());
        };
        assert!(reason.contains("-32601"), "{reason}");
        assert!(reason.contains("unsupported method"), "{reason}");
    }

    #[test]
    fn the_verdict_of_one_answer_cannot_settle_another() {
        // Two requests in flight at once, answered in the opposite order to the one
        // they were sent in. Attributing by request id is the only thing that keeps
        // "allowed" from landing on the card the user rejected.
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        deliver(&mut session, vec![approval_open("approval-1"), approval_open("approval-2")], &mut sink);
        session.answer_interaction("approval-1", ApprovalVerdict::AllowOnce, &mut sink);
        session.answer_interaction("approval-2", ApprovalVerdict::Reject, &mut sink);
        let first = sink.frames[sink.frames.len() - 2]["id"].clone();
        let second = sink.frames[sink.frames.len() - 1]["id"].clone();
        session.on_frame(Inbound::Response { id: second, outcome: Ok(json!({"accepted": true})) }, &mut sink);
        session.on_frame(Inbound::Response { id: first, outcome: Ok(json!({"accepted": true})) }, &mut sink);
        assert_eq!(
            session.interactions()[0].state(),
            &InteractionState::Applied { verdict: ApprovalVerdict::AllowOnce }
        );
        assert_eq!(
            session.interactions()[1].state(),
            &InteractionState::Applied { verdict: ApprovalVerdict::Reject }
        );
    }

    #[test]
    fn an_answer_that_could_not_be_written_leaves_the_card_actionable() {
        // The channel is broken, so the session is ending anyway — but the card must
        // not sit on "sending…" claiming a send that never happened.
        struct DeafSink {
            marks: Vec<String>,
        }

        impl FrameSink for DeafSink {
            fn send(&mut self, _frame: &Value) -> std::io::Result<()> {
                Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "gone"))
            }

            fn log(&mut self, _line: &str) {}

            fn mark(&mut self, line: &str) {
                self.marks.push(line.to_owned());
            }
        }

        let mut session = Session::new(identity());
        let mut recording = RecordingSink::default();
        deliver(&mut session, vec![approval_open("approval-1")], &mut recording);
        let mut sink = DeafSink { marks: Vec::new() };
        assert!(!session.answer_interaction("approval-1", ApprovalVerdict::AllowOnce, &mut sink));
        assert_eq!(session.interactions()[0].state(), &InteractionState::Pending);
        assert!(session.interactions()[0].is_actionable());
        assert_eq!(sink.marks, vec!["interaction approval-1 answering allowed-once".to_owned()]);
        // And the id is free again, so a retry after the channel recovers is a fresh
        // request rather than one the session thinks is still in flight.
        let mut recovered = RecordingSink::default();
        assert!(session.answer_interaction("approval-1", ApprovalVerdict::AllowOnce, &mut recovered));
    }

    #[test]
    fn a_question_is_reported_but_not_answerable_by_this_build() {
        // The answer shape is a list of selected option ids. This build has no widget
        // for that, and inventing an answer would tell the model the user chose
        // something they never saw, so the refusal is deliberate.
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        deliver(
            &mut session,
            vec![notification(
                "interaction/open",
                json!({
                    "interactionId": "question-1",
                    "sessionId": "session-1",
                    "kind": "question",
                    "payload": {"questions": [
                        {"id": "q1", "header": "部署目标", "question": "部署到哪个环境？",
                         "options": [{"label": "staging"}, {"label": "production"}]},
                        {"id": "q2", "question": "需要通知谁？"},
                    ]},
                }),
            )],
            &mut sink,
        );
        let card = &session.interactions()[0];
        assert_eq!(card.kind(), InteractionKind::Question);
        assert!(!card.is_actionable());
        assert_eq!(card.question_count(), 2);
        assert_eq!(
            card.questions(),
            vec![
                (Some("部署目标".to_owned()), "部署到哪个环境？".to_owned()),
                (None, "需要通知谁？".to_owned()),
            ]
        );
        assert!(!session.answer_interaction("question-1", ApprovalVerdict::AllowOnce, &mut sink));
        assert!(sink.logs.join("\n").contains("cannot answer a question"));
        assert!(session.dismiss_interaction("question-1"));
        assert!(session.interactions().is_empty());
    }

    #[test]
    fn the_detail_prefers_the_text_written_to_be_read() {
        // Preference order matters: `reason` is an audit string ("escalate sandbox to
        // …"), while `displayReason` is what the asker wrote *for a human to read*.
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        deliver(
            &mut session,
            vec![notification(
                "interaction/open",
                json!({
                    "interactionId": "a-1", "sessionId": "s", "kind": "approval",
                    "payload": {
                        "toolName": "bash",
                        "reason": "escalate sandbox to danger-full-access: justification",
                        "displayReason": {"en": "Allow this with danger-full-access?", "zh": "允许使用 danger-full-access 权限？"},
                    },
                }),
            )],
            &mut sink,
        );
        let card = &session.interactions()[0];
        assert_eq!(card.detail("zh").as_deref(), Some("允许使用 danger-full-access 权限？"));
        assert_eq!(card.detail("fr").as_deref(), Some("Allow this with danger-full-access?"));
    }

    #[test]
    fn the_detail_falls_back_to_the_audit_reason_and_survives_junk() {
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        deliver(
            &mut session,
            vec![
                notification(
                    "interaction/open",
                    json!({"interactionId": "a-1", "kind": "approval",
                           "payload": {"reason": "escalate sandbox to workspace-write: write a file"}}),
                ),
                notification(
                    "interaction/open",
                    json!({"interactionId": "a-2", "kind": "approval",
                           "payload": {"displayReason": "not an object", "reason": 7}}),
                ),
                notification(
                    "interaction/open",
                    json!({"interactionId": "a-3", "kind": "approval",
                           "payload": {"displayReason": {"zh": "   "}, "reason": "  "}}),
                ),
            ],
            &mut sink,
        );
        let cards = session.interactions();
        assert_eq!(cards[0].detail("zh").as_deref(), Some("escalate sandbox to workspace-write: write a file"));
        assert_eq!(cards[1].detail("zh"), None, "a non-object displayReason is not a string to show");
        assert_eq!(cards[2].detail("zh"), None, "whitespace is not a reason");
        assert_eq!(cards[0].tool_name(), None);
    }

    #[test]
    fn peer_text_is_bounded_before_it_reaches_the_window() {
        // A frame may be a megabyte and the text comes from a model or a hook. The
        // bound belongs here rather than in the layout code, so no path can hand the
        // window an unbounded string to lay out.
        let long_reason = "x".repeat(5000);
        let long_question = "问".repeat(5000);
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        deliver(
            &mut session,
            vec![
                notification(
                    "interaction/open",
                    json!({"interactionId": "a-1", "kind": "approval", "payload": {"reason": long_reason}}),
                ),
                notification(
                    "interaction/open",
                    json!({"interactionId": "q-1", "kind": "question",
                           "payload": {"questions": [{"id": "q1", "question": long_question}]}}),
                ),
            ],
            &mut sink,
        );
        let cards = session.interactions();
        let detail = cards[0].detail("zh").expect("a reason");
        assert_eq!(detail.chars().count(), MAX_TEXT_CHARS + 1, "600 characters plus the ellipsis");
        assert!(detail.ends_with('…'));
        let question = &cards[1].questions()[0].1;
        assert_eq!(question.chars().count(), MAX_TEXT_CHARS + 1);
        assert!(question.ends_with('…'), "a multi-byte cut lands on a character boundary");
    }

    #[test]
    fn only_the_first_few_questions_are_summarised() {
        let questions: Vec<Value> = (0..8)
            .map(|index| json!({"id": format!("q{index}"), "question": format!("question {index}")}))
            .collect();
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        deliver(
            &mut session,
            vec![notification(
                "interaction/open",
                json!({"interactionId": "q-1", "kind": "question", "payload": {"questions": questions}}),
            )],
            &mut sink,
        );
        let card = &session.interactions()[0];
        assert_eq!(card.question_count(), 8, "the count is the real total");
        assert_eq!(card.questions().len(), MAX_QUESTIONS_SHOWN, "but only a few are read out");
    }

    #[test]
    fn the_pending_list_is_bounded() {
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        for index in 0..MAX_INTERACTIONS + 3 {
            deliver(&mut session, vec![approval_open(&format!("approval-{index}"))], &mut sink);
        }
        assert_eq!(session.interactions().len(), MAX_INTERACTIONS);
        assert!(sink.logs.join("\n").contains("are already pending"));
    }

    /// Take the handshake, discovery and attach through to a followed conversation.
    fn follow_a_conversation(session: &mut Session, sink: &mut RecordingSink) {
        session.on_frame(hello_ok(), sink);
        session.pump_follow_at(0, sink);
        let list = sink.frames.last().expect("a list request")["id"].clone();
        session.on_frame(
            Inbound::Response {
                id: list,
                outcome: Ok(json!({"items": [
                    {"sessionId": "older", "updatedAt": 1, "cwd": "/work/project"},
                    {"sessionId": "session-1", "updatedAt": 9, "cwd": "/work/project"},
                ]})),
            },
            sink,
        );
        let attach = sink.frames.last().expect("an attach request")["id"].clone();
        session.on_frame(
            Inbound::Response { id: attach, outcome: Ok(json!({"sessionId": "session-1", "generation": 3})) },
            sink,
        );
    }

    #[test]
    fn conversation_discovery_starts_only_after_the_handshake() {
        // The host refuses session calls on a channel that has not said `hello`, so
        // asking early would be a failure the panel then had to retry through.
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        session.pump_follow_at(0, &mut sink);
        assert!(sink.frames.is_empty(), "nothing is asked before the handshake");
        session.on_frame(hello_ok(), &mut sink);
        session.pump_follow_at(1_000, &mut sink);
        assert_eq!(sink.frames.last().expect("a list request")["method"], "sessions/list");
    }

    #[test]
    fn the_newest_conversation_is_attached_with_a_fresh_request_id() {
        // This is the step that decides whether an approval can reach the panel at
        // all: with nothing attached, every interaction is deferred before the panel
        // ever hears about it.
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        session.on_frame(hello_ok(), &mut sink);
        session.pump_follow_at(0, &mut sink);
        let list_id = sink.frames.last().expect("a list request")["id"].clone();
        session.on_frame(
            Inbound::Response {
                id: list_id.clone(),
                outcome: Ok(json!({"items": [
                    {"sessionId": "older", "updatedAt": 1},
                    {"sessionId": "newest", "updatedAt": 9},
                ]})),
            },
            &mut sink,
        );
        let attach = sink.frames.last().expect("an attach request");
        assert_eq!(attach["method"], "session/attach");
        assert_eq!(attach["params"]["sessionId"], "newest");
        assert_ne!(attach["id"], list_id, "the attach is a second request, not a repeat of the first");

        let attach_id = attach["id"].clone();
        session.on_frame(
            Inbound::Response { id: attach_id, outcome: Ok(json!({"sessionId": "newest", "generation": 2})) },
            &mut sink,
        );
        assert_eq!(session.follow().session_id(), Some("newest"));
        assert_eq!(session.follow().generation(), Some(2));
        assert!(
            sink.marks.iter().any(|line| line == "follow newest generation=2"),
            "the fact that decides ownership is recorded: {:?}",
            sink.marks,
        );
    }

    #[test]
    fn a_lost_answer_is_retried_rather_than_stopping_discovery() {
        // One dropped response must not leave the panel owning nothing for the rest of
        // the session — a state that looks exactly like having nothing to do.
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        session.on_frame(hello_ok(), &mut sink);
        session.pump_follow_at(0, &mut sink);
        let sent = sink.frames.len();
        session.pump_follow_at(1_000, &mut sink);
        assert_eq!(sink.frames.len(), sent, "the first answer is still owed");
        session.pump_follow_at(30_000, &mut sink);
        assert_eq!(sink.frames.len(), sent, "past the timeout the request is dropped, not repeated");
        assert!(sink.logs.join("\n").contains("it will be retried"));
        session.pump_follow_at(34_000, &mut sink);
        assert_eq!(sink.frames.len(), sent + 1, "and asked again on the next interval");
    }

    #[test]
    fn traffic_for_the_followed_conversation_is_counted() {
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        follow_a_conversation(&mut session, &mut sink);
        deliver(
            &mut session,
            vec![
                notification(
                    "session/snapshot",
                    json!({"sessionId": "session-1", "generation": 3, "cursor": 40, "records": [{}]}),
                ),
                notification("session/event", json!({"sessionId": "session-1", "generation": 3, "seq": 41})),
                notification("session/event", json!({"sessionId": "older", "generation": 1, "seq": 2})),
            ],
            &mut sink,
        );
        assert_eq!(session.follow().events(), 1, "only the followed conversation counts");
        assert!(
            !sink.logs.join("
").contains("not use"),
            "session traffic is handled, not reported as an unknown notification",
        );
    }

    #[test]
    fn dismissing_an_unknown_card_reports_that_nothing_happened() {
        let mut session = Session::new(identity());
        assert!(!session.dismiss_interaction("approval-9"));
    }
}

//! The session: handshake, liveness, and shutdown.
//!
//! This module owns the *state* of a connection to the Harness host and knows
//! nothing about processes or sockets. Frames arrive through [`FrameSource`] and
//! leave through [`FrameSink`], both of which are small enough to implement with
//! a vector in a test. That is deliberate: the handshake order and the answers to
//! liveness traffic are the parts most likely to be wrong, and they must be
//! verifiable without a host, a model, or a spawned process.
//!
//! Two rules shape everything here:
//!
//! - **stdout carries protocol frames and nothing else.** Log lines go to the
//!   sink's `log`, which is stderr. A stray `println!` would corrupt the stream
//!   the host is parsing, so no code in this crate writes to stdout directly.
//! - **The host judges liveness by `ping`, not by `host/heartbeat`.** A peer that
//!   forwarded heartbeats but never answered `ping` would be declared dead while
//!   happily talking, so `ping` is answered before anything else is considered.

use serde_json::{Value, json};
use std::collections::BTreeMap;

use crate::app::session::follow::{Follow, Outgoing, PanelLifecycle};
use crate::app::session::interaction::{
    ApprovalVerdict, Handoff, Interaction, InteractionKind, InteractionState, MAX_INTERACTIONS,
    QuestionAnswer, SentAnswer, validate_answers,
};
use crate::app::session::transcript::Transcript;

/// Every prompt key this process has issued, counted once.
///
/// Not per session, and never reset: it is the part of a key that makes it new even when the clock has
/// not moved and the process id is the same — see [`Session::prompt_key`] for what goes wrong when a key
/// repeats.
static PROMPT_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Which outbound request a response settles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sending {
    /// A `session/prompt`.
    Prompt,
    /// A `session/cancel`.
    Cancel,
}

/// What became of an outbound request.
///
/// `Accepted` is the host's own word: it means the request was taken, not that the turn
/// is over. The composer shows it for exactly that reason — "已提交" and "已完成" are
/// different claims, and only the conversation can make the second one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    /// Nothing has been sent.
    Idle,
    /// Sent, and the host has not answered yet.
    Sending,
    /// The host took it.
    Accepted,
    /// The host refused it, or it could not be written. The reason is shown as it came.
    Failed {
        /// Why, in the host's words when it came from the host.
        reason: String,
    },
}

impl Delivery {
    /// Whether a request is awaiting a verdict.
    #[must_use]
    pub fn is_sending(&self) -> bool {
        matches!(self, Self::Sending)
    }

    /// One line for the composer, or `None` when there is nothing to say.
    #[must_use]
    pub fn describe(&self) -> Option<String> {
        match self {
            Self::Idle | Self::Accepted => None,
            Self::Sending => Some("已提交，等待 Harness 确认…".to_owned()),
            Self::Failed { reason } => Some(format!("未发送：{reason}")),
        }
    }
}
use crate::ipc::protocol::{CAPABILITIES, PROTOCOL_VERSION, error_code};
use crate::ipc::rpc::{self, Inbound, Outcome, RpcError, Router};

pub mod follow;
pub mod interaction;
/// Shared by the session's own tests: the fake host lives here.
#[cfg(test)]
pub(crate) mod test_support;
pub mod transcript;

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

/// Why a conversation cannot be started: the panel does not know where to put one.
///
/// The host's router requires a non-empty `workspaceId`, so this is not a limitation the panel
/// can work around by asking more cleverly — it is a question only the user can answer.
pub const CREATE_WITHOUT_WORKSPACE: &str = "先选择一个工作区";

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
    /// What became of the last prompt, and of the last stop request.
    ///
    /// Kept because a send has two outcomes and only one of them is visible: the user
    /// types, the composer clears, and whether the host *took* it is a separate fact
    /// that has to be shown rather than assumed.
    prompt: Delivery,
    cancel: Delivery,
    /// Requests awaiting a verdict, so a response can be attributed to what asked.
    sends_in_flight: BTreeMap<i64, Sending>,
    /// A prompt that cannot be sent until the conversation it needs exists.
    ///
    /// Held here rather than in the panel because only this layer knows when the creation has
    /// finished: it finishes as an attach, and an attach is done when its answer arrives.
    pending_prompt: Option<String>,
    /// The id and send time of the outstanding follow request, if any.
    follow_request: Option<(i64, i64)>,
    /// Requests the *user* caused, by their id: options, model changes, permission changes.
    ///
    /// Deliberately not the `follow_request` slot: that one belongs to the polling loop, where one
    /// request outstanding at a time is the discipline. These are a person clicking, and a click
    /// while the loop happens to be mid-poll must not be lost or answer for the wrong request.
    setting_requests: BTreeMap<i64, SettingRequest>,
    /// What the host last said may be chosen, and what is chosen now.
    options: Option<SessionOptions>,
    /// The last failure of a setting change, for the panel to show.
    setting_failure: Option<String>,
    /// The conversation's statistics, as the host last reported them.
    stats: Option<Stats>,
    /// Which conversation those statistics describe. `stats` outlives a detach (nothing else
    /// needs to forget it), so the pair is what makes [`Session::stats`] answerable.
    stats_session: Option<String>,
    next_request_id: i64,
    handshake_sent: bool,
    handshake_done: bool,
}

/// What the user asked the host to change about a session.
///
/// Kept per request so an answer can be matched to the question: a response is a bare JSON value,
/// and "a model was selected" and "a permission was selected" have different shapes and different
/// consequences for what the panel shows.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SettingRequest {
    /// `session/options`: the catalogs.
    Options { session_id: Option<String> },
    /// `session/select`: model and reasoning effort.
    SelectModel,
    /// `session/permission`: the permission preset.
    SetPermission(String),
}

/// One model the host offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelOption {
    /// Provider route, sent back with a choice.
    pub provider: String,
    /// Host-provided display name for the provider group.
    pub provider_name: String,
    /// The provider's own model id.
    pub id: String,
    /// The label a person reads.
    pub name: String,
    /// Description supplied by Harness, never a local preset table.
    pub description: Option<String>,
    /// The efforts this model accepts, in the host's order.
    pub efforts: Vec<EffortOption>,
    /// The effort the host defaults to, when it names one.
    pub default_effort: Option<String>,
}

/// One reasoning effort a model accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffortOption {
    /// Stable value sent back.
    pub id: String,
    /// The label a person reads.
    pub name: String,
    /// Description supplied by Harness, never a local preset table.
    pub description: Option<String>,
}

/// One permission preset the host offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionOption {
    /// Stable value sent back.
    pub value: String,
    /// The label a person reads.
    pub name: String,
    /// Description supplied by Harness, never a local preset table.
    pub description: Option<String>,
}

/// What the host says may be chosen for the attached conversation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionOptions {
    /// Every model, in the host's order. Flattened: the panel shows one list, and the provider is
    /// carried on each entry rather than being a heading to navigate.
    pub models: Vec<ModelOption>,
    /// The model in effect, as `(provider, id)`.
    pub current_model: Option<(String, String)>,
    /// The reasoning effort in effect, when one is.
    pub current_effort: Option<String>,
    /// The permission presets offered.
    pub permissions: Vec<PermissionOption>,
    /// The permission preset in effect.
    pub current_permission: Option<String>,
}

/// A conversation's statistics, as the host reports them.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Stats {
    /// Turns with at least one closed step.
    pub turns: u64,
    /// Closed steps.
    pub steps: u64,
    /// Output tokens per second, when it can be computed.
    pub tokens_per_second: Option<f64>,
    /// Every token the session has been billed for.
    pub total_tokens: Option<u64>,
    /// Cache-read share of the prompt, as a percentage.
    ///
    /// A float because a share that would round to a full 100 is reported as `99.9` instead:
    /// telling somebody their whole prompt was cached when a sliver was not is the one error this
    /// figure must not make (see `readCacheHitPercent` in `src/harness/adapter.ts`).
    pub cache_hit_percent: Option<f64>,
    /// Prompt tokens of the most recent request.
    pub context_tokens: Option<u64>,
    /// The model's context capacity.
    pub context_limit: Option<u64>,
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
            prompt: Delivery::Idle,
            cancel: Delivery::Idle,
            sends_in_flight: BTreeMap::new(),
            pending_prompt: None,
            follow_request: None,
            setting_requests: BTreeMap::new(),
            options: None,
            setting_failure: None,
            stats: None,
            stats_session: None,
            next_request_id: 1,
            handshake_sent: false,
            handshake_done: false,
        }
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
            "session/stats" => match params.as_ref() {
                Some(params) => self.note_stats(params, sink),
                None => sink.log("session/stats carried no params"),
            },
            // The host's push of the Harness's own workspace stream. A workspace added,
            // renamed, removed or reordered in the Harness window reaches the list the picker
            // draws here, rather than waiting for the panel to ask again — which it only ever
            // did when the list was empty or had never arrived. See
            // `Follow::apply_workspace_frame` for what each frame means.
            "workspaces/changed" => match params.as_ref() {
                Some(params) => self.follow.apply_workspace_frame(params, sink),
                None => sink.log("workspaces/changed carried no params"),
            },
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
            // Applied for every session rather than only the followed one: the point
            // is to count what the host sends, and filtering to what this build happens
            // to understand would hide the kinds it does not.
            "session/snapshot" => {
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
                if let Some(params) = params.as_ref() {
                    self.transcript.apply_event(params);
                }
                self.follow.record(method, params.as_ref(), sink);
            }
            "session/stream" => {
                if let Some(params) = params.as_ref() {
                    self.transcript.apply_stream(params);
                }
                self.follow.record(method, params.as_ref(), sink);
            }
            "session/resync" => {
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
        // Prompts and stops first, for the same reason the answers below are: a response
        // attributed to the wrong request leaves the composer claiming "sending…" forever.
        if let Some(value) = id.as_i64() {
            if self.sends_in_flight.contains_key(&value) {
                self.resolve_send(value, outcome, sink);
                return;
            }
        }
        // In-flight answers are checked before the handshake id: they are the only
        // responses this process solicits after `hello`, and a mismatch here would
        // leave a card stuck on "sending…" with nothing to explain why.
        if let Some(interaction_id) = id.as_i64().and_then(|value| self.answers_in_flight.remove(&value)) {
            self.resolve_answer(&interaction_id, outcome, sink);
            return;
        }
        let request_id = id.as_i64();
        // The user's own requests, before the polling loop's: they are answered once and a person is
        // waiting on them, while the loop simply asks again.
        if let Some(request) = request_id.and_then(|value| self.setting_requests.remove(&value)) {
            self.resolve_setting(request, outcome, sink);
            return;
        }
        if let Some((pending, _)) = self.follow_request {
            if request_id == Some(pending) {
                self.follow_request = None;
                let now = rpc::now_millis();
                let previous = (self.follow.session_id().map(str::to_owned), self.follow.generation());
                if let Some(outgoing) = self.follow.resolve(outcome, now, sink) {
                    self.send_follow(outgoing, now, sink);
                }
                if previous != (self.follow.session_id().map(str::to_owned), self.follow.generation()) {
                    self.request_options(sink);
                }
                self.forget_a_conversation_that_is_gone(sink);
                // The conversation a waiting prompt needed may have just come into existence.
                self.flush_pending_prompt(sink);
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
                // And the pickers' contents, before anything is attached: the catalogs are
                // process-wide, so asking now means the model and permission menus are filled by the
                // time a user opens one rather than showing an empty list that fills in later.
                self.request_options(sink);
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

    /// The conversations the host last listed, newest first.
    #[must_use]
    pub fn conversations(&self) -> &[crate::app::session::follow::SessionSummary] {
        self.follow.sessions()
    }

    /// The workspaces the host last listed.
    #[must_use]
    pub fn workspaces(&self) -> &[crate::app::session::follow::Workspace] {
        self.follow.workspaces()
    }

    /// Whether the workspace list has ever been asked for.
    ///
    /// The picker asks once and then stops: an empty list is an answer, not a reason to ask
    /// again every frame.
    #[must_use]
    pub fn workspaces_asked(&self) -> bool {
        self.follow.workspaces_asked()
    }

    /// Whether the workspace list has been answered at least once.
    ///
    /// The distinction the picker needs: "not arrived yet" and "there are none" are different
    /// facts, and only the second one is a reason to ask the host for a default workspace.
    #[must_use]
    pub fn workspaces_loaded(&self) -> bool {
        self.follow.workspaces_loaded()
    }

    /// What came of asking the host for its first-use default workspace.
    #[must_use]
    pub fn default_workspace(&self) -> crate::app::session::follow::DefaultWorkspaceState {
        self.follow.default_workspace()
    }

    /// The workspace pinned as the default for a new conversation, if any.
    #[must_use]
    pub fn pinned_workspace(&self) -> Option<&str> {
        self.follow.pinned_workspace()
    }

    /// Why a half of the pin was forgotten, until the user summons again.
    #[must_use]
    pub fn pin_notice(&self) -> Option<crate::app::session::follow::PinNotice> {
        self.follow.pin_notice()
    }

    /// Take the fact that a half of the pin was forgotten, so the shell can write the file.
    pub fn take_pins_changed(&mut self) -> bool {
        self.follow.take_pins_changed()
    }

    /// Start a visit: a summon is a new visit, and the last one's explanation is over.
    pub fn begin_visit(&mut self) {
        self.follow.begin_visit();
    }

    /// Pin (or unpin) the workspace a new conversation defaults to.
    ///
    /// @param workspace_id - which workspace, or `None` to stop pinning one.
    pub fn pin_workspace(&mut self, workspace_id: Option<String>) {
        self.follow.set_pinned_workspace(workspace_id);
    }

    /// The conversation the user pinned, if any.
    #[must_use]
    pub fn pinned_conversation(&self) -> Option<&str> {
        self.follow.pinned()
    }

    /// The conversation the panel is attached to, whatever the list says.
    #[must_use]
    pub fn target_conversation(&self) -> Option<&str> {
        self.follow.target()
    }

    /// Adopt a pin read from disk, without asking the host for anything.
    ///
    /// Called once at startup, before the first discovery answer arrives: the pin decides
    /// which conversation that answer attaches, so it has to be in place first.
    ///
    /// @param session_id - the pinned conversation, if one was remembered.
    /// @param workspace_id - the pinned workspace, if one was remembered. It is an independent
    ///   fact: a file may hold either half without the other, and neither is derived from the
    ///   other at adoption.
    pub fn adopt_pinned(&mut self, session_id: Option<String>, workspace_id: Option<String>) {
        // The pinned conversation is a different one from whatever was attached before, so the
        // previous one's effort and permission must not be shown as this one's.
        self.forget_settings_for_another_conversation();
        self.follow.adopt_pinned(session_id, workspace_id);
    }

    /// Open one conversation, now.
    ///
    /// @param session_id - which one.
    /// @param sink - where the request goes.
    /// @returns whether the request was written.
    pub fn choose_conversation(&mut self, session_id: &str, sink: &mut dyn FrameSink) -> bool {
        self.forget_settings_for_another_conversation();
        let now = rpc::now_millis();
        let outgoing = self.follow.choose(session_id, now);
        self.send_request(outgoing, now, sink)
    }

    /// Pin one conversation, or stop pinning any.
    ///
    /// @param session_id - which one, or `None`.
    /// @param sink - where the request goes.
    /// @returns whether the request was written, when there was one to write.
    pub fn pin_conversation(&mut self, session_id: Option<String>, sink: &mut dyn FrameSink) -> bool {
        let now = rpc::now_millis();
        let outgoing = self.follow.set_pinned(session_id, now);
        self.send_request(outgoing, now, sink)
    }

    /// Go back to "new conversation" mode, dropping whatever was pinned or chosen.
    ///
    /// @param sink - where the fact goes, because a panel that empties itself silently is
    ///   indistinguishable from one that lost its place.
    pub fn start_new_conversation(&mut self, sink: &mut dyn FrameSink) {
        // Told to the host *because of what is being left*, not because of what the transcript
        // had received: the host decides who answers approvals from the attach it was asked
        // for, and that is exactly what is being given up here.
        let leaving = self.follow.session_id().map(str::to_owned);
        self.follow.start_new();
        if let Some(session_id) = leaving {
            self.notify_detached(&session_id, sink);
            self.transcript.reset();
        }
    }

    /// Create a conversation, and be in it.
    ///
    /// @param workspace_id - where to create it, or `None` for the host's default.
    /// @param sink - where the request goes.
    /// @returns whether the request was written.
    pub fn create_conversation(&mut self, workspace_id: Option<&str>, sink: &mut dyn FrameSink) -> bool {
        let now = rpc::now_millis();
        let outgoing = self.follow.create(workspace_id, now);
        self.send_request(outgoing, now, sink)
    }

    /// Ask for the workspace list, which the picker shows and discovery does not need.
    ///
    /// @param sink - where the request goes.
    /// @returns whether the request was written.
    pub fn request_workspaces(&mut self, sink: &mut dyn FrameSink) -> bool {
        let now = rpc::now_millis();
        let outgoing = self.follow.request_workspaces(now);
        self.send_request(outgoing, now, sink)
    }

    /// Ask the host to restart or stop this process.
    ///
    /// Offered to the user through the menu bar (`runtime/tray.rs`). The answer is not awaited:
    /// in the stop case this process is gone before it could arrive, which is the point.
    ///
    /// @param action - restart, or stop on purpose.
    /// @param sink - where the frame goes.
    /// @returns whether the request was written.
    pub fn request_panel_lifecycle(&mut self, action: PanelLifecycle, sink: &mut dyn FrameSink) -> bool {
        let now = rpc::now_millis();
        self.send_request(Some(Outgoing::PanelLifecycle { action }), now, sink)
    }

    /// Send one conversation request, when there is one.
    ///
    /// @param outgoing - what to ask for, if anything.
    /// @param now - caller-local time.
    /// @param sink - where the frame goes.
    /// @returns whether a request was written.
    fn send_request(&mut self, outgoing: Option<Outgoing>, now: i64, sink: &mut dyn FrameSink) -> bool {
        match outgoing {
            Some(outgoing) => {
                self.send_follow(outgoing, now, sink);
                true
            }
            // Nothing to ask: the request would duplicate one already in flight, and the
            // state the user asked for is already the state the layer is in.
            None => false,
        }
    }

    /// Send one conversation request, and remember that its answer is expected.
    ///
    /// @param outgoing - what to ask for.
    /// @param now - caller-local time, recorded so a lost answer can be noticed.
    /// @param sink - where the frame goes.
    fn send_follow(&mut self, outgoing: Outgoing, now: i64, sink: &mut dyn FrameSink) {
        if let Outgoing::CreateSession { workspace_id } = &outgoing {
            if workspace_id.as_deref().is_none_or(|id| id.trim().is_empty()) {
                // Caught here rather than at the wire: the caller asked for a conversation
                // without saying where, and the honest answer is that the panel needs a
                // workspace before it can make one.
                sink.log("a conversation was asked for without a workspace");
                sink.mark("create refused: no workspace");
                self.follow.abandon(now);
                if let Some(text) = self.pending_prompt.take() {
                    // The text the user typed is not sent, and they are told why.
                    self.prompt = Delivery::Failed { reason: CREATE_WITHOUT_WORKSPACE.to_owned() };
                    sink.log(&format!("{CREATE_WITHOUT_WORKSPACE}: {text:?} was not sent"));
                }
                return;
            }
        }
        let id = self.take_request_id();
        let frame = match &outgoing {
            Outgoing::ListSessions => rpc::request(id, "sessions/list", json!({})),
            Outgoing::ListWorkspaces => rpc::request(id, "workspaces/list", json!({})),
            // No parameters: the host's `workspaceController.initializeDefault` decides the
            // directory, creates it, and registers it. The panel only asks for it to exist.
            Outgoing::DefaultWorkspace => rpc::request(id, "workspace/default", json!({})),
            Outgoing::PanelLifecycle { action } => {
                rpc::request(id, "panel/lifecycle", json!({ "action": action.wire() }))
            }
            Outgoing::CreateSession { workspace_id } => {
                // The host's router requires a non-empty `workspaceId` and rejects the request
                // outright (`requireString`, `bridge/router.ts`), so "ask for the default by
                // sending nothing" is not a thing the protocol can express — the empty value is
                // the *host's own* internal default, not the wire's. There is no frame to send
                // here without an id, and sending one would be a request the host answers with
                // an error the user cannot act on.
                let workspace_id = workspace_id.clone().unwrap_or_default();
                rpc::request(id, "session/create", json!({ "workspaceId": workspace_id }))
            }
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

    /// Ask the host what may be chosen for this conversation.
    ///
    /// Safe before a conversation is attached: the catalogs are process-wide, so the pickers can be
    /// ready rather than appearing empty and filling in later. That is also why the request carries
    /// no session id when there is none — the host treats it as optional for this method alone.
    ///
    /// @param sink - where the request goes.
    pub fn request_options(&mut self, sink: &mut dyn FrameSink) {
        let id = self.take_request_id();
        let params = match self.follow.session_id() {
            Some(session_id) => json!({ "sessionId": session_id }),
            None => json!({}),
        };
        // Only the newest catalog read may settle, even when a menu is reopened quickly.
        self.setting_requests.retain(|_, request| !matches!(request, SettingRequest::Options { .. }));
        self.setting_requests.insert(id, SettingRequest::Options { session_id: self.follow.session_id().map(str::to_owned) });
        if let Err(error) = sink.send(&rpc::request(id, "session/options", params)) {
            self.setting_requests.remove(&id);
            self.setting_failure = Some(format!("无法读取可选项：{error}"));
        }
    }

    /// Switch the attached conversation's model and reasoning effort.
    ///
    /// **Both together**, because the host holds them in one selection: a model change resets the
    /// effort to that model's default, so sending an effort separately would briefly register one the
    /// model may not have.
    ///
    /// @param provider - the provider route of the chosen model.
    /// @param model - the chosen model's id.
    /// @param effort - the effort to pair with it, or `None` for the host's default.
    /// @param sink - where the request goes.
    pub fn select_model(
        &mut self,
        provider: &str,
        model: &str,
        effort: Option<&str>,
        sink: &mut dyn FrameSink,
    ) {
        let Some(session_id) = self.follow.session_id().map(str::to_owned) else {
            self.setting_failure = Some("尚未跟随任何会话".to_owned());
            return;
        };
        let id = self.take_request_id();
        let mut params = json!({ "sessionId": session_id, "provider": provider, "model": model });
        if let Some(effort) = effort {
            params["reasoningEffort"] = json!(effort);
        }
        self.setting_requests.insert(id, SettingRequest::SelectModel);
        if let Err(error) = sink.send(&rpc::request(id, "session/select", params)) {
            self.setting_requests.remove(&id);
            self.setting_failure = Some(format!("无法切换模型：{error}"));
        }
    }

    /// Apply one permission preset to the attached conversation.
    ///
    /// @param value - the preset's stable value, as the catalog spelled it.
    /// @param sink - where the request goes.
    pub fn set_permission(&mut self, value: &str, sink: &mut dyn FrameSink) {
        let Some(session_id) = self.follow.session_id().map(str::to_owned) else {
            self.setting_failure = Some("尚未跟随任何会话".to_owned());
            return;
        };
        let id = self.take_request_id();
        self.setting_requests
            .insert(id, SettingRequest::SetPermission(value.to_owned()));
        let frame = rpc::request(
            id,
            "session/permission",
            json!({ "sessionId": session_id, "value": value }),
        );
        if let Err(error) = sink.send(&frame) {
            self.setting_requests.remove(&id);
            self.setting_failure = Some(format!("无法切换权限：{error}"));
        }
    }

    /// Record the outcome of one setting request.
    ///
    /// @param request - which question was answered.
    /// @param outcome - the host's answer.
    /// @param sink - for the record.
    fn resolve_setting(
        &mut self,
        request: SettingRequest,
        outcome: Result<Value, RpcError>,
        sink: &mut dyn FrameSink,
    ) {
        match outcome {
            Ok(result) => {
                self.setting_failure = None;
                match request {
                    SettingRequest::Options { session_id } => {
                        if session_id.as_deref() != self.follow.session_id() { return; }
                        self.options = Some(parse_options(&result));
                        // On the record as a mark: "the model list is empty" is a question about
                        // this line, and the panel cannot show what it was never given.
                        let options = self.options.as_ref();
                        sink.mark(&format!(
                            "options {} models, {} permissions, model={} permission={}",
                            options.map_or(0, |options| options.models.len()),
                            options.map_or(0, |options| options.permissions.len()),
                            options
                                .and_then(|options| options.current_model.as_ref())
                                .map_or("?", |(_, model)| model.as_str()),
                            options
                                .and_then(|options| options.current_permission.as_deref())
                                .unwrap_or("?"),
                        ));
                    }
                    SettingRequest::SelectModel => {
                        // The authoritative value comes back in the projection, not here: this
                        // answer only says the host accepted the switch. The panel therefore
                        // re-reads, rather than drawing what it asked for.
                        sink.mark("model selected");
                        self.request_options(sink);
                    }
                    SettingRequest::SetPermission(value) => {
                        sink.mark(&format!("permission {value} selected"));
                        self.request_options(sink);
                    }
                }
            }
            Err(error) => {
                // Kept, not logged and dropped: a permission or model that did not change is
                // invisible otherwise — the panel would go on showing the old value as if the click
                // had never happened.
                let why = format!("[{}] {}", error.code, error.message);
                self.setting_failure = Some(why.clone());
                sink.mark(&format!("setting refused: {why}"));
            }
        }
    }

    /// Drop the values that belonged to another conversation.
    ///
    /// The catalogs are process-wide and survive; the *current* model, effort and permission are per
    /// session, so carrying them across a switch would show one conversation's permissions beside
    /// another's contents — which is the one thing this control exists to make visible.
    fn forget_settings_for_another_conversation(&mut self) {
        self.setting_requests.retain(|_, request| !matches!(request, SettingRequest::Options { .. }));
        if let Some(options) = &mut self.options {
            options.current_model = None;
            options.current_effort = None;
            options.current_permission = None;
        }
        self.stats = None;
        self.stats_session = None;
        self.setting_failure = None;
    }

    /// What the host last said may be chosen, if it has said.
    #[must_use]
    pub fn options(&self) -> Option<&SessionOptions> {
        self.options.as_ref()
    }

    /// The last failure of a setting change, for the panel to show.
    #[must_use]
    pub fn setting_failure(&self) -> Option<&str> {
        self.setting_failure.as_deref()
    }

    /// The statistics of the conversation on screen, if there is one and the host reported any.
    ///
    /// The numbers are a *conversation's* — token counts and cache hits describe the turns that
    /// happened in it — so this answers `None` twice over: while nothing is followed (the
    /// new-conversation state after "新建会话" keeps the last session's figures in hand), and
    /// while they belong to a session other than the one on screen.
    #[must_use]
    pub fn stats(&self) -> Option<Stats> {
        if self.follow.session_id() != self.stats_session.as_deref() {
            return None;
        }
        self.stats
    }

    /// Record the statistics the host pushed.
    ///
    /// @param params - the notification's params: `{sessionId, stats}`.
    fn note_stats(&mut self, params: &Value, sink: &mut dyn FrameSink) {
        // Only the conversation being shown: a notification for another one would put its token
        // counts under this conversation's answer.
        let session_id = params.get("sessionId").and_then(Value::as_str);
        if session_id != self.follow.session_id() {
            return;
        }
        let Some(stats) = params.get("stats") else {
            sink.log("session/stats carried no statistics");
            return;
        };
        let recorded = Stats {
            turns: stats.get("turns").and_then(Value::as_u64).unwrap_or(0),
            steps: stats.get("steps").and_then(Value::as_u64).unwrap_or(0),
            tokens_per_second: stats.get("tokensPerSecond").and_then(Value::as_f64),
            total_tokens: stats.get("totalTokens").and_then(Value::as_u64),
            cache_hit_percent: stats.get("cacheHitPercent").and_then(Value::as_f64),
            context_tokens: stats.get("contextTokens").and_then(Value::as_u64),
            context_limit: stats.get("contextLimit").and_then(Value::as_u64),
        };
        let ran = recorded.steps > 0;
        self.stats = Some(recorded);
        self.stats_session = session_id.map(str::to_owned);

        // A conversation this panel created is attached the moment it exists, so the options read
        // that follows the attach arrives *before* Harness has recorded what the conversation will
        // use: the model projection is built from `request/header`, and no request has been made yet.
        // The host answers `model=? permission=workspace-write` — honestly, and uselessly: without a
        // second read the footer says "unknown" for the rest of the run while the conversation runs
        // on the deployment default.
        //
        // A closed step is exactly when that record exists, so ask once more then — and only while
        // the model is still missing, so a conversation that has never run is not asked about again
        // for reporting statistics of its own.
        let knows_model = self
            .options
            .as_ref()
            .is_some_and(|options| options.current_model.is_some());
        if ran && !knows_model {
            self.request_options(sink);
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
    /// - the request is not an **approval** — a question has its own answer shape and
    ///   its own entry point ([`Session::answer_question`]), and sending a verdict for a
    ///   question would tell the model the user chose something they never saw;
    /// - the request is already in flight or already resolved — the grant is
    ///   one-shot, so a second send would be a second decision for one question;
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
        if let Some(card) = self.interactions.iter().find(|held| held.id() == interaction_id) {
            if card.kind() != InteractionKind::Approval {
                // A verdict is not an answer to a question. Checked here rather than in
                // `answer` because only the caller knows which shape it is holding, and
                // a question must travel through `answer_question` or not at all.
                sink.log(&format!("not answering {interaction_id}: it is not an approval card"));
                return false;
            }
        }
        self.answer(interaction_id, SentAnswer::Approval(verdict), sink)
    }

    /// Answer the questions of one question card.
    ///
    /// The questions this build will answer are exactly the ones [`Interaction::questions`]
    /// showed (see [`Interaction::question_problem`] for the caps), so the answer is
    /// checked against those before anything is sent. Refuses in four cases:
    ///
    /// - the id is not held;
    /// - the request is already in flight or already resolved, for the same reason an
    ///   approval is: the host settles a request once;
    /// - the card is an approval, or a question this build cannot answer in full —
    ///   the answer would be a partial one for a request the user saw in part;
    /// - the answer itself does not cover the questions, per
    ///   [`crate::app::session::interaction::validate_answers`].
    ///
    /// @param interaction_id - which card to answer.
    /// @param answers - one answer per question, naming the options by label.
    /// @param sink - where the answer goes.
    /// @returns whether an answer was sent.
    pub fn answer_question(
        &mut self,
        interaction_id: &str,
        answers: Vec<QuestionAnswer>,
        sink: &mut dyn FrameSink,
    ) -> bool {
        let Some(index) = self.interactions.iter().position(|held| held.id() == interaction_id) else {
            sink.log(&format!("not answering {interaction_id}: this panel is not holding it"));
            return false;
        };
        if self.interactions[index].kind() != InteractionKind::Question {
            sink.log(&format!("not answering {interaction_id}: it is not a question card"));
            return false;
        }
        if let Some(problem) = self.interactions[index].question_problem() {
            sink.log(&format!(
                "not answering {interaction_id}: this build cannot answer it here ({problem})",
            ));
            return false;
        }
        if let Some(problem) = validate_answers(&self.interactions[index].questions(), &answers) {
            sink.log(&format!("refusing the answer for {interaction_id}: {problem}"));
            return false;
        }
        self.answer(interaction_id, SentAnswer::Question(answers), sink)
    }

    /// Send one answer frame and record that it is in flight.
    ///
    /// The shared half of [`Session::answer_interaction`] and [`Session::answer_question`]:
    /// the guards are the same for both kinds (the request must be pending, the frame
    /// must be attributable, a failed write must leave the card actionable), and the only
    /// difference is the shape of `answer` on the wire. Splitting it this way keeps those
    /// guards in one place instead of two copies that can drift apart.
    ///
    /// @param interaction_id - which request to answer.
    /// @param answer - what was decided.
    /// @param sink - where the answer goes.
    /// @returns whether an answer was sent.
    fn answer(
        &mut self,
        interaction_id: &str,
        answer: SentAnswer,
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
        let request_id = self.take_request_id();
        let body = match &answer {
            SentAnswer::Approval(verdict) => {
                json!({ "interactionId": interaction_id, "answer": { "outcome": verdict.wire() } })
            }
            SentAnswer::Question(answers) => json!({
                "interactionId": interaction_id,
                "answer": { "answers": answers.iter().map(answer_json).collect::<Vec<_>>() },
            }),
        };
        let frame = rpc::request(request_id, "interaction/answer", body);
        let summary = answer.summary();
        self.interactions[index].state = InteractionState::Submitting { answer };
        self.answers_in_flight.insert(request_id, interaction_id.to_owned());
        sink.mark(&format!("interaction {interaction_id} answering {summary}"));
        if let Err(error) = sink.send(&frame) {
            self.interactions[index].state = InteractionState::Pending;
            self.answers_in_flight.remove(&request_id);
            sink.log(&format!("could not send an answer for {interaction_id}: {error}"));
            return false;
        }
        true
    }

    /// Send what the user typed to the conversation this panel follows.
    ///
    /// The key is generated here and is one-shot: the host admits a given key once, and a
    /// retry after a failure deliberately gets a *new* one, because reusing the failed key
    /// is what the host refuses ("submit it again to retry deliberately").
    ///
    /// @param text - the user's text. Blank input is refused here rather than sent: the
    ///   host would accept it and the conversation would gain an empty turn.
    /// @param sink - where the frame goes.
    /// @returns what is known now; the host's verdict arrives later and is reported
    ///   through [`Session::prompt_delivery`].
    pub fn send_prompt(&mut self, text: &str, workspace_id: Option<&str>, sink: &mut dyn FrameSink) -> Delivery {
        let text = text.trim();
        if text.is_empty() {
            self.prompt = Delivery::Failed { reason: "内容为空".to_owned() };
            return self.prompt.clone();
        }
        if self.prompt.is_sending() {
            // The host would refuse a *different* key for the same moment anyway, and a
            // second copy of the same text is not what the user asked for.
            return self.prompt.clone();
        }
        let Some(session_id) = self.follow.session_id().map(str::to_owned) else {
            // Nothing to send to: the panel is in "new conversation" mode, and this is the
            // moment the conversation is created — on submit, never on open. A panel that
            // created one every time it was summoned would fill the harness's list with
            // conversations nobody ever used.
            // Nowhere to create it: the host requires a workspace, so the panel asks the user
            // for one instead of sending a request the host will reject.
            let Some(workspace_id) = workspace_id.filter(|id| !id.trim().is_empty()) else {
                // Two different problems, two different answers: a list that has not arrived
                // yet needs patience, and a list with nothing in it needs the user.
                self.prompt = Delivery::Failed {
                    reason: if self.follow.workspaces_asked() {
                        CREATE_WITHOUT_WORKSPACE.to_owned()
                    } else {
                        "正在获取工作区，请稍候再发送".to_owned()
                    },
                };
                return self.prompt.clone();
            };
            let now = rpc::now_millis();
            let Some(outgoing) = self.follow.create(Some(workspace_id), now) else {
                self.prompt = Delivery::Failed { reason: "正在创建会话，请稍候再发送".to_owned() };
                return self.prompt.clone();
            };
            self.pending_prompt = Some(text.to_owned());
            self.prompt = Delivery::Sending;
            self.send_follow(outgoing, now, sink);
            sink.mark(&format!("create-then-prompt {} bytes", text.len()));
            return self.prompt.clone();
        };
        self.send_to(&session_id, text, sink)
    }

    /// Write one prompt for a conversation that exists.
    ///
    /// @param session_id - where it goes.
    /// @param text - what to say.
    /// @param sink - where the frame goes.
    /// @returns how the attempt ended.
    fn send_to(&mut self, session_id: &str, text: &str, sink: &mut dyn FrameSink) -> Delivery {
        let key = Self::prompt_key(session_id);
        let request_id = self.take_request_id();
        let frame = rpc::request(
            request_id,
            "session/prompt",
            json!({ "sessionId": session_id, "requestId": key, "text": text }),
        );
        self.prompt = Delivery::Sending;
        self.sends_in_flight.insert(request_id, Sending::Prompt);
        sink.mark(&format!("prompt {key} sending {} bytes", text.len()));
        if let Err(error) = sink.send(&frame) {
            self.sends_in_flight.remove(&request_id);
            self.prompt = Delivery::Failed { reason: format!("写入失败：{error}") };
            sink.log(&format!("could not send a prompt: {error}"));
        }
        self.prompt.clone()
    }

    /// The idempotency key for one prompt attempt.
    ///
    /// **It has to be new for a new attempt, and the attempt may be in a different process than the
    /// last one.** The host keeps keys for the life of a *conversation*: its session controller
    /// answers `accepted: true` **without admitting anything** when the `requestId` matches a
    /// `user/message` already in that session's history (`hasPromptRequest` in
    /// `packages/api/session-controller/src/commands.ts`). A counter that begins at zero in every
    /// process therefore loses the first prompt after every restart — the input is cleared, the host
    /// says "accepted", and the message is simply gone.
    ///
    /// So the key is built from things that do not restart with the panel: a sequence that runs for the
    /// life of the process ([`PROMPT_SEQUENCE`]), the process id, and the clock. The session id is there
    /// so the marker stays readable. A per-session counter is *not* enough on its own: two sessions
    /// in one process, in the same millisecond, would produce the same key.
    ///
    /// @param session_id - the conversation the prompt goes to.
    /// @returns the key to put in `requestId`.
    fn prompt_key(session_id: &str) -> String {
        Self::prompt_key_for(
            session_id,
            // The run this attempt belongs to: the clock keeps one run out of the next, the process id
            // keeps two panels started in the same millisecond apart.
            &format!("{}-{}", std::process::id(), rpc::now_millis()),
            PROMPT_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1,
        )
    }

    /// The key's shape, as a function of the three things it is made of.
    ///
    /// A pure function because the property that matters is about **runs**, and a run cannot be produced
    /// from inside one: two `Session`s in a test share the process id and can share the clock, so a test
    /// that builds two of them proves nothing about a panel that was restarted. Here the run is an
    /// argument, and a key that ignores it fails (`a_prompt_key_names_the_attempt_not_just_the_conversation`).
    ///
    /// @param session_id - the conversation the prompt goes to.
    /// @param run - what identifies this run of the panel.
    /// @param sequence - how many keys this run has issued, counting this one.
    /// @returns the key to put in `requestId`.
    fn prompt_key_for(session_id: &str, run: &str, sequence: u64) -> String {
        format!("{session_id}:prompt-{run}-{sequence}")
    }

    /// Send the prompt that was waiting for a conversation to exist.
    ///
    /// Called after every follow answer: the creation this prompt is behind finishes as an
    /// attach, and an attach is only known to have worked when its own answer arrives.
    ///
    /// @param sink - where the prompt goes.
    fn flush_pending_prompt(&mut self, sink: &mut dyn FrameSink) {
        let Some(text) = self.pending_prompt.clone() else {
            return;
        };
        if let Some(reason) = self.follow.take_create_failure() {
            // The conversation could not be made, so the text was never sent. Saying so is the
            // whole job of `Delivery`: an attempt that failed silently is indistinguishable
            // from a panel that ignored the key.
            self.pending_prompt = None;
            self.prompt = Delivery::Failed { reason: format!("无法创建会话：{reason}") };
            sink.log(&format!("a prompt could not be sent: {reason}"));
            return;
        }
        let Some(session_id) = self.follow.session_id().map(str::to_owned) else {
            return;
        };
        self.pending_prompt = None;
        self.send_to(&session_id, &text, sink);
    }

    /// Tell the host the panel is no longer showing a conversation.
    ///
    /// The host decides who answers an approval, and it knows the panel's conversation from
    /// the attach it was asked for. A panel that quietly stopped showing one would go on
    /// claiming requests for it — for a conversation the user has left, with no context on
    /// screen to answer from.
    ///
    /// A notification rather than a request: it is a fact, and there is nothing to answer.
    ///
    /// @param session_id - the conversation being left.
    /// @param sink - where the notice goes. Best effort: a lost channel is reported by the
    ///   supervisor, and a failure here must not stop the panel from starting a new one.
    fn notify_detached(&self, session_id: &str, sink: &mut dyn FrameSink) {
        let notice = rpc::notification("session/detach", json!({ "sessionId": session_id }));
        if sink.send(&notice).is_ok() {
            sink.mark(&format!("detached {session_id}"));
        }
    }

    /// Drop the transcript of a conversation the panel is no longer attached to.
    ///
    /// The two are meant to agree — the transcript learns its conversation from a snapshot,
    /// which only arrives while attached — so a transcript with no attachment behind it means
    /// the attachment was let go, and the panel is showing a conversation it is not following.
    /// That happens when a pinned conversation disappears from the harness.
    ///
    /// @param sink - where the reason goes: a panel that silently empties itself is
    ///   indistinguishable from one that lost its place.
    fn forget_a_conversation_that_is_gone(&mut self, sink: &mut dyn FrameSink) {
        if self.follow.session_id().is_some() {
            return;
        }
        let Some(session_id) = self.transcript.session_id().map(str::to_owned) else {
            return;
        };
        sink.log(&format!("forgetting {session_id}: it is no longer being followed"));
        self.notify_detached(&session_id, sink);
        self.transcript.reset();
    }

    /// Ask the host to stop the turn in flight.
    ///
    /// @param sink - where the frame goes.
    /// @returns whether the request was written; the host's answer arrives later.
    pub fn cancel_turn(&mut self, sink: &mut dyn FrameSink) -> bool {
        if self.cancel.is_sending() {
            return false;
        }
        let Some(session_id) = self.follow.session_id().map(str::to_owned) else {
            self.cancel = Delivery::Failed { reason: "尚未跟随任何会话".to_owned() };
            return false;
        };
        let request_id = self.take_request_id();
        let frame = rpc::request(request_id, "session/cancel", json!({ "sessionId": session_id }));
        self.cancel = Delivery::Sending;
        self.sends_in_flight.insert(request_id, Sending::Cancel);
        sink.mark("cancel requested");
        if let Err(error) = sink.send(&frame) {
            self.sends_in_flight.remove(&request_id);
            self.cancel = Delivery::Failed { reason: format!("写入失败：{error}") };
            sink.log(&format!("could not ask the host to stop: {error}"));
            return false;
        }
        true
    }

    /// What became of the last prompt.
    #[must_use]
    pub fn prompt_delivery(&self) -> &Delivery {
        &self.prompt
    }

    /// What became of the last stop request.
    #[must_use]
    pub fn cancel_delivery(&self) -> &Delivery {
        &self.cancel
    }

    /// Read the host's verdict on an outbound request.
    fn resolve_send(&mut self, id: i64, outcome: Result<Value, RpcError>, sink: &mut dyn FrameSink) {
        let Some(kind) = self.sends_in_flight.remove(&id) else { return };
        let delivery = match outcome {
            Ok(_) => Delivery::Accepted,
            Err(error) => Delivery::Failed { reason: error.message.clone() },
        };
        match kind {
            Sending::Prompt => {
                // A prompt that was accepted is also the point at which the panel stops
                // offering to send it again; a failure keeps the reason the host gave,
                // because "unavailable: the same prompt is still awaiting confirmation"
                // is something the user can act on and "failed" is not.
                let line = match &delivery {
                    Delivery::Accepted => "prompt accepted".to_owned(),
                    Delivery::Failed { reason } => format!("prompt refused: {reason}"),
                    Delivery::Idle | Delivery::Sending => "prompt state unclear".to_owned(),
                };
                sink.mark(&line);
                self.prompt = delivery;
            }
            Sending::Cancel => {
                let line = match &delivery {
                    Delivery::Accepted => "cancel accepted".to_owned(),
                    Delivery::Failed { reason } => format!("cancel refused: {reason}"),
                    Delivery::Idle | Delivery::Sending => "cancel state unclear".to_owned(),
                };
                sink.mark(&line);
                self.cancel = delivery;
            }
        }
    }

    /// Drop one interaction from the panel.
    ///
    /// Local only, and honest about what it does not do: the host still holds the
    /// request until its own deadline. It exists for cards that cannot be acted on — a
    /// question too large to answer here, say — and for resolved ones the user has read.
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
    /// Both kinds of answer are matched the same way — by the id-uniqueness recorded in
    /// [`Session::answers_in_flight`] — because the host's response carries nothing but
    /// the request id, so the kind of the answer is read back from the card rather than
    /// assumed. A question card and an approval card settle through this one path.
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
        let InteractionState::Submitting { answer } = self.interactions[index].state.clone() else {
            sink.log(&format!("a response arrived for interaction {interaction_id}, which is not in flight"));
            return;
        };
        match outcome {
            Ok(result) if result.get("accepted").and_then(Value::as_bool) == Some(true) => {
                let summary = answer.summary();
                self.interactions[index].state = InteractionState::Applied { answer };
                sink.mark(&format!("interaction {interaction_id} applied {summary}"));
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

/// One entry of the `answer.answers` array on the wire.
///
/// `custom` is **omitted**, not nulled, when there is no free text: the upstream answer
/// shape treats an absent `custom` as "no Other answer", and an explicit null is a
/// distinction the host would have to interpret. Kept here rather than inline in the
/// frame so the omission is a property of the type that knows what a custom answer is.
///
/// @param answer - the answer to encode.
/// @returns the JSON object for one question's answer.
#[must_use]
fn answer_json(answer: &QuestionAnswer) -> Value {
    let mut value = json!({ "id": answer.id, "selected": answer.selected });
    if let (Some(custom), Some(object)) = (answer.custom.as_ref(), value.as_object_mut()) {
        object.insert("custom".to_owned(), Value::String(custom.clone()));
    }
    value
}


/// Read the choices a `session/options` answer described.
///
/// Every level is checked rather than trusted, and an entry that cannot be selected is dropped:
/// a model with no id cannot be sent back, so offering it would be offering a dead row. The shapes
/// here are the ones the host builds (`readModelGroups` / the permission catalog in
/// `src/harness/adapter.ts`), and an unrecognised shape yields an empty list rather than a guess.
///
/// @param result - the answer's result object.
/// @returns what may be chosen, with whatever could be read.
#[must_use]
pub fn parse_options(result: &Value) -> SessionOptions {
    let mut options = SessionOptions::default();
    if let Some(current) = result.get("current") {
        let provider = current.get("provider").and_then(Value::as_str);
        let model = current.get("model").and_then(Value::as_str);
        if let (Some(provider), Some(model)) = (provider, model) {
            options.current_model = Some((provider.to_owned(), model.to_owned()));
        }
        options.current_effort = current
            .get("reasoningEffort")
            .and_then(Value::as_str)
            .map(str::to_owned);
    }
    if let Some(groups) = result.get("groups").and_then(Value::as_array) {
        for group in groups {
            // `provider`, not `id`: the host projects the upstream group's `id` into a `provider`
            // field, because that is what it is — the provider route a choice must name. A fixture
            // written from the same assumption as this parser cannot catch a mix-up here.
            let provider = group
                .get("provider")
                .or_else(|| group.get("id"))
                .and_then(Value::as_str);
            let Some(provider) = provider else {
                continue;
            };
            let Some(models) = group.get("models").and_then(Value::as_array) else {
                continue;
            };
            for model in models {
                let Some(id) = model.get("id").and_then(Value::as_str) else {
                    continue;
                };
                let efforts = model
                    .get("efforts")
                    .and_then(Value::as_array)
                    .map(|efforts| {
                        efforts
                            .iter()
                            .filter_map(|effort| {
                                let id = effort.get("id").and_then(Value::as_str)?;
                                let name = effort
                                    .get("name")
                                    .and_then(Value::as_str)
                                    .unwrap_or(id);
                                Some(EffortOption { id: id.to_owned(), name: name.to_owned(), description: effort.get("description").and_then(Value::as_str).map(str::to_owned) })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                options.models.push(ModelOption {
                    provider: provider.to_owned(),
                    provider_name: group.get("name").and_then(Value::as_str).unwrap_or(provider).to_owned(),
                    description: model.get("description").and_then(Value::as_str).map(str::to_owned),
                    id: id.to_owned(),
                    name: model
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or(id)
                        .to_owned(),
                    efforts,
                    default_effort: model
                        .get("defaultEffort")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                });
            }
        }
    }
    if let Some(permissions) = result.get("permissions").and_then(Value::as_array) {
        options.permissions = permissions
            .iter()
            .filter_map(|permission| {
                let value = permission.get("value").and_then(Value::as_str)?;
                let name = permission
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(value);
                Some(PermissionOption { value: value.to_owned(), name: name.to_owned(), description: permission.get("description").and_then(Value::as_str).map(str::to_owned) })
            })
            .collect();
    }
    options.current_permission = result
        .get("permission")
        .and_then(Value::as_str)
        .map(str::to_owned);
    options
}

// Protocol contract: the handshake, the method surface, and the answers the host reads.
// Category 1 (the host/sidecar wire) plus category 2 tripwires for the harness's own payloads.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::session::test_support::*;

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
        assert_eq!(params["capabilities"], json!(["window", "hotkey", "tauri", "approval", "question"]));
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
    fn the_menu_bar_can_ask_the_host_to_restart_or_stop_this_process() {
        // The tray offers these two because the panel cannot do them itself: replacing or ending
        // the process is the supervisor's business, and an exit it reads as a crash would simply
        // be restarted. What this asserts is the wire half of that: both choices leave as a
        // `panel/lifecycle` request carrying the action the host's router expects.
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        let mut source = ScriptedSource::new(vec![hello_ok()]);
        session.run(&mut source, &mut sink);
        sink.frames.clear();

        assert!(session.request_panel_lifecycle(PanelLifecycle::Restart, &mut sink));
        assert!(session.request_panel_lifecycle(PanelLifecycle::Stop, &mut sink));

        let asked: Vec<Value> = sink
            .frames
            .iter()
            .map(|frame| json!([frame["method"], frame["params"]["action"]]))
            .collect();
        assert_eq!(
            asked,
            vec![
                json!(["panel/lifecycle", "restart"]),
                json!(["panel/lifecycle", "stop"]),
            ],
        );
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
    fn panel_visibility_is_reported_as_a_notification() {
        // A notification, not a request: the host handles it in its notification
        // path and the peer has nothing to wait for. It must also be sent when the
        // panel is hidden, since "hidden" is a fact the host routes on.
        let session = Session::new(identity());
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
        let session = Session::new(identity());
        let mut sink = RecordingSink::default();
        session.report_visibility(false, &["window", "hotkey"], &mut sink);
        assert_eq!(sink.frames[0]["params"]["capabilities"], json!(["window", "hotkey"]));
    }

    #[test]
    fn the_options_parser_reads_the_provider_field_the_host_actually_sends() {
        // The shape is the host's own answer (`modelCatalog()`), not written from the same
        // assumption as the parser: the host projects the upstream group's `id` into
        // `provider`, and a fixture that called it `id` would pass while the panel shows an
        // empty model menu.
        let payload = json!({
            "groups": [{
                "provider": "deepseek-official",
                "name": "DeepSeek",
                "models": [{
                    "id": "deepseek-flash",
                    "name": "DeepSeek-V41-Flash",
                    "efforts": [{"id": "off", "name": "Off"}, {"id": "high", "name": "High"}],
                    "defaultEffort": "high",
                }],
            }],
            "current": {"provider": "deepseek-official", "model": "deepseek-flash", "reasoningEffort": "high"},
            "permissions": [{"value": "read-only", "name": "只读"}],
            "permission": "read-only",
        });
        let options = parse_options(&payload);
        assert_eq!(options.models.len(), 1, "the real answer yields a model");
        let model = &options.models[0];
        assert_eq!(model.provider, "deepseek-official", "and the provider is the route to send back");
        assert_eq!(model.id, "deepseek-flash");
        assert_eq!(model.efforts.len(), 2);
        assert_eq!(model.default_effort.as_deref(), Some("high"));
        assert_eq!(options.current_model, Some(("deepseek-official".to_owned(), "deepseek-flash".to_owned())));
        assert_eq!(options.permissions.len(), 1);
        assert_eq!(options.current_permission.as_deref(), Some("read-only"));
    }

    /// Category 2: `session/stats` is the harness's payload, so the field names read here
    /// are its contract, not ours.
    #[test]
    fn stats_carries_the_fields_the_footer_reads() {
        let (mut session, mut sink) = followed();
        session.on_frame(
            notification(
                "session/stats",
                json!({
                    "sessionId": "session-1",
                    "stats": {
                        "turns": 3, "steps": 7, "totalTokens": 15_600, "tokensPerSecond": 200.5,
                        "cacheHitPercent": 75, "contextTokens": 4000, "contextLimit": 128000,
                    },
                }),
            ),
            &mut sink,
        );
        let stats = session.stats().expect("the followed conversation's figures are kept");
        assert_eq!(stats.turns, 3);
        assert_eq!(stats.steps, 7);
        assert_eq!(stats.total_tokens, Some(15_600));
        assert_eq!(stats.tokens_per_second, Some(200.5));
        assert_eq!(stats.cache_hit_percent, Some(75.0));
        assert_eq!(stats.context_tokens, Some(4000));
        assert_eq!(stats.context_limit, Some(128000));
    }

    /// Category 1: the host's pushed workspace frames reach the list the snapshot carries.
    ///
    /// The host follows the Harness's workspace stream and forwards each frame as
    /// `workspaces/changed`, which is what makes a workspace added or removed in the Harness
    /// visible in the panel without a restart. This asserts the wire half of that: the frame
    /// shapes the host sends are applied to `workspaces()`, and each applied change leaves one
    /// marker behind.
    #[test]
    fn a_pushed_workspace_frame_updates_the_workspace_list() {
        let ids = |session: &Session| -> Vec<String> {
            session.workspaces().iter().map(|entry| entry.workspace_id.clone()).collect()
        };
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();

        session.on_frame(
            notification(
                "workspaces/changed",
                json!({"type": "baseline", "value": {
                    "items": [
                        {"workspaceId": "w1", "title": "one", "path": "/one", "sessionIds": ["s1"], "createdAt": 5},
                        {"workspaceId": "w2", "title": "two", "path": "/two", "sessionIds": [], "createdAt": 9},
                    ],
                    "archivedSessionIds": [],
                    "pinnedSessionIds": ["s1"],
                }}),
            ),
            &mut sink,
        );
        assert_eq!(ids(&session), ["w1", "w2"], "a baseline is the list");
        assert!(session.workspaces_loaded(), "and it arrived without a request");

        // An upsert replaces a known row in place and appends an unknown one.
        for (id, title) in [("w2", "two renamed"), ("w3", "three")] {
            session.on_frame(
                notification(
                    "workspaces/changed",
                    json!({"type": "upsert", "workspace": {
                        "workspaceId": id, "title": title, "path": "/" , "sessionIds": [], "createdAt": 12,
                    }}),
                ),
                &mut sink,
            );
        }
        assert_eq!(session.workspaces()[1].title, "two renamed");
        assert_eq!(ids(&session), ["w1", "w2", "w3"]);

        session.on_frame(
            notification("workspaces/changed", json!({"type": "remove", "workspaceId": "w1"})),
            &mut sink,
        );
        session.on_frame(
            notification("workspaces/changed", json!({"type": "order", "workspaceIds": ["w3", "w2"]})),
            &mut sink,
        );
        assert_eq!(ids(&session), ["w3", "w2"]);

        // One marker per applied change: the host swallows this process's stderr, so the marker
        // is the only place a push is visible after the fact.
        for expected in [
            "workspaces changed: baseline 2",
            "workspaces changed: upsert w2",
            "workspaces changed: upsert w3",
            "workspaces changed: removed w1",
            "workspaces changed: order 2",
        ] {
            assert!(
                sink.marks.iter().any(|line| line == expected),
                "missing marker {expected:?} in {:?}",
                sink.marks,
            );
        }
    }

    /// A session following `session-1`: `session/stats` is only kept for the attached one.
    fn followed() -> (Session, RecordingSink) {
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        follow_a_conversation(&mut session, &mut sink);
        (session, sink)
    }
}

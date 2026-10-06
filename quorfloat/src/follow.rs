//! Following a conversation.
//!
//! The panel answers approvals for the conversation it is *showing*, and the host
//! decides that by `session/attach` (see `docs/protocol.md` §6): an interaction is
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

use crate::session::FrameSink;

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
}

impl Follow {
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
        match &self.in_flight {
            Some(InFlight::List { sent_at }) => now - *sent_at > REQUEST_TIMEOUT_MS,
            Some(InFlight::Attach { sent_at, .. }) => now - *sent_at > REQUEST_TIMEOUT_MS,
            None => false,
        }
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
            (InFlight::List { .. }, Ok(result)) => match newest_session(&result) {
                Some(found) if Some(found.session_id.as_str()) == self.session_id.as_deref() => {
                    // Already following the most recent conversation. Silence is
                    // deliberate: this is the steady state and it runs every few
                    // seconds.
                    None
                }
                Some(found) => {
                    if self.session_id.is_some() {
                        sink.log(&format!("a more recent conversation appeared: {}", found.session_id));
                    }
                    self.in_flight = Some(InFlight::Attach {
                        session_id: found.session_id.clone(),
                        label: found.label,
                        sent_at: now,
                    });
                    Some(Outgoing::Attach { session_id: found.session_id })
                }
                None => {
                    if self.session_id.is_none() {
                        sink.log("no conversation to follow yet");
                    }
                    None
                }
            },
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
struct SessionSummary {
    session_id: String,
    updated_at: i64,
    label: Option<String>,
}

/// Pick the most recently updated conversation out of a `sessions/list` result.
///
/// Ordered by `updatedAt` rather than by the array's order, which the protocol does
/// not promise to be meaningful. Entries without a usable id are skipped: attaching
/// to one would be a request the host must refuse.
///
/// @param result - the response value.
/// @returns the newest conversation, or `None` when there is none.
fn newest_session(result: &Value) -> Option<SessionSummary> {
    let items = result.get("items").and_then(Value::as_array)?;
    items
        .iter()
        .filter_map(|item| {
            let session_id = item.get("sessionId").and_then(Value::as_str)?;
            if session_id.is_empty() {
                return None;
            }
            Some(SessionSummary {
                session_id: session_id.to_owned(),
                updated_at: item.get("updatedAt").and_then(Value::as_i64).unwrap_or(0),
                label: item
                    .get("cwd")
                    .and_then(Value::as_str)
                    .and_then(|cwd| std::path::Path::new(cwd).file_name().map(|name| name.to_string_lossy().into_owned())),
            })
        })
        .max_by_key(|summary| summary.updated_at)
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
        follow.resolve(Ok(sessions(&[("a", 1)])), 10, &mut sink);
        assert!(follow.next(20).is_some() || follow.in_flight.is_some(), "the attach is now outstanding");
    }

    #[test]
    fn the_newest_conversation_is_chosen_not_the_first_listed() {
        // The protocol does not promise an order, so the choice is by `updatedAt`.
        let mut follow = started();
        let mut sink = Recorded::default();
        follow.next(0);
        let outgoing = follow.resolve(Ok(sessions(&[("old", 10), ("newest", 90), ("middle", 50)])), 5, &mut sink);
        assert_eq!(outgoing, Some(Outgoing::Attach { session_id: "newest".to_owned() }));
    }

    #[test]
    fn an_attach_is_recorded_with_its_generation_and_a_breadcrumb() {
        let mut follow = started();
        let mut sink = Recorded::default();
        follow.next(0);
        follow.resolve(Ok(sessions(&[("session-1", 10)])), 5, &mut sink);
        follow.resolve(Ok(json!({"sessionId": "session-1", "generation": 4, "replaced": false})), 6, &mut sink);
        assert_eq!(follow.session_id(), Some("session-1"));
        assert_eq!(follow.generation(), Some(4));
        assert_eq!(follow.label(), Some("project"));
        assert!(sink.joined().contains("following conversation session-1"));
        // The one fact that decides whether an approval can reach the panel must
        // survive the run.
        assert_eq!(sink.marks, vec!["follow session-1 generation=4".to_owned()]);
    }

    #[test]
    fn an_empty_list_is_not_an_error_and_is_retried_later() {
        // A fresh environment has no conversations at all. That is normal, must not be
        // logged every few seconds, and must not stop discovery.
        let mut follow = started();
        let mut sink = Recorded::default();
        follow.next(0);
        assert_eq!(follow.resolve(Ok(json!({"items": []})), 10, &mut sink), None);
        assert!(sink.joined().contains("no conversation to follow yet"));
        assert_eq!(follow.session_id(), None);
        assert_eq!(follow.next(10 + POLL_INTERVAL_MS - 1), None, "too early");
        assert_eq!(follow.next(10 + POLL_INTERVAL_MS), Some(Outgoing::ListSessions));
    }

    #[test]
    fn the_same_conversation_is_not_re_attached() {
        // The steady state runs every few seconds; re-attaching would churn the
        // subscription and log a line each time.
        let mut follow = started();
        let mut sink = Recorded::default();
        follow.next(0);
        follow.resolve(Ok(sessions(&[("session-1", 10)])), 1, &mut sink);
        follow.resolve(Ok(json!({"sessionId": "session-1", "generation": 1})), 2, &mut sink);
        let before = sink.logs.len();
        follow.next(2 + POLL_INTERVAL_MS);
        assert_eq!(follow.resolve(Ok(sessions(&[("session-1", 10)])), 3, &mut sink), None);
        assert_eq!(sink.logs.len(), before, "nothing new to say");
        assert_eq!(follow.session_id(), Some("session-1"));
    }

    #[test]
    fn a_newer_conversation_replaces_the_followed_one() {
        // Following is "the conversation the user is having", and they can start a new
        // one at any moment.
        let mut follow = started();
        let mut sink = Recorded::default();
        follow.next(0);
        follow.resolve(Ok(sessions(&[("session-1", 10)])), 1, &mut sink);
        follow.resolve(Ok(json!({"sessionId": "session-1", "generation": 1})), 2, &mut sink);
        follow.next(2 + POLL_INTERVAL_MS);
        let outgoing = follow.resolve(Ok(sessions(&[("session-1", 10), ("session-2", 99)])), 3, &mut sink);
        assert_eq!(outgoing, Some(Outgoing::Attach { session_id: "session-2".to_owned() }));
        assert!(sink.joined().contains("a more recent conversation appeared"));
        follow.resolve(Ok(json!({"sessionId": "session-2", "generation": 9})), 4, &mut sink);
        assert_eq!(follow.session_id(), Some("session-2"));
        assert_eq!(sink.marks, vec![
            "follow session-1 generation=1".to_owned(),
            "follow session-2 generation=9".to_owned(),
        ]);
    }

    #[test]
    fn a_refused_attach_keeps_looking() {
        // The host can refuse: the conversation may have been deleted between the list
        // and the attach. Giving up would leave the panel following nothing, which
        // looks exactly like having nothing to follow.
        let mut follow = started();
        let mut sink = Recorded::default();
        follow.next(0);
        follow.resolve(Ok(sessions(&[("session-1", 10)])), 1, &mut sink);
        let refused = follow.resolve(
            Err(crate::ipc::rpc::RpcError { code: -32004, message: "no such session".to_owned(), data: None }),
            2,
            &mut sink,
        );
        assert_eq!(refused, None);
        assert_eq!(follow.session_id(), None);
        assert!(sink.joined().contains("could not follow session-1"));
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
        assert_eq!(outgoing, Some(Outgoing::Attach { session_id: "ok".to_owned() }));
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

    /// A follow layer already attached to `session-1` at generation 3.
    fn following() -> Follow {
        let mut follow = started();
        let mut sink = Recorded::default();
        follow.next(0);
        follow.resolve(Ok(sessions(&[("session-1", 10)])), 1, &mut sink);
        follow.resolve(Ok(json!({"sessionId": "session-1", "generation": 3})), 2, &mut sink);
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

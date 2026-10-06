//! The requests the host hands to this panel, and what may be done about them.
//!
//! An interaction is not a message: it is a question with a lifetime. It arrives with an
//! id, it can be answered exactly once, the host's verdict decides whether the user may
//! try again, and it can expire. This module owns that lifetime, which is why it is
//! separate from [`super::Session`] — whose job is the wire, not the rules.
//!
//! Two kinds arrive, and this build claims only one of them:
//!
//! - an **approval**, which a click can settle;
//! - a **question**, which needs option widgets this build does not have yet, so it is
//!   reported and handed back to the Harness window (`docs/dsh-quorfloat.md` §6).
//!
//! The kind therefore travels on the wire instead of being inferred, and
//! [`InteractionKind::Approval`] is the one this build names in
//! [`crate::ipc::protocol::CAPABILITIES`].

use serde_json::Value;

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
    pub(super) id: String,
    pub(super) session_id: String,
    pub(super) kind: InteractionKind,
    pub(super) payload: Value,
    /// Where this interaction is in its life. Advanced by [`super::Session`], which is
    /// the module that owns the frames a transition may have to send.
    pub(super) state: InteractionState,
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
/// How many unanswered requests are kept. The parent enforces it on admission.
pub(super) const MAX_INTERACTIONS: usize = 16;

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

    /// Up to `MAX_QUESTIONS_SHOWN` of the questions being asked, as display text.
    ///
    /// This build has no way to *answer* a question (see [`super::Session::answer_interaction`]),
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
    pub(super) kind: InteractionKind,
    pub(super) reason: String,
    pub(super) surfaces: Vec<String>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::session::test_support::*;
    use crate::app::session::{FrameSink, Session};
    use crate::ipc::protocol::error_code;
    use crate::ipc::rpc::{Inbound, RpcError};
    use serde_json::json;

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
    #[test]
    fn dismissing_an_unknown_card_reports_that_nothing_happened() {
        let mut session = Session::new(identity());
        assert!(!session.dismiss_interaction("approval-9"));
    }
}

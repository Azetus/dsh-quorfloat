//! The requests the host hands to this panel, and what may be done about them.
//!
//! An interaction is not a message: it is a question with a lifetime. It arrives with an
//! id, it can be answered exactly once, the host's verdict decides whether the user may
//! try again, and it can expire. This module owns that lifetime, which is why it is
//! separate from [`super::Session`] — whose job is the wire, not the rules.
//!
//! Two kinds arrive, and this build answers both:
//!
//! - an **approval**, settled with one word from a closed vocabulary;
//! - a **question**, settled with the option labels the user selected — but only when
//!   the request is well-formed and small enough that nothing has to be hidden or
//!   guessed at (see [`Interaction::is_actionable`]). A request outside those caps is
//!   still reported so the user can see what was asked, and is handed back to the
//!   Harness window to answer there.
//!
//! The kind therefore travels on the wire instead of being inferred, and both
//! [`InteractionKind::Approval`] and [`InteractionKind::Question`] are what this build
//! names in [`crate::ipc::protocol::CAPABILITIES`].

use serde_json::Value;

/// Which waterfall an interaction came from.
///
/// The two are not variations of one thing: an approval is answered with a word
/// from a closed vocabulary, while a question is answered with the labels of the
/// options the user picked. That difference is why the kind travels on the wire
/// instead of being inferred, and why each kind has its own answer type
/// ([`ApprovalVerdict`] against [`QuestionAnswer`]).
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

/// One option a question offered.
///
/// The **label** is what travels back, not an id: the upstream answer shape carries
/// the text the user saw, so a renumbered option list cannot silently move an answer
/// onto a different choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionOption {
    /// The option's own text, which is also its wire value.
    pub label: String,
    /// The asker's explanation of the option, when it wrote one.
    pub description: Option<String>,
}

/// One question of a request, as the panel shows and answers it.
///
/// Only the fields an answer needs are modelled. Everything here is bounded by
/// `MAX_TEXT_CHARS` on the way in, because this text came from a model or a hook and
/// the panel is the surface that has to lay it out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    /// The id the answer must name.
    pub id: String,
    /// A short heading over the question, when the asker wrote one.
    pub header: Option<String>,
    /// The question itself.
    pub question: String,
    /// Extra context for the question, when the asker wrote any.
    pub detail: Option<String>,
    /// The options the answer may select from.
    pub options: Vec<QuestionOption>,
    /// Whether more than one option may be selected. Single-select is the default.
    pub multi_select: bool,
}

/// What the panel sent as the answer to one question.
///
/// This is the type the UI's choice is carried in, and it is separate from
/// [`Question`] because a question is what was *asked* while this is what was
/// *answered* — validation compares the two, which it could not do if they shared a
/// type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionAnswer {
    /// The id of the question this answers.
    pub id: String,
    /// The labels of the options the user selected.
    pub selected: Vec<String>,
    /// Free text for an "Other" answer, when the user typed one.
    pub custom: Option<String>,
}

/// What an answer this panel sent consisted of.
///
/// [`InteractionState::Submitting`] and [`InteractionState::Applied`] carry this rather
/// than a bare verdict so that a card can still say *what* was sent for either kind —
/// an approval can be re-read from one word, but a question's answer would otherwise be
/// lost the moment the frame left.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SentAnswer {
    /// An approval decision.
    Approval(ApprovalVerdict),
    /// The answers to a question card's questions.
    Question(Vec<QuestionAnswer>),
}

impl SentAnswer {
    /// How this answer reads in a breadcrumb.
    ///
    /// Public because both the marker written when the answer goes out and the one
    /// written when the host settles it must say the same thing about the same send —
    /// two summaries would be two chances to disagree.
    ///
    /// @returns a short, log-safe summary — the approval term, or how many questions
    ///   were answered, because a marker must not carry a model's text.
    #[must_use]
    pub fn summary(&self) -> String {
        match self {
            Self::Approval(verdict) => verdict.wire().to_owned(),
            Self::Question(answers) => format!("{} answered", answers.len()),
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
        answer: SentAnswer,
    },
    /// The host applied the answer. The end of the line: the grant is one-shot.
    Applied {
        /// What was applied.
        answer: SentAnswer,
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

/// How many questions of one request are summarised for a card that cannot be answered.
///
/// A report-only card is there to be *read*, so every extra question is height the user
/// scrolls past for a request they cannot settle anyway. Answerable cards use
/// `MAX_QUESTIONS_ANSWERED`, which is a correctness bound rather than a display one.
const MAX_QUESTIONS_SHOWN: usize = 3;

/// How many questions this build will answer in one card.
///
/// An answer has to cover everything the user was shown, and a form longer than this
/// stops being something a floating panel can present honestly. A request that exceeds
/// it is reported and handed back to the Harness window rather than half-answered.
const MAX_QUESTIONS_ANSWERED: usize = 4;

/// How many options one question may offer and still be answered here.
///
/// Beyond this the option list is truncated for layout, and answering a truncated list
/// would mean sending an answer to a question the user never fully saw.
const MAX_OPTIONS_PER_QUESTION: usize = 12;

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
    ///
    /// An approval is always actionable while it is pending — one word settles it and
    /// nothing about the payload can make that word a guess. A question is actionable
    /// only when it is **well-formed and small enough**; see [`Interaction::question_problem`]
    /// for what is checked and why an awkward request is refused rather than trimmed.
    ///
    /// @returns whether a click on this card can send an answer.
    #[must_use]
    pub fn is_actionable(&self) -> bool {
        self.state == InteractionState::Pending
            && match self.kind {
                InteractionKind::Approval => true,
                InteractionKind::Question => self.question_problem().is_none(),
            }
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

    /// The questions this card shows, bounded to what the panel can lay out.
    ///
    /// A card this build can answer is shown with **all** of its questions (capped at
    /// `MAX_QUESTIONS_ANSWERED`, which is also the answering cap, so an answer always
    /// covers exactly what the user saw). A card that cannot be answered is shown with
    /// at most `MAX_QUESTIONS_SHOWN`, because it exists to tell the user what was
    /// asked before they switch to the Harness window.
    ///
    /// @returns the questions in display order, capped as described; empty for an
    ///   approval or an unreadable payload.
    #[must_use]
    pub fn questions(&self) -> Vec<Question> {
        let cap = if self.question_problem().is_none() {
            MAX_QUESTIONS_ANSWERED
        } else {
            MAX_QUESTIONS_SHOWN
        };
        parse_questions(self.payload.get("questions")).into_iter().take(cap).collect()
    }

    /// How many questions the request contains, including ones not shown.
    ///
    /// @returns the raw count from the payload, or zero when it has no question list.
    #[must_use]
    pub fn question_count(&self) -> usize {
        self.payload.get("questions").and_then(Value::as_array).map_or(0, Vec::len)
    }

    /// Why this card's questions cannot be answered here, if they cannot.
    ///
    /// A question card is answerable only when nothing has to be hidden or guessed at:
    /// 1..=`MAX_QUESTIONS_ANSWERED` questions, every question carrying a non-empty id
    /// and non-empty question text, no two questions sharing an id, and at most
    /// `MAX_OPTIONS_PER_QUESTION` options per question. The rule behind all of them is
    /// the same — **an answer must cover what the user was shown** — so a request that
    /// would need a trimmed list, an invented id, or a guess at which of two
    /// identically-numbered questions was meant is handed back to the Harness window
    /// instead of being answered partially.
    ///
    /// @returns a marker line naming the reason, or `None` when the card is answerable.
    ///   The line is written to be read in a marker file: short, and never the model's
    ///   own text.
    #[must_use]
    pub fn question_problem(&self) -> Option<String> {
        let entries = match self.payload.get("questions") {
            Some(Value::Array(entries)) => entries,
            _ => return Some("no question list".to_owned()),
        };
        let mut problems: Vec<String> = Vec::new();
        if entries.is_empty() {
            problems.push("no questions".to_owned());
        } else if entries.len() > MAX_QUESTIONS_ANSWERED {
            problems.push(format!("{} questions (max {MAX_QUESTIONS_ANSWERED})", entries.len()));
        }
        for (index, entry) in entries.iter().enumerate() {
            let position = index + 1;
            let id = entry.get("id").and_then(Value::as_str).map(str::trim).unwrap_or_default();
            if id.is_empty() {
                problems.push(format!("question {position} has no id"));
            } else if entries[..index]
                .iter()
                .any(|earlier| earlier.get("id").and_then(Value::as_str).map(str::trim) == Some(id))
            {
                // Two questions under one id cannot be answered apart: the answer would
                // land on whichever was matched first, and the user's choice for the
                // other would silently vanish.
                problems.push(format!("id {id} is used twice"));
            }
            match entry.get("question").and_then(Value::as_str) {
                Some(text) if !text.trim().is_empty() => {}
                _ => problems.push(format!("question {position} has no text")),
            }
            if let Some(options) = entry.get("options").and_then(Value::as_array) {
                if options.len() > MAX_OPTIONS_PER_QUESTION {
                    problems.push(format!(
                        "question {position} offers {} options (max {MAX_OPTIONS_PER_QUESTION})",
                        options.len(),
                    ));
                }
            }
        }
        if problems.is_empty() {
            None
        } else {
            Some(problems.join("; "))
        }
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

/// Read one bounded, non-empty string, or `None`.
///
/// Anything that cannot be read as text with content in it is absent rather than an
/// empty string: the panel must not show a heading that is really whitespace, and
/// validation must not accept a question id of `""`.
///
/// @param value - the field, if any.
/// @returns the trimmed text, truncated for display.
#[must_use]
fn text(value: Option<&Value>) -> Option<String> {
    let text = value.and_then(Value::as_str)?.trim();
    if text.is_empty() {
        return None;
    }
    Some(truncate(text))
}

/// Read the questions out of a payload's `questions` field.
///
/// The list is built here, once, and both the display path and the validation path read
/// the same result — a validator that re-parsed the payload could disagree with what was
/// drawn, which is exactly the failure an answer must not be able to cause.
///
/// @param questions - the `questions` field, if any.
/// @returns every entry that can be built, in payload order. Entries are kept even when
///   they are unanswerable (a missing id, a missing question text): they are still what
///   the asker asked, and dropping them would make the card misreport the request.
///   Report-only cards are capped by [`Interaction::questions`], not here.
#[must_use]
fn parse_questions(questions: Option<&Value>) -> Vec<Question> {
    questions
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .map(|entry| {
                    let options = entry
                        .get("options")
                        .and_then(Value::as_array)
                        .map(|options| {
                            options
                                .iter()
                                .filter_map(|option| {
                                    Some(QuestionOption {
                                        label: text(option.get("label"))?,
                                        description: text(option.get("description")),
                                    })
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    Question {
                        id: entry.get("id").and_then(Value::as_str).unwrap_or_default().trim().to_owned(),
                        header: text(entry.get("header")),
                        question: text(entry.get("question")).unwrap_or_default(),
                        detail: text(entry.get("detail")),
                        options,
                        multi_select: entry.get("multiSelect").and_then(Value::as_bool).unwrap_or(false),
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parse the answer list a frontend or a peer sent.
///
/// Deliberately total: an unreadable answer becomes an empty selection rather than a
/// dropped question, so the answer is refused by validation *by name* ("q2 has no
/// selection and no custom text") instead of silently arriving short. The wire omits
/// `custom` when there is none, so an absent and a null `custom` mean the same thing.
///
/// @param answers - the `answers` array, if any.
/// @returns one [`QuestionAnswer`] per readable entry, in order.
#[must_use]
pub fn parse_answers(answers: Option<&Value>) -> Vec<QuestionAnswer> {
    answers
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .map(|entry| QuestionAnswer {
                    id: entry.get("id").and_then(Value::as_str).unwrap_or_default().to_owned(),
                    selected: entry
                        .get("selected")
                        .and_then(Value::as_array)
                        .map(|labels| {
                            labels.iter().filter_map(Value::as_str).map(str::to_owned).collect()
                        })
                        .unwrap_or_default(),
                    custom: entry
                        .get("custom")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|text| !text.is_empty())
                        .map(str::to_owned),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Whether one answer to one question covers it.
///
/// Three checks, all of them about the answer meaning what it says: the labels must be
/// ones this question offered (an unoffered label would tell the model the user chose
/// something that was never on screen), single-select questions must not carry several
/// labels at once, and the answer must not be empty. **`custom` may substitute for a
/// selection** — that is the "Other" answer — but the two together are still just one
/// answer.
///
/// @param question - the question that was asked.
/// @param answer - the answer to check.
/// @returns a marker line naming the first problem, or `None` when the answer is sound.
#[must_use]
fn answer_problem(question: &Question, answer: &QuestionAnswer) -> Option<String> {
    if answer.selected.is_empty() && answer.custom.is_none() {
        return Some(format!("question {} has no selection and no custom text", question.id));
    }
    if !question.multi_select && answer.selected.len() > 1 {
        return Some(format!("question {} is single-select but {} labels were sent", question.id, answer.selected.len()));
    }
    for label in &answer.selected {
        if !question.options.iter().any(|option| option.label == *label) {
            return Some(format!("question {} was not offered the label {label:?}", question.id));
        }
    }
    None
}

/// Check a whole answer list against the questions of the card it answers.
///
/// The UI requires a complete, offered answer before it enables the send button; this is
/// defence in depth for the same rule, because the frontend is not the only possible
/// caller of `answer_question`. Four refusals, in the order that makes the message most
/// useful:
///
/// - the card holds no answerable questions (report-only, or not a question at all);
/// - an answer names an id that is not one of the card's questions (a stale widget, or a
///   caller answering a different card);
/// - the same question is answered twice (two answers for one question would leave the
///   model to pick one);
/// - the answer does not cover its question — see `answer_problem`;
/// - the card has a question nothing answered.
///
/// @param questions - the card's questions, as parsed for display.
/// @param answers - what was sent.
/// @returns a marker line naming the first problem, or `None` when the answer is sound.
#[must_use]
pub fn validate_answers(questions: &[Question], answers: &[QuestionAnswer]) -> Option<String> {
    if questions.is_empty() {
        return Some("the card holds no answerable questions".to_owned());
    }
    if answers.is_empty() {
        return Some("no answers were sent".to_owned());
    }
    for (index, answer) in answers.iter().enumerate() {
        let Some(question) = questions.iter().find(|question| question.id == answer.id) else {
            return Some(format!("{:?} is not a question of this card", answer.id));
        };
        if answers[..index].iter().any(|earlier| earlier.id == answer.id) {
            return Some(format!("question {} was answered twice", answer.id));
        }
        if let Some(problem) = answer_problem(question, answer) {
            return Some(problem);
        }
    }
    for question in questions {
        if !answers.iter().any(|answer| answer.id == question.id) {
            return Some(format!("question {} was not answered", question.id));
        }
    }
    None
}

/// What [`Interaction::questions`] assumes about its two caps.
///
/// A card is treated as answerable only when it has at most `MAX_QUESTIONS_ANSWERED`
/// questions, so the answering view must never end up shorter than the reporting one —
/// otherwise a report-only card would show *more* questions than an answerable one, and
/// a user could be shown a question no answer can reach. Checked at compile time
/// because both bounds are constants and a broken relation is a build error, not a
/// runtime surprise.
const _: () = assert!(MAX_QUESTIONS_ANSWERED >= MAX_QUESTIONS_SHOWN);

// Upstream tripwires: the harness's `interaction/open` and `interaction/hint` payloads (2),
// plus the category 1 answer frame the host's validator reads.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::session::test_support::*;
    use crate::app::session::Session;
    use crate::ipc::rpc::Inbound;
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

        assert!(session.answer_interaction("approval-1", ApprovalVerdict::AllowOnce, &mut sink));
        let answer = sink.frames.last().expect("an answer was sent");
        assert_eq!(answer["method"], "interaction/answer");
        assert_eq!(answer["params"]["interactionId"], "approval-1");
        assert_eq!(answer["params"]["answer"]["outcome"], "allowed-once");
        assert!(answer["id"].is_number(), "an answer is a request, so it carries an id");
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
    fn a_question_open_is_decoded_with_ids_options_and_headers() {
        // The shape the panel draws and the frontend types against: the ids decide which
        // question an answer names, and `multiSelect` decides whether more than one option
        // may be picked.
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        deliver(&mut session, vec![well_formed_question_open()], &mut sink);
        let card = &session.interactions()[0];
        assert_eq!(card.kind(), InteractionKind::Question);
        assert_eq!(card.question_count(), 2);
        assert_eq!(
            card.questions(),
            vec![
                Question {
                    id: "q1".to_owned(),
                    header: Some("部署目标".to_owned()),
                    question: "部署到哪个环境？".to_owned(),
                    detail: Some("发布前的最后一个确认".to_owned()),
                    options: vec![
                        QuestionOption { label: "staging".to_owned(), description: Some("预发".to_owned()) },
                        QuestionOption { label: "production".to_owned(), description: None },
                    ],
                    multi_select: false,
                },
                Question {
                    id: "q2".to_owned(),
                    header: None,
                    question: "需要通知谁？".to_owned(),
                    detail: None,
                    options: vec![QuestionOption { label: "ops".to_owned(), description: None }],
                    multi_select: true,
                },
            ]
        );
    }

    /// Category 1: the answer frame the host's validator expects — question ids, option
    /// **labels**, and `custom` present only when the user typed free text.
    #[test]
    fn a_question_answer_names_the_labels_the_conversation_showed() {
        let mut session = Session::new(identity());
        let mut sink = RecordingSink::default();
        deliver(&mut session, vec![well_formed_question_open()], &mut sink);
        let answers = vec![
            QuestionAnswer { id: "q1".to_owned(), selected: vec!["staging".to_owned()], custom: None },
            QuestionAnswer { id: "q2".to_owned(), selected: vec!["ops".to_owned()], custom: Some("值班群".to_owned()) },
        ];
        assert!(session.answer_question("question-1", answers, &mut sink));
        let frame = sink.frames.last().expect("an answer was sent");
        assert_eq!(frame["method"], "interaction/answer");
        assert!(frame["id"].is_number(), "an answer is a request, so it carries an id");
        assert_eq!(frame["params"]["interactionId"], "question-1");
        assert_eq!(
            frame["params"]["answer"]["answers"],
            json!([
                {"id": "q1", "selected": ["staging"]},
                {"id": "q2", "selected": ["ops"], "custom": "值班群"},
            ])
        );
        assert!(
            frame["params"]["answer"]["answers"][0].get("custom").is_none(),
            "no custom text means the key is absent, not null: {frame}",
        );
    }

    /// The host's `interaction/open` for a well-formed two-question request.
    fn well_formed_question_open() -> Inbound {
        question_open(json!({"questions": [
            {
                "id": "q1",
                "header": "部署目标",
                "question": "部署到哪个环境？",
                "detail": "发布前的最后一个确认",
                "options": [
                    {"label": "staging", "description": "预发"},
                    {"label": "production"},
                ],
            },
            {
                "id": "q2",
                "question": "需要通知谁？",
                "options": [{"label": "ops"}],
                "multiSelect": true,
            },
        ]}))
    }

    /// The host's `interaction/open` for a question request with the given payload.
    fn question_open(payload: Value) -> Inbound {
        notification(
            "interaction/open",
            json!({
                "interactionId": "question-1",
                "sessionId": "session-1",
                "kind": "question",
                "payload": payload,
            }),
        )
    }
}

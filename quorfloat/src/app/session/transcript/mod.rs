//! The conversation, as lines the panel can draw.
//!
//! The host sends records; a record is an envelope around an event whose `type` is a
//! **free-form string** and nothing in the contract enumerates the kinds, so this module
//! is built around two rules:
//!
//! - **Known kinds are rendered; internal kinds are counted, not lost.** The ones a user
//!   would call conversation become entries. The ones that describe the machinery
//!   (`step/start`, `request/header`, …) are skipped, and the count is reported, because
//!   "we hid 40 events" is a very different statement from "nothing arrived".
//! - **Nothing but the conversation is drawn, and nothing is lost by it.** An unknown
//!   kind gets no line (the reader asked for a conversation window, not an event log),
//!   but it is *counted by kind and named in the marker*, so a harness
//!   upgrade still cannot look like silence: `transcript skipped: … 2 unrecognised
//!   (something/new, other/thing)`.
//!
//! Everything here is fed from frames that can arrive twice, out of order, from a
//! previous subscription, or not at all. The bookkeeping that makes that safe — `seq`
//! deduplication, generation checks, a bounded buffer, and a live stream that the final
//! record replaces — is the point of the module, and the reason it is tested without a
//! window.

use std::sync::Arc;

use serde_json::Value;

mod records;
mod stream;

use self::records::{content_blocks, find_call_id, message_text};
use self::stream::Live;

/// How many entries are kept. A floating panel is not an archive: past this, the oldest
/// are dropped and the fact that they were is part of the transcript.
const MAX_ENTRIES: usize = 400;

/// How much text one entry keeps. A tool result can be megabytes, and a panel that
/// allocates them all is a panel that dies on the one conversation that mattered.
const MAX_ENTRY_BYTES: usize = 32 * 1024;

/// One block inside an assistant message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// What the user sees as the answer.
    Text(String),
    /// The model's reasoning, shown dimmer and collapsible later.
    Reasoning(String),
    /// A tool the model asked for.
    Call {
        /// Tool name.
        name: String,
        /// Arguments, as the JSON text the model produced.
        arguments: String,
    },
}

/// One line of the conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// What the user sent.
    User {
        /// Message text.
        text: String,
    },
    /// What the model produced.
    Assistant {
        /// Blocks in order.
        blocks: Vec<Block>,
        /// Whether this is still being generated.
        streaming: bool,
    },
    /// What a tool returned.
    Tool {
        /// Tool name, when the call it answers is known.
        name: Option<String>,
        /// Output text.
        text: String,
        /// Whether the tool reported failure.
        is_error: bool,
    },
    /// A system or lifecycle line worth showing.
    System {
        /// Line text.
        text: String,
    },
}

/// The conversation as it stands, built only from frames the host sent.
#[derive(Debug, Clone, Default)]
pub struct Transcript {
    session_id: Option<String>,
    generation: Option<i64>,
    /// Highest `seq` folded in. Records at or below it are duplicates.
    cursor: i64,
    /// Whether a snapshot has been folded in; before that, events cannot be placed.
    seeded: bool,
    /// The lines, shared rather than copied.
    ///
    /// The window reads this every frame it paints, and a transcript is allowed to hold
    /// megabytes of tool output. Handing out a clone would copy all of it on every
    /// repaint — once a second even when nothing happens, because the clock that keeps a
    /// hidden panel alive also wakes the visible one. `Arc` makes that read free and
    /// makes the next write copy once.
    entries: Arc<Vec<Entry>>,
    /// The assistant message currently being generated, if any.
    live: Option<Live>,
    /// Tool names by call id, so a result can name the tool it belongs to.
    calls: Vec<(String, String)>,
    /// Title the harness chose, when it has announced one.
    title: Option<String>,
    /// Whether a turn is being worked on right now.
    ///
    /// Set by `turn/start` and cleared by `turn/end`, which are the host's own
    /// boundaries. The composer needs it to offer "stop" only when there is something to
    /// stop: a stop button in an idle conversation is a button that does nothing.
    turn_active: bool,
    dropped: usize,
    duplicates: usize,
    stale: usize,
    internal: usize,
    /// The `source.kind` values recognised as injected rather than typed.
    ///
    /// Kept so that a harness which starts injecting under a new name is discoverable: the
    /// filter would hide those messages, and silence is the one failure mode this layer must
    /// not have.
    injected: std::collections::BTreeSet<String>,
    unknown: usize,
    /// The kinds behind `unknown`, so hiding them from the panel does not hide them from
    /// whoever has to explain a harness upgrade.
    unknown_kinds: std::collections::BTreeSet<String>,
}

impl Transcript {
    /// A transcript for one session, empty until a snapshot arrives.
    #[must_use]
    pub fn new() -> Self {
        Self { cursor: -1, ..Self::default() }
    }

    /// The lines to draw, oldest first.
    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The same lines, for a caller that needs to keep them past the current frame.
    #[must_use]
    pub fn shared_entries(&self) -> Arc<Vec<Entry>> {
        Arc::clone(&self.entries)
    }

    /// The session this transcript describes, once known.
    #[must_use]
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// The harness's title for this conversation, once it has chosen one.
    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// Whether an answer is being generated right now.
    ///
    /// "There is a live buffer **with something in it**": [`Self::live_entry`] already hides
    /// an empty one, and a buffer holding nothing is not an answer — but the composer reads
    /// this flag to choose between send and stop: an empty buffer would otherwise leave it
    /// offering "stop" for a turn that was over.
    ///
    /// The buffer's own lifetime is what makes that enough: it is created by a stream frame
    /// and settled by the turn boundary (see the `turn/end` arm below), so a live buffer
    /// only exists while a turn is running.
    #[must_use]
    pub fn is_streaming(&self) -> bool {
        self.live.as_ref().is_some_and(|live| !live.is_empty())
    }

    /// Whether the host says a turn is in progress.
    #[must_use]
    pub fn is_turn_active(&self) -> bool {
        self.turn_active
    }

    /// Forget everything: the conversation, its title, and every line of it.
    ///
    /// For a conversation that no longer exists at the host. There is nothing worth keeping:
    /// those lines describe a conversation the panel is not following any more, and leaving
    /// them on screen would be showing the user something they cannot reply to.
    pub fn reset(&mut self) {
        self.session_id = None;
        self.entries = std::sync::Arc::new(Vec::new());
        self.live = None;
        self.title = None;
        self.turn_active = false;
    }

    /// The subscription generation the current entries belong to.
    #[must_use]
    pub fn generation(&self) -> Option<i64> {
        self.generation
    }

    /// The highest record sequence folded in.
    #[must_use]
    pub fn cursor(&self) -> i64 {
        self.cursor
    }

    /// What was set aside rather than rendered, for the log line.
    ///
    /// Reported because "the panel is quiet" has several causes that look identical on
    /// screen: nothing arrived, everything arrived twice, it all belonged to a replaced
    /// subscription, or it was internal. The counts are how those are told apart.
    /// The injected `source.kind` values this transcript has recognised.
    ///
    /// Empty in a session with no scaffolding in it. A name appearing here is not a problem in
    /// itself — it is the record of a decision the filter made for the reader to check.
    #[must_use]
    pub fn injected_kinds(&self) -> Vec<&str> {
        self.injected.iter().map(String::as_str).collect()
    }

    #[must_use]
    pub fn skipped(&self) -> Skipped {
        Skipped {
            duplicates: self.duplicates,
            stale: self.stale,
            internal: self.internal,
            unknown: self.unknown,
            unknown_kinds: self.unknown_kinds.iter().cloned().collect(),
            dropped: self.dropped,
        }
    }

    /// Forget everything, because the subscription this transcript described is gone.
    fn clear(&mut self) {
        Arc::make_mut(&mut self.entries).clear();
        self.live = None;
        self.calls.clear();
        self.title = None;
        self.cursor = -1;
        self.seeded = false;
        self.dropped = 0;
        self.turn_active = false;
    }

    /// Fold in a `session/snapshot`: the conversation as of a cursor.
    ///
    /// Replaces rather than appends. A snapshot is what the host says the conversation
    /// *is*, so merging it into whatever was on screen would keep entries the host has
    /// just contradicted.
    ///
    /// @param params - the notification parameters.
    pub fn apply_snapshot(&mut self, params: &Value) {
        let session_id = params.get("sessionId").and_then(Value::as_str);
        if session_id != self.session_id.as_deref() {
            self.clear();
        }
        self.session_id = session_id.map(str::to_owned);
        self.generation = params.get("generation").and_then(Value::as_i64);
        self.cursor = -1;
        self.seeded = true;
        // A snapshot that arrives while a stream is in flight replaces the stream too:
        // the stream described a subscription that no longer exists.
        self.live = None;
        for record in params.get("records").and_then(Value::as_array).into_iter().flatten() {
            self.fold_record(record);
        }
        // The cursor comes from the snapshot, not from the last record: an empty
        // snapshot still has a position, and treating it as "no records" would make the
        // next event look like a gap.
        if let Some(cursor) = params.get("cursor").and_then(Value::as_i64) {
            self.cursor = self.cursor.max(cursor);
        }
    }

    /// Fold in a `session/event`: one more record.
    ///
    /// @param params - the notification parameters.
    /// @returns whether it was folded in.
    pub fn apply_event(&mut self, params: &Value) -> bool {
        if !self.belongs(params) {
            return false;
        }
        let Some(seq) = params.get("seq").and_then(Value::as_i64) else { return false };
        if !self.seeded {
            // Nothing has said where this conversation starts. Counting it as accepted
            // would advance the cursor past records that will never be seen.
            self.stale += 1;
            return false;
        }
        if seq <= self.cursor {
            self.duplicates += 1;
            return true;
        }
        // Fold the event as if it were a record: an event carries the same fields, one
        // level up. Keeping one code path is what keeps live and replay identical.
        let record = serde_json::json!({
            "type": "event",
            "event": {
                "type": params.get("type").cloned().unwrap_or(Value::Null),
                "seq": seq,
                "time": params.get("time").cloned().unwrap_or(Value::Null),
                "data": params.get("data").cloned().unwrap_or(Value::Null),
            },
        });
        self.fold_record(&record);
        self.cursor = seq;
        true
    }

    /// Fold in a `session/stream`: part of the answer being generated.
    ///
    /// @param params - the notification parameters.
    /// @returns whether it was folded in.
    pub fn apply_stream(&mut self, params: &Value) -> bool {
        if !self.belongs(params) {
            return false;
        }
        let Some(frame) = params.get("frame") else { return false };
        // The live state is created on first use and reset by a `start` frame, which is
        // the frame that means "the previous attempt is gone".
        let live = self.live.get_or_insert_with(Live::default);
        self.unknown += live.fold(frame);
        true
    }

    /// Fold in a `session/resync`: the host lost the thread.
    ///
    /// Whatever is on screen is now known-incomplete, and this module cannot repair it
    /// by itself — the answer is a fresh subscription. What it *can* do is say so to the
    /// caller, which re-attaches **and puts the gap on the marker**: the panel draws no
    /// line for a gap, so the record is where "the history skipped a turn" has to live.
    ///
    /// @param params - the notification parameters.
    /// @returns the reason string, for the caller's log line.
    pub fn apply_resync(&mut self, params: &Value) -> Option<String> {
        let reason = params.get("reason").and_then(Value::as_str).unwrap_or("unknown");
        let expected = params.get("expected").and_then(Value::as_i64);
        let received = params.get("received").and_then(Value::as_i64);
        let detail = match (expected, received) {
            (Some(expected), Some(received)) => format!("{reason} (expected {expected}, received {received})"),
            _ => reason.to_owned(),
        };
        // Counted rather than drawn: it is one of the background events the panel hides.
        self.internal += 1;
        Some(detail)
    }

    /// Whether a frame belongs to the subscription this transcript is following.
    ///
    /// A frame from a replaced generation describes a conversation state nobody is
    /// following any more; counting it would make the panel show a mixture of two.
    fn belongs(&mut self, params: &Value) -> bool {
        let session_id = params.get("sessionId").and_then(Value::as_str);
        match (session_id, self.session_id.as_deref()) {
            (Some(frame), Some(mine)) if frame == mine => {}
            _ => {
                self.stale += 1;
                return false;
            }
        }
        if let Some(generation) = params.get("generation").and_then(Value::as_i64) {
            if self.generation.is_some_and(|current| current != generation) {
                self.stale += 1;
                return false;
            }
        }
        true
    }

    /// Fold one record envelope.
    fn fold_record(&mut self, record: &Value) {
        let Some(event) = record.get("event") else { return };
        let kind = event.get("type").and_then(Value::as_str).unwrap_or_default();
        let data = event.get("data").cloned().unwrap_or(Value::Null);
        if let Some(seq) = event.get("seq").and_then(Value::as_i64) {
            if seq <= self.cursor {
                self.duplicates += 1;
                return;
            }
            self.cursor = seq;
        }
        self.fold_event(kind, &data);
    }

    /// Turn one event into entries, or decide it is not something to draw.
    fn fold_event(&mut self, kind: &str, data: &Value) {
        match kind {
            "user/message" => {
                // The user's own words end any stream still on screen: the answer it
                // belonged to is over, whatever the stream has not been told.
                self.live = None;
                // …but only what the user actually said. The harness records its own
                // scaffolding in this same record type: a runtime-context snapshot — the file
                // policy, the runtime context, the tools available — arrives as a *user
                // message* whose `source.kind` is not `user`. Drawing it puts the harness's
                // internal instructions into the conversation as if they had been typed.
                if !from_the_user(data, &mut self.injected) {
                    self.internal += 1;
                    return;
                }
                let text = message_text(data);
                if !text.is_empty() {
                    self.push(Entry::User { text });
                }
            }
            "assistant/message" => self.push_message(data),
            // The system prompt, inserted into the history by the harness itself — its
            // `request/context` says `systemPromptUpdate: "in-history"`. The user never wrote
            // it and has no reason to read it back.
            "system/message" => self.internal += 1,
            "tool/call" => self.note_call(data),
            "tool/result" => self.push_tool_result(data),
            "session/title" => {
                if let Some(title) = data.get("title").and_then(Value::as_str) {
                    self.title = Some(title.to_owned());
                }
            }
            // A turn boundary is state rather than a line: the composer reads it, and a
            // rule drawn between every turn would be noise in a panel this short.
            "turn/start" => self.turn_active = true,
            "turn/end" => {
                self.turn_active = false;
                // The turn boundary settles whatever the stream left behind. A buffer with
                // nothing in it is dropped (a straggler frame creates one that would
                // otherwise sit there for the rest of the conversation), and an answer that
                // only ever arrived as deltas is kept as a settled entry — the turn is over,
                // so nothing about it may still say "正在生成".
                if let Some(live) = self.live.take() {
                    if !live.is_empty() {
                        self.push(Entry::Assistant { blocks: live.blocks().to_vec(), streaming: false });
                    }
                }
            }
            // Machinery. Counted, never drawn: a panel that shows `request/header` has stopped
            // being a conversation window.
            //
            // The list is the harness's own **known event vocabulary** (`known-event-types.ts`)
            // minus what this file handles above. Naming every one of them is the point: the
            // fallback below exists so a harness *upgrade* cannot look like silence, and it can only
            // do that job if the vocabulary it already knows is spelled out here. These kinds are
            // durable notes *about* the session rather than lines of the conversation — the approval
            // *card* is where a user answers, and `model/selection` belongs in the footer's picker —
            // so drawing them would put event log in the middle of a conversation.
            "approval/asked" | "approval/decided"
            | "step/start" | "step/end" | "command/run" | "command/done" | "workspace/changes"
            | "session/end-seed" | "permission/preset" | "sandbox/mode" | "approval/policy" | "agent/inbox/spliced"
            | "request/header" | "request/context" | "session/title-llm-request"
            | "model/selection" | "assistant/attempt" | "llm/retry" | "llm/retry-started"
            | "plan/mode" | "goal/change" | "schedule/change"
            | "compaction/start" | "compaction/end" | "compaction/summary" | "compaction/prune"
            | "subagent/catalog" | "subagent/descriptor" | "subagent/model-selection-policy"
            | "tool-workflow/agent-start" | "tool-workflow/agent-end"
            | "tool-workflow/run-start" | "tool-workflow/run-end"
            | "tool/ptc-dispatch" | "tool/ptc-dispatch-start"
            | "hook/invoked" | "hook/result" | "image/offload"
            | "agent-preset/selected"
            | "web/deepseek-search-llm-request" => {
                self.internal += 1;
            }
            // A kind this build has never seen. **Counted and named, never drawn**: the panel is a
            // conversation window, and the marker is where "the harness sent something we do not
            // understand" is answered.
            other => {
                if other.starts_with("session-log-") {
                    self.internal += 1;
                    return;
                }
                self.unknown += 1;
                self.unknown_kinds.insert(other.to_owned());
            }
        }
    }

    /// Fold an assistant message, replacing any live stream with what actually landed.
    fn push_message(&mut self, data: &Value) {
        self.live = None;
        let blocks = content_blocks(data);
        if blocks.is_empty() {
            self.internal += 1;
            return;
        }
        // A tool call announced by the message and again as its own event is one call,
        // not two; remember the ids so the second one does not add a second line.
        for block in &blocks {
            if let Block::Call { name, arguments } = block {
                if let Some(id) = find_call_id(data) {
                    self.remember_call(&id, name);
                }
                let _ = arguments;
            }
        }
        self.push(Entry::Assistant { blocks, streaming: false });
    }

    /// Fold a standalone `tool/call`, which the assistant message usually already shows.
    fn note_call(&mut self, data: &Value) {
        let name = data.get("name").and_then(Value::as_str).unwrap_or("unknown").to_owned();
        let arguments = data.get("arguments").and_then(Value::as_str).unwrap_or_default().to_owned();
        if let Some(id) = data.get("callId").and_then(Value::as_str) {
            self.remember_call(id, &name);
        }
        // Already drawn as part of the assistant message it belongs to. Adding it again
        // is the classic duplicate: same call, two bubbles, one conversation.
        if self.entries.iter().rev().take(4).any(|entry| {
            matches!(entry, Entry::Assistant { blocks, .. }
                if blocks.iter().any(|block| matches!(block, Block::Call { name: shown, .. } if shown == &name)))
        }) {
            return;
        }
        self.push(Entry::Assistant {
            blocks: vec![Block::Call { name, arguments }],
            streaming: false,
        });
    }

    /// Fold a tool result.
    fn push_tool_result(&mut self, data: &Value) {
        let message = data.get("message").cloned().unwrap_or(Value::Null);
        let is_error = message.get("isError").and_then(Value::as_bool).unwrap_or(false);
        let name = message
            .get("toolCallId")
            .and_then(Value::as_str)
            .and_then(|id| self.calls.iter().rev().find(|(call, _)| call == id).map(|(_, name)| name.clone()));
        let text = message_text(data);
        if text.is_empty() {
            self.internal += 1;
            return;
        }
        self.push(Entry::Tool { name, text, is_error });
    }

    fn push(&mut self, entry: Entry) {
        Arc::make_mut(&mut self.entries).push(bounded(entry));
        self.cap();
    }

    fn remember_call(&mut self, id: &str, name: &str) {
        if !self.calls.iter().any(|(call, _)| call == id) {
            self.calls.push((id.to_owned(), name.to_owned()));
        }
    }

    /// Enforce the buffer limit, keeping the newest.
    fn cap(&mut self) {
        if self.entries.len() <= MAX_ENTRIES {
            return;
        }
        let excess = self.entries.len() - MAX_ENTRIES;
        Arc::make_mut(&mut self.entries).drain(0..excess);
        self.dropped += excess;
    }

    /// The live stream as an entry, for a renderer that wants to draw it.
    #[must_use]
    pub fn live_entry(&self) -> Option<Entry> {
        let live = self.live.as_ref()?;
        if live.is_empty() {
            return None;
        }
        Some(Entry::Assistant { blocks: live.blocks().to_vec(), streaming: true })
    }
}

/// Keep one entry's text within the per-entry budget.
///
/// A tool result can be megabytes of build output, and a floating panel that holds all
/// of it is a panel that dies on the one command whose output mattered. The cut is
/// stated in the text rather than applied quietly, because a truncated result that looks
/// complete is worse than a long one.
fn bounded(entry: Entry) -> Entry {
    match entry {
        Entry::User { text } => Entry::User { text: clip(text) },
        Entry::System { text } => Entry::System { text: clip(text) },
        Entry::Tool { name, text, is_error } => Entry::Tool { name, text: clip(text), is_error },
        Entry::Assistant { blocks, streaming } => Entry::Assistant {
            blocks: blocks
                .into_iter()
                .map(|block| match block {
                    Block::Text(text) => Block::Text(clip(text)),
                    Block::Reasoning(text) => Block::Reasoning(clip(text)),
                    Block::Call { name, arguments } => Block::Call { name, arguments: clip(arguments) },
                })
                .collect(),
            streaming,
        },
    }
}

/// Cut a string to [`MAX_ENTRY_BYTES`], on a character boundary, saying what was cut.
fn clip(text: String) -> String {
    if text.len() <= MAX_ENTRY_BYTES {
        return text;
    }
    // UTF-8 boundaries, not byte offsets: slicing mid-character panics, and this runs on
    // whatever the model or a tool produced.
    let mut end = MAX_ENTRY_BYTES;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let dropped = text.len() - end;
    format!("{}\n…（已截断 {dropped} 字节；完整内容请在 Harness 窗口查看）", &text[..end])
}

/// Counts of what the transcript set aside.
/// Whether a `user/message` is something the user typed.
///
/// Read from `source.kind` rather than guessed from the text: a runtime-context snapshot is a
/// user message with `kind: "runtime-context"`, and guessing from wording would break the first
/// time the harness rephrases it. A message with no `source` at all is treated as the user's —
/// an unfamiliar harness should show too much rather than hide a real message.
///
/// @param data - the record's payload.
/// @param seen - collects the kinds recognised as injected, for diagnostics.
/// @returns whether it should be drawn as the user's own words.
fn from_the_user(data: &Value, seen: &mut std::collections::BTreeSet<String>) -> bool {
    match data
        .get("source")
        .and_then(|source| source.get("kind"))
        .and_then(Value::as_str)
    {
        Some("user") | None => true,
        Some(other) => {
            seen.insert(other.to_owned());
            false
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Skipped {
    /// Records at or below the cursor.
    pub duplicates: usize,
    /// Frames from another session, or a replaced generation.
    pub stale: usize,
    /// Machinery this build does not draw.
    pub internal: usize,
    /// Kinds this build does not recognise — counted, and named in `unknown_kinds`.
    pub unknown: usize,
    /// Those kinds by name, so the line says *what* arrived rather than only how much.
    pub unknown_kinds: Vec<String>,
    /// Entries dropped to stay within the buffer limit.
    pub dropped: usize,
}

impl Skipped {
    /// Whether anything was set aside.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.duplicates == 0 && self.stale == 0 && self.internal == 0 && self.unknown == 0 && self.dropped == 0
    }

    /// One line for the log, or `None` when nothing was skipped.
    #[must_use]
    pub fn describe(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let mut parts = Vec::new();
        if self.duplicates > 0 {
            parts.push(format!("{} duplicate", self.duplicates));
        }
        if self.stale > 0 {
            parts.push(format!("{} stale", self.stale));
        }
        if self.internal > 0 {
            parts.push(format!("{} internal", self.internal));
        }
        if self.unknown > 0 {
            parts.push(if self.unknown_kinds.is_empty() {
                format!("{} unrecognised", self.unknown)
            } else {
                format!("{} unrecognised ({})", self.unknown, self.unknown_kinds.join(", "))
            });
        }
        if self.dropped > 0 {
            parts.push(format!("{} dropped", self.dropped));
        }
        Some(format!("transcript skipped: {}", parts.join(", ")))
    }
}

// Upstream tripwires: the harness's `session/snapshot` records and `session/stream`
// frames, decoded exactly as the harness sends them (category 2).
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A record as the host sends it inside a snapshot.
    fn record(seq: i64, kind: &str, data: Value) -> Value {
        json!({"type": "event", "event": {"type": kind, "seq": seq, "time": 1_791_000_000_000i64, "data": data}})
    }

    /// A snapshot carrying the records, as `session/snapshot` does.
    fn snapshot(records: Vec<Value>, cursor: i64, generation: i64) -> Value {
        json!({"sessionId": "session-1", "generation": generation, "cursor": cursor,
               "hasMore": false, "records": records, "assistantStream": {"revision": 0}})
    }

    /// One live `session/stream` notification, wrapped as the host sends it.
    ///
    /// The frames below are verbatim host shapes, `attemptId`, `revision` and all —
    /// including the frame counter in `index` that is *not* the block index.
    fn live(frame: Value) -> Value {
        json!({"sessionId": "session-1", "generation": 1, "frame": frame})
    }

    #[test]
    fn a_snapshot_becomes_entries_in_order() {
        let mut transcript = Transcript::new();
        transcript.apply_snapshot(&snapshot(
            vec![
                // Different wrappers on purpose: this is what the harness sends.
                record(0, "user/message", json!({"role": "user", "content": [{"type": "text", "text": "你好"}]})),
                record(1, "assistant/message", json!({"message": {"role": "assistant",
                    "content": [{"type": "text", "text": "你好，有什么可以帮你"}]}})),
            ],
            1,
            1,
        ));

        assert_eq!(transcript.session_id(), Some("session-1"));
        assert_eq!(transcript.cursor(), 1);
        assert_eq!(transcript.entries().len(), 2);
        assert_eq!(transcript.entries()[0], Entry::User { text: "你好".to_owned() });
        let Entry::Assistant { blocks, streaming } = &transcript.entries()[1] else {
            panic!("expected an assistant entry: {:?}", transcript.entries()[1]);
        };
        assert_eq!(blocks, &vec![Block::Text("你好，有什么可以帮你".to_owned())]);
        assert!(!streaming);
    }

    #[test]
    fn a_user_message_is_read_from_the_unwrapped_payload_the_harness_sends() {
        // `user/message` puts content at `data.content`; every other message event wraps
        // it in `data.message`. Reading only the wrapped form loses every user turn, and
        // the loss looks like "the model answers questions nobody asked".
        let mut transcript = Transcript::new();
        transcript.apply_snapshot(&snapshot(
            vec![record(0, "user/message", json!({"role": "user", "id": "m-1",
                "content": [{"type": "text", "text": "直接提问"}], "source": {"kind": "user"}}))],
            0,
            1,
        ));
        assert_eq!(transcript.entries(), &[Entry::User { text: "直接提问".to_owned() }]);
    }

    #[test]
    fn a_live_stream_accumulates_text_deltas() {
        let mut transcript = Transcript::new();
        transcript.apply_snapshot(&snapshot(vec![], 0, 1));

        transcript.apply_stream(&live(json!({"type": "start", "attemptId": "session-1:1",
            "revision": 1, "startedAfterSeq": 251, "step": 1, "turn": 11})));
        transcript.apply_stream(&live(json!({"type": "chunk", "attemptId": "session-1:1", "revision": 2,
            "index": 0, "time": 1, "chunk": {"type": "block-start", "blockType": "text", "index": 0}})));
        transcript.apply_stream(&live(json!({"type": "chunk", "attemptId": "session-1:1", "revision": 3,
            "index": 1, "time": 2, "chunk": {"type": "text-delta", "index": 0, "text": "这是一"}})));
        transcript.apply_stream(&live(json!({"type": "chunk", "attemptId": "session-1:1", "revision": 4,
            "index": 2, "time": 3, "chunk": {"type": "text-delta", "index": 0, "text": "句话"}})));

        assert!(transcript.is_streaming());
        let Some(Entry::Assistant { blocks, streaming }) = transcript.live_entry() else { panic!("assistant") };
        assert!(streaming);
        assert_eq!(blocks, vec![Block::Text("这是一句话".to_owned())]);
    }
}

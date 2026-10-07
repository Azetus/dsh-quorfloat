//! The conversation, as lines the panel can draw.
//!
//! The host sends records; a record is an envelope around an event whose `type` is a
//! **free-form string** (`protocol.md` §5). One ordinary session produced 24 kinds, and
//! nothing in the contract enumerates them — so this module is built around two rules:
//!
//! - **Known kinds are rendered; internal kinds are counted, not lost.** The ones a user
//!   would call conversation become entries. The ones that describe the machinery
//!   (`step/start`, `request/header`, …) are skipped, and the count is reported, because
//!   "we hid 40 events" is a very different statement from "nothing arrived".
//! - **Nothing but the conversation is drawn, and nothing is lost by it.** An unknown
//!   kind gets no line (the reader asked for a conversation window, not an event log —
//!   2026-10-07), but it is *counted by kind and named in the marker*, so a harness
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

    /// The same allocation, for the test that proves lending is not copying.
    #[cfg(test)]
    fn entries_arc(&self) -> Arc<Vec<Entry>> {
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
    #[must_use]
    pub fn is_streaming(&self) -> bool {
        self.live.is_some()
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
    /// line for it any more (2026-10-07), so the record is where "the history skipped a
    /// turn" has to live.
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
            "turn/end" => self.turn_active = false,
            // Machinery. Counted, never drawn: a panel that shows `request/header` has stopped
            // being a conversation window.
            //
            // The list is the harness's own **known event vocabulary** (`known-event-types.ts`)
            // minus what this file handles above. Naming every one of them is the point: the
            // fallback below exists so a harness *upgrade* cannot look like silence, and it can only
            // do that job if the vocabulary it already knows is spelled out here. `model/selection`
            // is how this was found — it is the same kind of fact as `permission/preset` (a durable
            // note about the session, not a line of the conversation), and it was reaching the
            // transcript as `事件 · model/selection / 未识别的事件` every time the model changed.
            // The approval log is the same kind of fact: the *card* is where a user answers, and
            // once it is answered there is nothing to read — the tool result that follows carries
            // the consequence. Drawing "请求提权 / 提权请求：rejected" put two lines of event log in
            // the middle of a conversation (see `docs/progress.md` §4).
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
            // understand" is answered (`docs/AGENT.md` §4.4).
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

    /// A user message event, in the shape the harness really sends: content is *not*
    /// wrapped in a `message` object here, unlike every other message event.
    fn user_event(seq: i64, text: &str) -> Value {
        json!({"sessionId": "session-1", "generation": 1, "seq": seq, "type": "user/message", "time": 1,
               "data": {"role": "user", "id": "m-1", "content": [{"type": "text", "text": text}],
                        "source": {"kind": "user"}}})
    }

    /// An assistant message event.
    fn assistant_event(seq: i64, text: &str) -> Value {
        json!({"sessionId": "session-1", "generation": 1, "seq": seq, "type": "assistant/message", "time": 1,
               "data": {"message": {"role": "assistant", "id": "a-1",
                                    "content": [{"type": "reasoning", "text": "想一想"},
                                                {"type": "text", "text": text}]},
                        "step": 1, "turn": 1, "usage": {"outputTokens": 3}}})
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
    fn an_empty_snapshot_still_has_a_position() {
        // Treating "no records" as "no position" is how the first real event gets
        // misread as a duplicate or a gap.
        let mut transcript = Transcript::new();
        transcript.apply_snapshot(&snapshot(vec![], 41, 1));
        assert_eq!(transcript.cursor(), 41);
        assert!(transcript.apply_event(&user_event(42, "在吗")));
        assert_eq!(transcript.entries().len(), 1);
    }

    #[test]
    fn the_same_event_twice_is_one_entry() {
        let mut transcript = Transcript::new();
        transcript.apply_snapshot(&snapshot(vec![], 1, 1));
        assert!(transcript.apply_event(&user_event(2, "一次")));
        assert!(transcript.apply_event(&user_event(2, "一次")));
        assert_eq!(transcript.entries().len(), 1);
        assert_eq!(transcript.skipped().duplicates, 1);
    }

    #[test]
    fn records_inside_a_snapshot_are_deduplicated_too() {
        // The same record can appear both in a snapshot and as a live event, and the
        // snapshot wins because it is what the host says the conversation is.
        let mut transcript = Transcript::new();
        let duplicated = record(2, "user/message", json!({"content": [{"type": "text", "text": "重复"}]}));
        transcript.apply_snapshot(&snapshot(vec![duplicated.clone()], 2, 1));
        assert!(transcript.apply_event(&user_event(2, "重复")));
        assert_eq!(transcript.entries().len(), 1);
    }

    #[test]
    fn a_frame_from_another_session_is_not_folded_in() {
        let mut transcript = Transcript::new();
        transcript.apply_snapshot(&snapshot(vec![], 0, 1));
        let mut event = user_event(1, "别人的会话");
        event["sessionId"] = json!("session-2");
        assert!(!transcript.apply_event(&event));
        assert!(transcript.entries().is_empty());
        assert_eq!(transcript.skipped().stale, 1);
    }

    #[test]
    fn a_frame_from_a_replaced_generation_is_not_folded_in() {
        let mut transcript = Transcript::new();
        transcript.apply_snapshot(&snapshot(vec![], 0, 7));
        let mut event = user_event(1, "旧订阅");
        event["generation"] = json!(6);
        assert!(!transcript.apply_event(&event));
        assert!(transcript.entries().is_empty());
    }

    #[test]
    fn an_event_before_any_snapshot_is_refused_rather_than_placed() {
        let mut transcript = Transcript::new();
        assert!(!transcript.apply_event(&user_event(9, "无锚点")));
        assert!(transcript.entries().is_empty());
        assert_eq!(transcript.skipped().stale, 1);
    }

    #[test]
    fn a_snapshot_for_another_session_replaces_everything() {
        let mut transcript = Transcript::new();
        transcript.apply_snapshot(&snapshot(vec![record(0, "user/message", json!({"content": [{"type": "text", "text": "旧的"}]}))], 0, 1));
        assert_eq!(transcript.entries().len(), 1);

        let mut other = snapshot(vec![record(0, "user/message", json!({"content": [{"type": "text", "text": "新的"}]}))], 0, 1);
        other["sessionId"] = json!("session-2");
        transcript.apply_snapshot(&other);
        assert_eq!(transcript.session_id(), Some("session-2"));
        assert_eq!(transcript.entries(), &[Entry::User { text: "新的".to_owned() }]);
    }

    #[test]
    fn internal_kinds_are_counted_not_drawn() {
        let mut transcript = Transcript::new();
        transcript.apply_snapshot(&snapshot(
            vec![
                record(0, "step/start", json!({"step": 1, "turn": 1})),
                record(1, "request/header", json!({"header": {}})),
                record(2, "session-log-deepseek/delivery-accepted", json!({"throughSeq": 17})),
            ],
            2,
            1,
        ));
        assert!(transcript.entries().is_empty(), "{:?}", transcript.entries());
        assert_eq!(transcript.skipped().internal, 3);
    }

    #[test]
    fn changing_the_model_does_not_write_a_line_into_the_conversation() {
        // The bug this exists for: `model/selection` reached the transcript as
        // `事件 · model/selection / 未识别的事件` — twice, once per switch — because the reducer's
        // machinery list named only the types it happened to have met. A model change is a durable
        // note *about* the session, exactly like `permission/preset`, and belongs in the footer's
        // picker rather than in the conversation.
        let mut transcript = Transcript::new();
        transcript.apply_snapshot(&snapshot(
            vec![
                record(0, "model/selection", json!({"provider": "deepseek-official", "model": "deepseek-flash"})),
                record(1, "model/selection", json!({"provider": "deepseek-official", "model": "deepseek-v41", "reasoningEffort": "max"})),
            ],
            1,
            1,
        ));
        assert!(transcript.entries().is_empty(), "no line for a selection: {:?}", transcript.entries());
        assert_eq!(transcript.skipped().internal, 2);
        assert_eq!(transcript.skipped().unknown, 0, "and it is known, not merely unexplained");
    }

    #[test]
    fn the_harnesss_machinery_vocabulary_is_not_drawn() {
        // Every type the harness names in `known-event-types.ts` and this file does not handle.
        // Named one by one on purpose: the fallback notice below exists so a harness *upgrade*
        // cannot look like silence, and it can only do that if the vocabulary already known is
        // listed. A new harness event therefore appears here as a failing list, which is the
        // moment to decide whether it is a line of the conversation or machinery.
        const MACHINERY: &[&str] = &[
            "assistant/attempt", "llm/retry", "llm/retry-started", "model/selection",
            "plan/mode", "goal/change", "schedule/change", "agent-preset/selected",
            "compaction/start", "compaction/end", "compaction/summary", "compaction/prune",
            "subagent/catalog", "subagent/descriptor", "subagent/model-selection-policy",
            "tool-workflow/agent-start", "tool-workflow/agent-end",
            "tool-workflow/run-start", "tool-workflow/run-end",
            "tool/ptc-dispatch", "tool/ptc-dispatch-start",
            "hook/invoked", "hook/result", "image/offload",
            "web/deepseek-search-llm-request",
        ];
        let records: Vec<_> = MACHINERY
            .iter()
            .enumerate()
            .map(|(seq, kind)| record(seq as i64, kind, json!({"text": "不该出现的正文"})))
            .collect();
        let mut transcript = Transcript::new();
        transcript.apply_snapshot(&snapshot(records, MACHINERY.len() as i64 - 1, 1));
        assert!(
            transcript.entries().is_empty(),
            "machinery must not be drawn: {:?}",
            transcript.entries(),
        );
        assert_eq!(transcript.skipped().internal, MACHINERY.len());
        assert_eq!(transcript.skipped().unknown, 0, "all of it is known");
    }

    #[test]
    fn an_unrecognised_kind_is_named_in_the_log_rather_than_drawn() {
        // The rule that matters most: a harness upgrade must not look like silence. Since 2026-10-07
        // the answer lives in the marker instead of the thread — the panel is a conversation window,
        // and "we do not understand `something/new`" is a fact for whoever reads the log, not a line
        // for whoever is reading an answer.
        let mut transcript = Transcript::new();
        transcript.apply_snapshot(&snapshot(
            vec![record(0, "something/new", json!({"title": "新的东西"}))],
            0,
            1,
        ));
        assert!(transcript.entries().is_empty(), "nothing is drawn: {:?}", transcript.entries());
        let skipped = transcript.skipped();
        assert_eq!(skipped.unknown, 1);
        let described = skipped.describe().expect("a line for the log");
        assert!(described.contains("something/new"), "and it names the kind: {described}");
    }

    #[test]
    fn a_turn_in_progress_is_visible_to_the_composer() {
        // The composer offers "stop" only while this is true: a stop button in an idle
        // conversation is a button that does nothing.
        let mut transcript = Transcript::new();
        transcript.apply_snapshot(&snapshot(vec![], 3, 1));
        assert!(!transcript.is_turn_active());

        transcript.apply_event(&json!({"sessionId": "session-1", "generation": 1, "seq": 4,
            "type": "turn/start", "time": 1, "data": {"turn": 2}}));
        assert!(transcript.is_turn_active());

        transcript.apply_event(&json!({"sessionId": "session-1", "generation": 1, "seq": 5,
            "type": "turn/end", "time": 2, "data": {"turn": 2, "reason": {"kind": "completed"}}}));
        assert!(!transcript.is_turn_active());
    }

    #[test]
    fn a_turn_boundary_is_state_rather_than_a_line() {
        // Drawn, it would put a rule between every exchange in a panel this short.
        let mut transcript = Transcript::new();
        transcript.apply_snapshot(&snapshot(vec![], 3, 1));
        transcript.apply_event(&json!({"sessionId": "session-1", "generation": 1, "seq": 4,
            "type": "turn/end", "time": 2, "data": {"turn": 2, "reason": {"kind": "completed"}}}));
        assert!(transcript.entries().is_empty(), "{:?}", transcript.entries());
    }

    #[test]
    fn a_title_is_kept_out_of_the_transcript() {
        let mut transcript = Transcript::new();
        transcript.apply_snapshot(&snapshot(vec![record(0, "session/title", json!({"title": "字体与审批", "source": {"kind": "fallback"}}))], 0, 1));
        assert_eq!(transcript.title(), Some("字体与审批"));
        assert!(transcript.entries().is_empty());
    }

    #[test]
    fn a_tool_call_is_not_drawn_twice_when_its_message_already_showed_it() {
        let mut transcript = Transcript::new();
        transcript.apply_snapshot(&snapshot(
            vec![
                record(0, "assistant/message", json!({"message": {"role": "assistant", "content": [
                    {"type": "tool-call", "id": "call_1", "name": "bash", "arguments": "{\"command\":\"ls\"}"}]}})),
                record(1, "tool/call", json!({"name": "bash", "arguments": "{\"command\":\"ls\"}", "callId": "call_1"})),
            ],
            1,
            1,
        ));
        assert_eq!(transcript.entries().len(), 1, "{:?}", transcript.entries());
        let Entry::Assistant { blocks, .. } = &transcript.entries()[0] else { panic!("assistant") };
        assert_eq!(blocks.len(), 1);
    }

    #[test]
    fn a_tool_result_names_the_tool_that_produced_it() {
        let mut transcript = Transcript::new();
        transcript.apply_snapshot(&snapshot(
            vec![
                record(0, "tool/call", json!({"name": "bash", "arguments": "{}", "callId": "call_9"})),
                record(1, "tool/result", json!({"message": {"role": "tool", "toolCallId": "call_9", "isError": true,
                    "content": [{"type": "text", "text": "exit=1"}]}})),
            ],
            1,
            1,
        ));
        let Entry::Tool { name, text, is_error } = &transcript.entries()[1] else { panic!("tool") };
        assert_eq!(name.as_deref(), Some("bash"));
        assert_eq!(text, "exit=1");
        assert!(is_error);
    }

    /// The bug this filter was written for: the panel showed the harness's own instructions as
    /// if the user had typed them.
    #[test]
    fn injected_context_and_the_system_prompt_are_not_shown() {
        let mut transcript = Transcript::new();
        transcript.apply_snapshot(&snapshot(vec![
            // A real user message.
            record(0, "user/message", json!({
                "role": "user", "id": "m-1",
                "content": [{"type": "text", "text": "帮我看看这段代码"}],
                "source": {"kind": "user", "rpcId": "r-1"},
            })),
            // The runtime context: a user message by record type, scaffolding by source.
            record(1, "user/message", json!({
                "role": "user", "id": "m-2",
                "content": [{"type": "text", "text": "Current runtime context. This snapshot supersedes…"}],
                "source": {"kind": "runtime-context", "form": "snapshot"},
            })),
            // The system prompt, which the harness inserts into the history itself.
            record(2, "system/message", json!({
                "role": "system",
                "message": {"role": "system", "content": [{"type": "text", "text": "You are an AI agent powered by DeepSeek Harness."}]},
            })),
        ], 0, 1));

        let shown: Vec<String> = transcript
            .entries()
            .iter()
            .map(|entry| match entry {
                Entry::User { text } => text.clone(),
                other => format!("{other:?}"),
            })
            .collect();
        assert_eq!(shown, vec!["帮我看看这段代码".to_owned()], "only the user's own words");
        assert_eq!(transcript.skipped().internal, 2, "and the two scaffolding records were counted");
        assert_eq!(transcript.injected_kinds(), vec!["runtime-context"], "with the kind on the record");
    }

    /// An unfamiliar source is shown rather than hidden: hiding a real message is the worse
    /// mistake, and the kind is recorded so that the choice can be reviewed.
    #[test]
    fn a_message_with_no_source_is_still_the_users() {
        let mut transcript = Transcript::new();
        transcript.apply_snapshot(&snapshot(vec![
            record(0, "user/message", json!({
                "role": "user", "id": "m-1",
                "content": [{"type": "text", "text": "没有 source 的消息"}],
            })),
        ], 0, 1));
        assert_eq!(transcript.entries().len(), 1);
        assert!(matches!(&transcript.entries()[0], Entry::User { text } if text == "没有 source 的消息"));
        assert!(transcript.injected_kinds().is_empty());
    }

    #[test]
    fn the_buffer_keeps_the_newest_and_says_how_many_it_dropped() {
        let mut transcript = Transcript::new();
        let records = (0..(MAX_ENTRIES as i64 + 25))
            .map(|seq| record(seq, "user/message", json!({"content": [{"type": "text", "text": format!("第 {seq} 条")}]})))
            .collect();
        transcript.apply_snapshot(&snapshot(records, MAX_ENTRIES as i64 + 24, 1));

        assert_eq!(transcript.entries().len(), MAX_ENTRIES);
        assert_eq!(transcript.skipped().dropped, 25);
        assert_eq!(transcript.entries()[0], Entry::User { text: "第 25 条".to_owned() });
        assert!(transcript.skipped().describe().expect("a line").contains("25 dropped"));
    }

    /// One live `session/stream` notification, wrapped as the host sends it.
    ///
    /// The frames below are verbatim shapes from a real capture, `attemptId`, `revision`
    /// and all — including the frame counter in `index` that is *not* the block index.
    fn live(frame: Value) -> Value {
        json!({"sessionId": "session-1", "generation": 1, "frame": frame})
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

    #[test]
    fn the_frames_own_counter_is_not_the_block_index() {
        // The trap in the captured shape: a live frame's `index` counts frames (29 here),
        // while the block it describes is `chunk.index` (1). Reading the frame's counter
        // as the block index builds thirty placeholder blocks per token, and the answer
        // arrives in a panel full of blanks.
        let mut transcript = Transcript::new();
        transcript.apply_snapshot(&snapshot(vec![], 0, 1));
        transcript.apply_stream(&live(json!({"type": "chunk", "revision": 31, "index": 29,
            "chunk": {"type": "text-delta", "index": 1, "text": "这是一"}})));

        let Some(Entry::Assistant { blocks, .. }) = transcript.live_entry() else { panic!("assistant") };
        assert_eq!(blocks.len(), 2, "one placeholder for block 0 and the text at block 1: {blocks:?}");
        assert_eq!(blocks[1], Block::Text("这是一".to_owned()));
    }

    #[test]
    fn reasoning_deltas_land_in_a_reasoning_block() {
        let mut transcript = Transcript::new();
        transcript.apply_snapshot(&snapshot(vec![], 0, 1));
        transcript.apply_stream(&live(json!({"type": "chunk", "revision": 3, "index": 1,
            "chunk": {"type": "reasoning-delta", "index": 0, "text": "想一下"}})));
        let Some(Entry::Assistant { blocks, .. }) = transcript.live_entry() else { panic!("assistant") };
        assert_eq!(blocks, vec![Block::Reasoning("想一下".to_owned())]);
    }

    #[test]
    fn a_new_attempt_replaces_the_one_in_flight() {
        // A retried attempt must not append itself to the abandoned one: the user would
        // read two half-answers spliced together.
        let mut transcript = Transcript::new();
        transcript.apply_snapshot(&snapshot(vec![], 0, 1));
        transcript.apply_stream(&live(json!({"type": "chunk", "index": 1,
            "chunk": {"type": "text-delta", "index": 0, "text": "第一次"}})));
        transcript.apply_stream(&live(json!({"type": "start", "attemptId": "session-1:2",
            "revision": 9, "startedAfterSeq": 260, "step": 1, "turn": 12})));
        transcript.apply_stream(&live(json!({"type": "chunk", "index": 2,
            "chunk": {"type": "text-delta", "index": 0, "text": "第二次"}})));

        let Some(Entry::Assistant { blocks, .. }) = transcript.live_entry() else { panic!("assistant") };
        assert_eq!(blocks, vec![Block::Text("第二次".to_owned())]);
    }

    #[test]
    fn an_unrecognised_delta_kind_is_counted_rather_than_guessed() {
        // A tool call streaming is the shape this build has never captured. Guessing at
        // it would put invented text on screen; counting it sends someone to a capture.
        let mut transcript = Transcript::new();
        transcript.apply_snapshot(&snapshot(vec![], 0, 1));
        transcript.apply_stream(&live(json!({"type": "chunk", "index": 1,
            "chunk": {"type": "tool-input-delta", "index": 0, "delta": "{\"command\":"}})));
        assert_eq!(transcript.skipped().unknown, 1);
    }

    #[test]
    fn a_delta_without_its_start_frame_still_lands() {
        // Losing the block-start must not lose the text: a delta is better evidence of
        // what the block is than the absence of a frame is.
        let mut transcript = Transcript::new();
        transcript.apply_snapshot(&snapshot(vec![], 0, 1));
        transcript.apply_stream(&live(json!({"type": "chunk", "index": 1,
            "chunk": {"type": "reasoning-delta", "index": 0, "text": "边想边说"}})));
        let Some(Entry::Assistant { blocks, .. }) = transcript.live_entry() else { panic!("assistant") };
        assert_eq!(blocks, vec![Block::Reasoning("边想边说".to_owned())]);
    }

    #[test]
    fn a_finished_block_corrects_the_accumulated_deltas() {
        let mut transcript = Transcript::new();
        transcript.apply_snapshot(&snapshot(vec![], 0, 1));
        transcript.apply_stream(&live(json!({"type": "chunk", "index": 27,
            "chunk": {"type": "text-delta", "index": 0, "text": "半句"}})));
        transcript.apply_stream(&live(json!({"type": "chunk", "index": 28,
            "chunk": {"type": "block-end", "index": 0, "block": {"type": "text", "text": "完整的一句"}}})));
        let Some(Entry::Assistant { blocks, .. }) = transcript.live_entry() else { panic!("assistant") };
        assert_eq!(blocks, vec![Block::Text("完整的一句".to_owned())]);
    }

    #[test]
    fn the_record_replaces_the_stream_it_was_generating() {
        // Otherwise a finished answer is drawn twice: once from the stream, once from
        // the message that carries it.
        let mut transcript = Transcript::new();
        transcript.apply_snapshot(&snapshot(vec![], 0, 1));
        transcript.apply_stream(&live(json!({"type": "chunk", "index": 1,
            "chunk": {"type": "text-delta", "index": 0, "text": "临时"}})));
        assert!(transcript.is_streaming());

        transcript.apply_event(&assistant_event(1, "最终答案"));
        assert!(!transcript.is_streaming());
        assert_eq!(transcript.entries().len(), 1);
        assert_eq!(
            transcript.entries()[0],
            Entry::Assistant { blocks: vec![Block::Reasoning("想一想".to_owned()), Block::Text("最终答案".to_owned())], streaming: false },
        );
    }

    #[test]
    fn a_users_message_ends_a_stream_that_never_finished() {
        let mut transcript = Transcript::new();
        transcript.apply_snapshot(&snapshot(vec![], 0, 1));
        transcript.apply_stream(&live(json!({"type": "chunk", "index": 1,
            "chunk": {"type": "text-delta", "index": 0, "text": "没写完"}})));
        transcript.apply_event(&user_event(1, "算了"));
        assert!(!transcript.is_streaming(), "a new question means the old answer is over");
    }

    #[test]
    fn a_stream_from_a_replaced_generation_is_refused() {
        let mut transcript = Transcript::new();
        transcript.apply_snapshot(&snapshot(vec![], 0, 3));
        assert!(!transcript.apply_stream(&live(json!({"type": "chunk", "index": 1,
            "chunk": {"type": "text-delta", "index": 0, "text": "旧"}}))));
        assert!(!transcript.is_streaming());
    }

    #[test]
    fn a_resync_is_recorded_without_a_line_in_the_conversation() {
        let mut transcript = Transcript::new();
        transcript.apply_snapshot(&snapshot(vec![], 5, 1));
        let detail = transcript.apply_resync(&json!({"sessionId": "session-1", "generation": 1,
            "reason": "sequence-gap", "expected": 6, "received": 9}));
        assert_eq!(detail.as_deref(), Some("sequence-gap (expected 6, received 9)"));
        // The repair is automatic and the caller marks it; what the panel no longer does is write an
        // event line about it into the conversation.
        assert!(transcript.entries().is_empty(), "nothing is drawn: {:?}", transcript.entries());
        assert_eq!(transcript.skipped().internal, 1, "and it is counted as set aside");
    }

    #[test]
    fn a_huge_tool_result_is_cut_and_says_so() {
        let mut transcript = Transcript::new();
        let huge = "输".repeat(MAX_ENTRY_BYTES); // three bytes per character
        transcript.apply_snapshot(&snapshot(
            vec![record(0, "tool/result", json!({"message": {"role": "tool", "isError": false,
                "content": [{"type": "text", "text": huge}]}}))],
            0,
            1,
        ));
        let Entry::Tool { text, .. } = &transcript.entries()[0] else { panic!("tool") };
        assert!(text.len() < MAX_ENTRY_BYTES + 200, "{} bytes kept", text.len());
        assert!(text.contains("已截断"), "the cut is visible");
        // And it is still valid UTF-8 after being cut mid-way through the string.
        assert!(text.is_char_boundary(text.len()));
    }

    #[test]
    fn handing_out_the_lines_does_not_copy_them() {
        // The window reads this every frame; a clone here would be the whole transcript.
        let mut transcript = Transcript::new();
        transcript.apply_snapshot(&snapshot(
            vec![record(0, "user/message", json!({"content": [{"type": "text", "text": "共享"}]}))],
            0,
            1,
        ));
        let shared = transcript.shared_entries();
        assert_eq!(shared.len(), 1);
        // Same allocation, and appending afterwards leaves the reader's copy alone.
        assert!(Arc::ptr_eq(&shared, &transcript.entries_arc()));
        transcript.apply_event(&user_event(1, "后来的"));
        assert_eq!(shared.len(), 1, "the reader keeps what it read");
        assert_eq!(transcript.entries().len(), 2);
    }

    #[test]
    fn skipping_nothing_says_nothing() {
        let mut transcript = Transcript::new();
        transcript.apply_snapshot(&snapshot(vec![record(0, "user/message", json!({"content": [{"type": "text", "text": "嗨"}]}))], 0, 1));
        assert!(transcript.skipped().is_empty());
        assert_eq!(transcript.skipped().describe(), None);
    }
}

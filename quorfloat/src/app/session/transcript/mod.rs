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

pub mod assembly;
mod records;
mod stream;

use self::records::{content_blocks, find_call_id, message_text};
use self::stream::Live;

/// How much text the whole window keeps, measured in the bytes of the lines themselves.
///
/// **Bytes, not entries, and deliberately so.** This used to be a count (`400`), and a count
/// is the wrong unit for a window the user can *extend upwards*: fifty loaded messages of
/// ordinary prose and fifty of build output are the same number of entries and three orders
/// of magnitude apart in memory — and a count cap reaps the oldest entries first, which are
/// exactly the older pages the reader just asked for. The budget below is generous enough
/// that paging several pages of ordinary text never reaches it, and what it does reap is
/// reported (`Skipped::dropped`), never silent.
///
/// The bound that actually matters is the Harness's own: a loaded page does not outlive the
/// conversation it belongs to. When the panel is attached to another conversation the window
/// is replaced wholesale ([`Transcript::clear`], via `apply_snapshot`), and nothing here
/// caches a page across that.
const MAX_WINDOW_BYTES: usize = 4 * 1024 * 1024;

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
    /// Lowest `seq` the window can still show.
    ///
    /// The backwards cursor: this is what a page request asks below (`beforeSeq` in the
    /// Harness's own vocabulary), and it moves down as older pages are folded in above the
    /// current oldest line. `None` means the window holds no records at all.
    oldest: Option<i64>,
    /// Whether the host says history still precedes the oldest line on screen.
    ///
    /// Carried on every snapshot and on every page answer, and the only thing that decides
    /// whether the panel offers to load more: a control that is drawn when there is nothing
    /// behind it is a control that lies.
    has_more: bool,
    /// Bytes of entry text currently held, against [`MAX_WINDOW_BYTES`].
    ///
    /// Measured on every fold rather than recomputed: the window is read on every frame the
    /// panel paints, and summing a conversation's text on each read would be the one thing
    /// in this module that is linear in the conversation.
    bytes: usize,
    /// Whether an older page is being folded in right now.
    ///
    /// While this is set the byte budget does not reap: a page the reader asked for is not
    /// thrown away by the act of receiving it, however large it is. The budget resumes on the
    /// next live line, which is the growth it exists to bound.
    holding: bool,
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
    /// them on screen would be showing the user something they cannot reply to. The loaded
    /// pages go with them, which is the memory policy the Harness uses too: there is no page
    /// cache, so switching conversations re-reads the tail and nothing older.
    pub fn reset(&mut self) {
        self.session_id = None;
        self.entries = std::sync::Arc::new(Vec::new());
        self.live = None;
        self.title = None;
        self.turn_active = false;
        self.oldest = None;
        self.has_more = false;
        self.bytes = 0;
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

    /// The lowest sequence the window can still show, if any.
    ///
    /// The backwards cursor a page request is placed below. `None` when the window holds no
    /// records: there is nothing to ask below, and a request would be asking about a
    /// conversation this transcript has not been shown.
    #[must_use]
    pub fn oldest_seq(&self) -> Option<i64> {
        self.oldest
    }

    /// Whether the host says older history exists.
    #[must_use]
    pub fn has_more(&self) -> bool {
        self.has_more
    }

    /// Bytes of entry text currently held; for the log line and for tests.
    #[must_use]
    pub fn held_bytes(&self) -> usize {
        self.bytes
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
        self.oldest = None;
        self.has_more = false;
        self.bytes = 0;
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
        // Whether more history precedes this window is the host's fact, and it travels with
        // the snapshot (single frame or chunked `begin`). A snapshot that does not carry it
        // is read as "nothing older", which is the safe half: no control is drawn that would
        // ask for a page nobody said existed.
        self.has_more = params.get("hasMore").and_then(Value::as_bool).unwrap_or(false);
        // A snapshot that arrives while a stream is in flight replaces the stream too:
        // the stream described a subscription that no longer exists.
        self.live = None;
        for record in params.get("records").and_then(Value::as_array).into_iter().flatten() {
            self.fold_record(record);
        }
        // The backwards cursor is the oldest record the window now shows — the same field the
        // Harness's own client derives after a page (`records[0].event.seq`). Kept if the
        // window already had one: a repeated snapshot for the same conversation adds *newer*
        // records, and the oldest line on screen does not move.
        self.oldest = self.oldest.or_else(|| first_seq(params.get("records")));
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
        // A live event folded into a window that held nothing is the oldest line there is.
        // It does not move a window that already had lines: live events are newer than
        // everything already on screen.
        self.oldest = self.oldest.or(Some(seq));
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

    /// Fold one older page in **above** the current oldest line.
    ///
    /// This is the receiving half of `session/history`: the payload is the host's answer
    /// verbatim, `{ records, hasMore, cursor }`, where `cursor` is the backwards cursor for
    /// the page *below* this one (the Harness derives the same value from the page's oldest
    /// record). Three rules:
    ///
    /// - **Above, not after.** The page is older than everything on screen, so its entries go
    ///   in front of the existing ones; appending them would reorder the conversation.
    /// - **The forward watermark does not move.** The page's sequences are all below
    ///   `cursor`, and folding them through the ordinary path would count them as duplicates.
    ///   They are folded without touching the watermark, so a live event that has already been
    ///   applied stays applied.
    /// - **Only history is placed.** The scalars a page can lie about — the title, the turn
    ///   state, the live stream — are restored afterwards: an old page's `session/title` is
    ///   not today's title, and an old `turn/start` does not mean a turn is running now.
    ///
    /// @param page - the `session/history` result.
    /// @returns what folding the page did, for the record.
    pub fn prepend_page(&mut self, page: &Value) -> PrependOutcome {
        let records = page.get("records").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
        let has_more = page.get("hasMore").and_then(Value::as_bool).unwrap_or(false);
        let first = first_seq(page.get("records"));
        let last = last_seq(page.get("records"));
        // A page that is not strictly older than the oldest line on screen is a repeat (a
        // double click, a retried answer) or an answer to a superseded request. Folding it
        // would show the reader the same turns twice — the failure the dedup watermark exists
        // to prevent — so it is refused and counted, and the page's `hasMore` is still
        // believed: it describes the history, not this window.
        if let (Some(oldest), Some(last)) = (self.oldest, last) {
            if last >= oldest {
                self.duplicates += 1;
                self.has_more = has_more;
                return PrependOutcome { entries: 0, oldest: self.oldest, has_more, refused: true };
            }
        }
        if records.is_empty() {
            self.has_more = has_more;
            return PrependOutcome { entries: 0, oldest: self.oldest, has_more, refused: false };
        }
        // Side effects a historical page must not have: they describe the *present*.
        let title = self.title.take();
        let turn_active = self.turn_active;
        let live = self.live.take();
        let before = self.entries.len();
        self.holding = true;
        for record in records {
            self.fold_older_record(record);
        }
        self.holding = false;
        self.title = title;
        self.turn_active = turn_active;
        self.live = live;
        // Move what this page added from the end to the front, in one step: the fold path
        // appends because live records arrive after everything else, and the page is the one
        // case where the new lines go above.
        let held = std::mem::take(&mut self.entries);
        let mut entries = match Arc::try_unwrap(held) {
            Ok(entries) => entries,
            // A reader is holding the window (the shell shares it per frame): copy once
            // rather than mutate a buffer somebody else can see.
            Err(shared) => (*shared).clone(),
        };
        let mut older = entries.split_off(before);
        let added = older.len();
        older.append(&mut entries);
        self.entries = Arc::new(older);
        self.oldest = page.get("cursor").and_then(Value::as_i64).or(first);
        self.has_more = has_more;
        PrependOutcome { entries: added, oldest: self.oldest, has_more, refused: false }
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

    /// Fold one record envelope, enforcing the watermark.
    fn fold_record(&mut self, record: &Value) {
        let Some(event) = record.get("event") else { return };
        if let Some(seq) = event.get("seq").and_then(Value::as_i64) {
            if seq <= self.cursor {
                self.duplicates += 1;
                return;
            }
            self.cursor = seq;
        }
        self.fold_event_data(event);
    }

    /// Fold one *older* record envelope, without touching the watermark.
    ///
    /// The history path needs this: every record on an older page is below the cursor by
    /// construction, so [`Self::fold_record`]'s duplicate rule would set the whole page
    /// aside and the reader's request would arrive as nothing at all.
    fn fold_older_record(&mut self, record: &Value) {
        let Some(event) = record.get("event") else { return };
        self.fold_event_data(event);
    }

    /// Turn one event envelope into entries, or decide it is not something to draw.
    fn fold_event_data(&mut self, event: &Value) {
        let kind = event.get("type").and_then(Value::as_str).unwrap_or_default();
        let data = event.get("data").cloned().unwrap_or(Value::Null);
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
        let entry = bounded(entry);
        self.bytes += entry_bytes(&entry);
        Arc::make_mut(&mut self.entries).push(entry);
        self.cap();
    }

    fn remember_call(&mut self, id: &str, name: &str) {
        if !self.calls.iter().any(|(call, _)| call == id) {
            self.calls.push((id.to_owned(), name.to_owned()));
        }
    }

    /// Enforce the byte budget, dropping the oldest text.
    ///
    /// Measured in bytes rather than entries (see [`MAX_WINDOW_BYTES`]), and the newest line
    /// is never dropped: a window that trimmed itself to nothing would show a blank panel for
    /// a conversation that is not blank. What is dropped is counted, so the marker says so.
    fn cap(&mut self) {
        if self.holding || self.bytes <= MAX_WINDOW_BYTES {
            return;
        }
        let entries = Arc::make_mut(&mut self.entries);
        let mut excess = 0usize;
        let mut freed = 0usize;
        while excess < entries.len().saturating_sub(1) && self.bytes - freed > MAX_WINDOW_BYTES {
            freed += entry_bytes(&entries[excess]);
            excess += 1;
        }
        if excess == 0 {
            return;
        }
        entries.drain(0..excess);
        self.bytes -= freed;
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

/// What folding one older page did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrependOutcome {
    /// Lines added above the oldest line the window held.
    pub entries: usize,
    /// The oldest sequence the window can show afterwards, if any.
    pub oldest: Option<i64>,
    /// Whether the host says still older history remains.
    pub has_more: bool,
    /// Whether the page was refused because it was not older than what is on screen.
    pub refused: bool,
}

impl PrependOutcome {
    /// One line for the log, so a page that arrived as nothing says why.
    #[must_use]
    pub fn describe(&self) -> String {
        let oldest = self.oldest.map_or("none".to_owned(), |seq| seq.to_string());
        if self.refused {
            return format!("older page dropped: not older than what is shown (oldest={oldest})");
        }
        format!(
            "older page: {} entries, oldest={oldest}, hasMore={}",
            self.entries, self.has_more,
        )
    }
}

/// The bytes of text one entry holds, which is what the window budget measures.
///
/// The text *is* the entry: nothing here counts the fixed cost of an enum or a `Vec`, because
/// what a floating panel has to bound is the megabytes a tool result can carry, and the text
/// is the only part that is not bounded by the shape of the type.
fn entry_bytes(entry: &Entry) -> usize {
    match entry {
        Entry::User { text } | Entry::System { text } => text.len(),
        Entry::Tool { name, text, .. } => name.as_ref().map_or(0, String::len) + text.len(),
        Entry::Assistant { blocks, .. } => blocks
            .iter()
            .map(|block| match block {
                Block::Text(text) | Block::Reasoning(text) => text.len(),
                Block::Call { name, arguments } => name.len() + arguments.len(),
            })
            .sum(),
    }
}

/// The sequence of the oldest record in a wire `records` array, if it carries one.
///
/// Read from the first record rather than the smallest seq: pages are chronological, and this
/// is the same derivation the Harness's own client makes (`records[0].event.seq`) when it
/// decides what to send as the next `beforeSeq`.
fn first_seq(records: Option<&Value>) -> Option<i64> {
    records
        .and_then(Value::as_array)
        .and_then(|records| records.first())
        .and_then(|record| record.get("event"))
        .and_then(|event| event.get("seq"))
        .and_then(Value::as_i64)
}

/// The sequence of the newest record in a wire `records` array, if it carries one.
fn last_seq(records: Option<&Value>) -> Option<i64> {
    records
        .and_then(Value::as_array)
        .and_then(|records| records.last())
        .and_then(|record| record.get("event"))
        .and_then(|event| event.get("seq"))
        .and_then(Value::as_i64)
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
    /// Lines dropped to stay within the window's byte budget. Never silent: a reader whose
    /// older page was reaped has to be able to find out why.
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

    /// One `session/history` answer, as the host plugin sends it.
    fn page(records: Vec<Value>, has_more: bool, cursor: Option<i64>) -> Value {
        json!({"records": records, "hasMore": has_more, "cursor": cursor})
    }

    /// One user line, for building pages and snapshots readably.
    fn user(seq: i64, text: &str) -> Value {
        record(seq, "user/message", json!({"role": "user", "content": [{"type": "text", "text": text}]}))
    }

    #[test]
    fn an_older_page_is_folded_above_the_window_and_moves_the_backwards_cursor() {
        // The window the panel already has: the tail page, 50..51, with history below it.
        let mut transcript = Transcript::new();
        transcript.apply_snapshot(&json!({
            "sessionId": "session-1", "generation": 1, "cursor": 51, "hasMore": true,
            "records": [user(50, "新问题"), user(51, "新回答")],
            "assistantStream": {"revision": 0},
        }));
        assert_eq!(transcript.oldest_seq(), Some(50));
        assert!(transcript.has_more());

        // `session/history { beforeSeq: 50 }`: the page below it.
        let outcome = transcript.prepend_page(&page(vec![user(48, "旧问题"), user(49, "旧回答")], true, Some(48)));
        assert!(!outcome.refused);
        assert_eq!(outcome.entries, 2);
        assert_eq!(outcome.oldest, Some(48));
        assert!(outcome.has_more);

        // Above, not after: the older lines are first, and the forward watermark has not moved
        // (a live event that has already been applied stays applied).
        assert_eq!(
            transcript.entries(),
            &[
                Entry::User { text: "旧问题".to_owned() },
                Entry::User { text: "旧回答".to_owned() },
                Entry::User { text: "新问题".to_owned() },
                Entry::User { text: "新回答".to_owned() },
            ],
        );
        assert_eq!(transcript.cursor(), 51);
        assert_eq!(transcript.oldest_seq(), Some(48), "the next request asks below 48");
        assert!(transcript.has_more());

        // The last page: `hasMore: false` retires the affordance.
        let outcome = transcript.prepend_page(&page(vec![user(46, "最早")], false, Some(46)));
        assert_eq!(outcome.entries, 1);
        assert_eq!(transcript.oldest_seq(), Some(46));
        assert!(!transcript.has_more());
    }

    #[test]
    fn a_page_that_is_not_older_is_refused_rather_than_shown_twice() {
        // The failure a repeated answer or a second click would produce: the same turns drawn
        // twice. The dedup watermark does not cover this path — older records are below it by
        // definition — so the page itself is refused.
        let mut transcript = Transcript::new();
        transcript.apply_snapshot(&snapshot(vec![user(50, "新问题"), user(51, "新回答")], 51, 1));
        let answer = page(vec![user(48, "旧问题"), user(49, "旧回答")], true, Some(48));
        assert_eq!(transcript.prepend_page(&answer).entries, 2);
        let before = transcript.entries().to_vec();

        // The same page a second time, and a page that overlaps the window.
        let repeat = transcript.prepend_page(&answer);
        assert!(repeat.refused);
        assert_eq!(repeat.entries, 0);
        assert!(transcript.prepend_page(&page(vec![user(49, "旧回答"), user(50, "新问题")], true, Some(49))).refused);
        assert_eq!(transcript.entries(), before.as_slice(), "nothing was folded twice");
        assert_eq!(transcript.oldest_seq(), Some(48), "and the cursor did not move");
    }

    #[test]
    fn one_entry_larger_than_the_per_entry_budget_is_clipped_and_says_so() {
        // The receiving half of the host's per-record page clamp: whatever arrives, one line
        // keeps at most `MAX_ENTRY_BYTES` of text. The mirror lives on the host side
        // (`HISTORY_RECORD_BUDGET_BYTES` in `src/harness/session-layer.ts`, the same 32 KiB), so a
        // record clamped before it was sent costs this side nothing. The text is CJK on purpose:
        // a cut at a byte offset would panic on a character boundary, which is why the clip walks
        // back to one.
        let mut transcript = Transcript::new();
        let text = "中".repeat(MAX_ENTRY_BYTES / 3 + 1);
        transcript.apply_snapshot(&snapshot(vec![user(0, &text)], 0, 1));

        let Entry::User { text: kept } = &transcript.entries()[0] else {
            panic!("expected the user entry: {:?}", transcript.entries()[0]);
        };
        assert!(kept.starts_with('中'), "the kept text is the beginning of what was sent");
        let cut = kept.find('\n').expect("the marker starts its own line");
        assert!(cut <= MAX_ENTRY_BYTES, "the kept prefix is within the per-entry budget: {cut}");
        assert!(kept[cut..].contains("已截断"), "and it states what was dropped rather than looking complete");
        assert_eq!(transcript.held_bytes(), kept.len(), "the window budget measures what was kept");
    }

    #[test]
    fn the_window_is_bounded_by_bytes_and_reports_what_it_dropped() {
        // The retention policy, in one assertion: a count cap would have reaped the oldest
        // lines (the pages the reader just loaded) after a fixed number of entries, whatever
        // they weighed. The budget is bytes, and what it drops is on the record.
        let mut transcript = Transcript::new();
        transcript.apply_snapshot(&snapshot(vec![user(0, "起点")], 0, 1));
        let line = "中".repeat(MAX_ENTRY_BYTES / 3);
        // Live growth, well past the window budget: `bounded` clips each line at the per-entry
        // budget, so this is what paging one enormous conversation actually looks like. Twice
        // as many lines as fit, plus the crossing one.
        let lines = 2 * MAX_WINDOW_BYTES / MAX_ENTRY_BYTES + 8;
        for index in 0..lines {
            let seq = index as i64 + 1;
            transcript.apply_event(&json!({"sessionId": "session-1", "generation": 1, "seq": seq,
                "type": "assistant/message", "time": 1,
                "data": {"message": {"role": "assistant", "content": [{"type": "text", "text": line}]}}}));
        }
        assert!(transcript.skipped().dropped > 0, "the budget reaped, and says so");
        assert!(
            transcript.held_bytes() <= MAX_WINDOW_BYTES + MAX_ENTRY_BYTES,
            "within the budget plus the line that crossed it: {}",
            transcript.held_bytes(),
        );
        // The newest line survives whatever the budget did.
        assert_eq!(transcript.cursor(), lines as i64);
    }
}

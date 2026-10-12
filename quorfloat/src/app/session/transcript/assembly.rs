//! Rebuilding one snapshot from the parts it arrives in.
//!
//! `session/snapshot` was one frame, and one frame has a ceiling: the host's NDJSON
//! cap is 1 MiB on both sides, and a conversation whose records add up to more than
//! that was dropped by the reader before any of it could be folded. The transcript
//! then never became seeded, every later `session/event` was counted stale, and the
//! panel stayed blank for the rest of the run with nothing on the record to say why.
//!
//! Chunking is the fix, and this is the receiving half of it: the host sends
//! `session/snapshot/begin` (identity and meta), one or more `session/snapshot/part`
//! (bounded slices of records), and `session/snapshot/end`. This module holds the
//! parts until the sequence is complete and then hands back **the same payload shape
//! the single-frame snapshot had**, so the existing fold path is the only code that
//! ever turns records into a conversation.
//!
//! Three rules make the rebuild safe:
//!
//! - **Identity, not arrival order.** Parts name the conversation (and generation)
//!   they belong to, so two subscriptions being opened at once cannot be glued into
//!   one another's records.
//! - **Nothing is folded half-way.** A missing part means the sequence is dropped and
//!   the host is asked again; a conversation folded from a partial history would
//!   silently present an invented one.
//! - **Bounded.** The number of parts per snapshot and the number of open snapshots
//!   are both capped, so a peer that never finishes cannot make this process grow.

use serde_json::{Map, Value, json};

/// How long an open assembly waits for its remaining parts.
///
/// Generous, because the parts are written to a pipe and the host may be busy
/// building them: it is a bound on a stuck sequence, not a latency budget.
pub const ASSEMBLY_TIMEOUT_MS: i64 = 10_000;

/// The most parts one snapshot may declare.
///
/// With a 256 KiB budget this is two orders of magnitude more than a 50-message
/// history needs; it exists so a peer that names an absurd count cannot make this
/// process size a vector from it.
pub const MAX_PARTS: usize = 4_096;

/// What one `session/snapshot/part` turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// Stored at the index it named.
    Stored,
    /// No open assembly names that conversation.
    Unknown,
    /// The part repeated an index that already arrived, or carried no usable index.
    Refused,
}

/// One snapshot being rebuilt from `begin` + `part`* + `end`.
#[derive(Debug, Clone)]
pub struct Assembly {
    session_id: String,
    generation: Option<i64>,
    /// The meta the single-frame snapshot used to carry: cursor, hasMore,
    /// projections, assistantStream. Kept verbatim so the fold path sees exactly the
    /// payload it always saw.
    meta: Map<String, Value>,
    /// Parts by index; `None` until that index arrives.
    parts: Vec<Option<Vec<Value>>>,
    /// How many parts have arrived, which is *not* `parts.len()`: an index that
    /// arrives out of order leaves gaps in the vector.
    received: usize,
    /// How many parts the `begin` frame announced, when it named a number.
    expected: Option<usize>,
    /// The largest part measured so far, for the marker line.
    bytes_max: usize,
    /// When `begin` arrived, against the local clock.
    started_at: i64,
}

impl Assembly {
    /// Start an assembly from a `session/snapshot/begin` payload.
    ///
    /// @param params - the begin notification's parameters.
    /// @param now - caller-local time in milliseconds, for the timeout.
    /// @returns the assembly, or `None` when the payload names no conversation —
    ///   parts could not be matched to anything without one.
    #[must_use]
    pub fn begin(params: &Value, now: i64) -> Option<Self> {
        let session_id = params.get("sessionId").and_then(Value::as_str)?.to_owned();
        let generation = params.get("generation").and_then(Value::as_i64);
        let expected = params
            .get("parts")
            .and_then(Value::as_u64)
            .map(|count| (count as usize).min(MAX_PARTS));
        // Everything the begin carried except the fields this type owns, so a future
        // host can add meta without this side having to learn about it.
        let mut meta = params.as_object().cloned().unwrap_or_default();
        meta.remove("sessionId");
        meta.remove("generation");
        meta.remove("parts");
        Some(Self {
            session_id,
            generation,
            meta,
            parts: Vec::new(),
            received: 0,
            expected,
            bytes_max: 0,
            started_at: now,
        })
    }

    /// The conversation the parts describe.
    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// When this assembly started, against the local clock.
    #[must_use]
    pub fn started_at(&self) -> i64 {
        self.started_at
    }

    /// How many parts have arrived.
    #[must_use]
    pub fn received(&self) -> usize {
        self.received
    }

    /// The largest part's measured size in bytes.
    #[must_use]
    pub fn bytes_max(&self) -> usize {
        self.bytes_max
    }

    /// Whether this payload belongs to the snapshot being assembled here.
    ///
    /// The conversation is required to match; a generation is only compared when
    /// both sides name one, so a part from a host that omitted it is still placed.
    #[must_use]
    pub fn accepts(&self, params: &Value) -> bool {
        if params.get("sessionId").and_then(Value::as_str) != Some(self.session_id.as_str()) {
            return false;
        }
        match (params.get("generation").and_then(Value::as_i64), self.generation) {
            (Some(frame), Some(mine)) => frame == mine,
            _ => true,
        }
    }

    /// Store one `session/snapshot/part`.
    ///
    /// @param params - the part notification's parameters.
    /// @returns whether it was stored, belonged nowhere, or was refused.
    pub fn part(&mut self, params: &Value) -> Part {
        if !self.accepts(params) {
            return Part::Unknown;
        }
        let Some(index) = params.get("index").and_then(Value::as_u64).map(|value| value as usize) else {
            return Part::Refused;
        };
        if index >= MAX_PARTS {
            return Part::Refused;
        }
        if index >= self.parts.len() {
            self.parts.resize(index + 1, None);
        }
        if self.parts[index].is_some() {
            // A repeat is not an error worth a line per part, and the first copy of a
            // part is the one that stands: overwriting could not be distinguished
            // from a host that re-sent a corrected slice.
            return Part::Refused;
        }
        let records = params.get("records").and_then(Value::as_array).cloned().unwrap_or_default();
        self.parts[index] = Some(records);
        self.received += 1;
        // Measured on the payload as it arrived; the frame's own envelope is a
        // constant this side of the budget, so this is the number worth reporting.
        self.bytes_max = self.bytes_max.max(serde_json::to_vec(params).map_or(0, |bytes| bytes.len()));
        Part::Stored
    }

    /// Whether every part the sequence declared has arrived.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        match self.expected {
            Some(expected) => {
                self.received == expected
                    && self.parts.len() <= expected
                    && self.parts.iter().all(Option::is_some)
            }
            // No declared count: complete when every index that arrived has no hole
            // before it and at least one part exists. An empty snapshot with no count
            // cannot be told from a lost one, so it is not treated as complete.
            None => self.received > 0 && self.parts.iter().all(Option::is_some),
        }
    }

    /// The first index that never arrived, when the sequence is incomplete.
    #[must_use]
    pub fn missing(&self) -> Option<usize> {
        if self.is_complete() {
            return None;
        }
        let end = self.expected.unwrap_or(self.parts.len());
        for index in 0..end {
            if self.parts.get(index).is_none_or(Option::is_none) {
                return Some(index);
            }
        }
        Some(end)
    }

    /// The single-frame `session/snapshot` payload these parts stand for.
    ///
    /// `None` while the sequence is incomplete: a fold is all-or-nothing, because a
    /// conversation built from half a history is one nobody can tell from a complete
    /// one.
    #[must_use]
    pub fn fold_params(&self) -> Option<Value> {
        if !self.is_complete() {
            return None;
        }
        let mut params = self.meta.clone();
        params.insert("sessionId".to_owned(), Value::String(self.session_id.clone()));
        if let Some(generation) = self.generation {
            params.insert("generation".to_owned(), json!(generation));
        }
        let records: Vec<Value> = self.parts.iter().flatten().flatten().cloned().collect();
        params.insert("records".to_owned(), Value::Array(records));
        Some(Value::Object(params))
    }

    /// One phrase for a marker line: enough to tell which snapshot was dropped and
    /// how far it got.
    #[must_use]
    pub fn describe(&self) -> String {
        match self.expected {
            Some(expected) => {
                format!("{}: {}/{} parts, {} bytes max", self.session_id, self.received, expected, self.bytes_max)
            }
            None => format!("{}: {} parts, {} bytes max", self.session_id, self.received, self.bytes_max),
        }
    }
}

// Protocol contract: the begin/part/end sequence rebuilds exactly the payload the
// single-frame snapshot carried, and a hole is never folded.
#[cfg(test)]
mod tests {
    use super::*;

    fn begin(parts: u64) -> Value {
        json!({
            "sessionId": "session-1",
            "generation": 3,
            "cursor": 9,
            "hasMore": false,
            "parts": parts,
            "assistantStream": {"revision": 0},
        })
    }

    fn part(index: usize, records: Value) -> Value {
        json!({"sessionId": "session-1", "generation": 3, "index": index, "records": records})
    }

    #[test]
    fn parts_rebuild_the_payload_the_single_frame_snapshot_carried() {
        let mut assembly = Assembly::begin(&begin(2), 0).expect("a begin with an identity");
        assert_eq!(assembly.part(&part(0, json!([{"event": {"seq": 1}}]))), Part::Stored);
        assert_eq!(assembly.part(&part(1, json!([{"event": {"seq": 2}}]))), Part::Stored);
        assert!(assembly.is_complete());

        let folded = assembly.fold_params().expect("a complete sequence folds");
        assert_eq!(folded["sessionId"], "session-1");
        assert_eq!(folded["generation"], 3);
        assert_eq!(folded["cursor"], 9, "the meta from begin survives");
        assert_eq!(folded["records"].as_array().map(Vec::len), Some(2), "in index order");
        assert_eq!(folded["records"][0]["event"]["seq"], 1);
        assert_eq!(folded["records"][1]["event"]["seq"], 2);
    }

    #[test]
    fn a_hole_keeps_the_sequence_incomplete() {
        let mut assembly = Assembly::begin(&begin(3), 0).expect("a begin");
        assert_eq!(assembly.part(&part(0, json!([]))), Part::Stored);
        assert_eq!(assembly.part(&part(2, json!([]))), Part::Stored);
        assert!(!assembly.is_complete());
        assert_eq!(assembly.missing(), Some(1), "the first hole is named");
        assert!(assembly.fold_params().is_none(), "an incomplete sequence is never folded");
    }

    #[test]
    fn a_part_for_another_conversation_is_not_stored() {
        let mut assembly = Assembly::begin(&begin(1), 0).expect("a begin");
        let foreign = json!({"sessionId": "other", "generation": 3, "index": 0, "records": []});
        assert_eq!(assembly.part(&foreign), Part::Unknown);
        assert!(!assembly.is_complete());
    }

    #[test]
    fn a_repeated_part_does_not_count_twice() {
        let mut assembly = Assembly::begin(&begin(2), 0).expect("a begin");
        assert_eq!(assembly.part(&part(0, json!([]))), Part::Stored);
        assert_eq!(assembly.part(&part(0, json!([]))), Part::Refused);
        assert_eq!(assembly.received(), 1);
    }

    #[test]
    fn an_empty_snapshot_completes_with_no_parts() {
        // An empty conversation still has a cursor, and the fold has to run for the
        // transcript to become seeded: zero parts is a complete sequence.
        let assembly = Assembly::begin(&begin(0), 0).expect("a begin");
        assert!(assembly.is_complete());
        assert_eq!(assembly.missing(), None);
        let folded = assembly.fold_params().expect("the empty payload folds");
        assert_eq!(folded["records"].as_array().map(Vec::len), Some(0));
    }
}

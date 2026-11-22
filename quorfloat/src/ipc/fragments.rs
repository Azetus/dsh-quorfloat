//! Transport fragmentation for logical JSON-RPC messages larger than one frame.
//!
//! The envelope is not a business message: the receiver reassembles the original JSON
//! text before classifying it. Sequence numbers, cursors, and record shapes therefore
//! never see the fragments.
//!
//! Each fragment is Base64 so the raw bytes survive JSON string escaping. A sequence
//! is accepted only in order and only while its total stays inside
//! [`MAX_LOGICAL_MESSAGE_BYTES`].

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};

use crate::ipc::protocol::{
    FRAGMENT_METHOD, FRAGMENT_RAW_BYTES, FRAGMENT_TIMEOUT_MS, MAX_LOGICAL_MESSAGE_BYTES,
};

/// Why a fragment sequence was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FragmentRejection {
    /// The parameters were not a fragment envelope.
    InvalidParams,
    /// The sequence did not start at zero, arrived out of order, or changed shape.
    OutOfOrder,
    /// The total exceeded [`MAX_LOGICAL_MESSAGE_BYTES`].
    TooLarge,
    /// The reassembled bytes were not a JSON value.
    NotJson,
    /// The sequence did not finish inside [`FRAGMENT_TIMEOUT_MS`].
    TimedOut,
}

impl FragmentRejection {
    /// A stable log label.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidParams => "invalid fragment parameters",
            Self::OutOfOrder => "fragment sequence out of order",
            Self::TooLarge => "fragment sequence exceeds the logical message limit",
            Self::NotJson => "reassembled fragment is not JSON",
            Self::TimedOut => "fragment sequence timed out",
        }
    }
}

/// What one accepted fragment produced.
#[derive(Debug, Clone, PartialEq)]
pub enum FragmentOutcome {
    /// More fragments are needed.
    Pending,
    /// The sequence was refused; it is dropped.
    Rejected(FragmentRejection),
    /// The complete logical JSON value.
    Complete(Value),
}

#[derive(Debug)]
struct Pending {
    id: String,
    count: usize,
    chunks: Vec<Option<Vec<u8>>>,
    bytes: usize,
    deadline: i64,
}

/// Reassembles one fragmented message at a time.
#[derive(Debug, Default)]
pub struct FragmentAssembler {
    pending: Option<Pending>,
}

impl FragmentAssembler {
    /// Start with no sequence in flight.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop an incomplete sequence without reporting it.
    pub fn reset(&mut self) {
        self.pending = None;
    }

    /// Feed one `quorfloat/fragment` notification.
    ///
    /// @param params - the notification parameters.
    /// @param now - caller-local milliseconds, used for the assembly deadline.
    /// @returns pending, a rejection, or the complete JSON value.
    pub fn accept(&mut self, params: &Value, now: i64) -> FragmentOutcome {
        if self.pending.as_ref().is_some_and(|pending| now > pending.deadline) {
            self.pending = None;
            return FragmentOutcome::Rejected(FragmentRejection::TimedOut);
        }
        let Some((id, index, count, data)) = read_params(params) else {
            return FragmentOutcome::Rejected(FragmentRejection::InvalidParams);
        };
        if self.pending.as_ref().is_some_and(|pending| pending.id != id) {
            self.pending = None;
        }
        if self.pending.is_none() {
            if index != 0 || count == 0 || count > MAX_LOGICAL_MESSAGE_BYTES / FRAGMENT_RAW_BYTES + 1 {
                return FragmentOutcome::Rejected(FragmentRejection::InvalidParams);
            }
            self.pending = Some(Pending {
                id,
                count,
                chunks: vec![None; count],
                bytes: 0,
                deadline: now.saturating_add(FRAGMENT_TIMEOUT_MS),
            });
        }
        let Some(pending) = self.pending.as_mut() else {
            return FragmentOutcome::Rejected(FragmentRejection::InvalidParams);
        };
        if pending.count != count || index >= count || pending.chunks[index].is_some() {
            self.pending = None;
            return FragmentOutcome::Rejected(FragmentRejection::OutOfOrder);
        }
        let Ok(chunk) = STANDARD.decode(data) else {
            self.pending = None;
            return FragmentOutcome::Rejected(FragmentRejection::InvalidParams);
        };
        pending.bytes = pending.bytes.saturating_add(chunk.len());
        if pending.bytes > MAX_LOGICAL_MESSAGE_BYTES {
            self.pending = None;
            return FragmentOutcome::Rejected(FragmentRejection::TooLarge);
        }
        pending.chunks[index] = Some(chunk);
        if pending.chunks.iter().any(Option::is_none) {
            return FragmentOutcome::Pending;
        }
        let pending = self.pending.take().expect("the sequence was just seen");
        let mut bytes = Vec::with_capacity(pending.bytes);
        for chunk in pending.chunks.into_iter().flatten() {
            bytes.extend_from_slice(&chunk);
        }
        match serde_json::from_slice(&bytes) {
            Ok(value) => FragmentOutcome::Complete(value),
            Err(_) => FragmentOutcome::Rejected(FragmentRejection::NotJson),
        }
    }
}

/// Split one JSON value into fragment notifications.
///
/// @param message - the logical JSON-RPC message.
/// @param id - a channel-unique sequence id.
/// @returns the fragment notifications, in order.
///
/// @throws String when the message exceeds the logical limit or cannot be serialized.
pub fn fragment_messages(message: &Value, id: &str) -> Result<Vec<Value>, String> {
    let bytes = serde_json::to_vec(message).map_err(|error| error.to_string())?;
    if bytes.len() > MAX_LOGICAL_MESSAGE_BYTES {
        return Err(format!("message exceeds {MAX_LOGICAL_MESSAGE_BYTES} bytes"));
    }
    let count = bytes.len().div_ceil(FRAGMENT_RAW_BYTES).max(1);
    let mut out = Vec::with_capacity(count);
    for index in 0..count {
        let start = index * FRAGMENT_RAW_BYTES;
        let end = (start + FRAGMENT_RAW_BYTES).min(bytes.len());
        out.push(json!({
            "jsonrpc": "2.0",
            "method": FRAGMENT_METHOD,
            "params": {
                "id": id,
                "index": index,
                "count": count,
                "data": STANDARD.encode(&bytes[start..end]),
            },
        }));
    }
    Ok(out)
}

/// Read one fragment envelope.
fn read_params(params: &Value) -> Option<(String, usize, usize, &str)> {
    let object = params.as_object()?;
    let id = object.get("id")?.as_str()?.to_owned();
    let index = usize::try_from(object.get("index")?.as_u64()?).ok()?;
    let count = usize::try_from(object.get("count")?.as_u64()?).ok()?;
    let data = object.get("data")?.as_str()?;
    if id.is_empty() {
        return None;
    }
    Some((id, index, count, data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_split_message_reassembles_and_keeps_its_shape() {
        let message = json!({ "jsonrpc": "2.0", "id": 7, "method": "session/prompt", "params": { "text": "x".repeat(FRAGMENT_RAW_BYTES + 32) } });
        let fragments = fragment_messages(&message, "f-1").expect("fragments");
        assert_eq!(fragments.len(), 2);
        let mut assembler = FragmentAssembler::new();
        assert_eq!(assembler.accept(&fragments[0]["params"], 0), FragmentOutcome::Pending);
        assert_eq!(assembler.accept(&fragments[1]["params"], 1), FragmentOutcome::Complete(message));
    }

    #[test]
    fn an_out_of_order_fragment_is_refused() {
        let message = json!({ "jsonrpc": "2.0", "id": 7, "method": "session/prompt", "params": { "text": "x".repeat(FRAGMENT_RAW_BYTES + 32) } });
        let fragments = fragment_messages(&message, "f-2").expect("fragments");
        let mut assembler = FragmentAssembler::new();
        assert_eq!(assembler.accept(&fragments[1]["params"], 0), FragmentOutcome::Rejected(FragmentRejection::InvalidParams));
    }

    #[test]
    fn an_incomplete_sequence_times_out() {
        let message = json!({ "jsonrpc": "2.0", "id": 7, "method": "session/prompt", "params": { "text": "x".repeat(FRAGMENT_RAW_BYTES + 32) } });
        let fragments = fragment_messages(&message, "f-3").expect("fragments");
        let mut assembler = FragmentAssembler::new();
        assert_eq!(assembler.accept(&fragments[0]["params"], 0), FragmentOutcome::Pending);
        assert_eq!(
            assembler.accept(&fragments[1]["params"], FRAGMENT_TIMEOUT_MS + 1),
            FragmentOutcome::Rejected(FragmentRejection::TimedOut),
        );
    }
}

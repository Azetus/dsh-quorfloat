//! Reading the harness's payload shapes.
//!
//! Every function here is pure: it takes one JSON value from a record and returns what
//! the transcript should say about it. They live apart from the reducer because they are
//! the part that was *learned* rather than designed — each one encodes a shape that was
//! captured from a real session, and two of them encode a trap:
//!
//! - **`user/message` does not wrap its content in a `message` object**, while
//!   `assistant/message`, `system/message` and `tool/result` all do. Reading only the
//!   wrapped form loses every user turn, which on screen looks like a model answering
//!   questions nobody asked.
//! - **`arguments` is a JSON string, not an object**, because that is what the model
//!   produced and the harness forwards verbatim.
//!
//! See `docs/protocol.md` §5 for the captured shapes.

use serde_json::Value;

use super::Block;

/// A stream frame's finished block.
pub(super) fn stream_block(block: &Value) -> Option<Block> {
    match block.get("type").and_then(Value::as_str)? {
        "text" => Some(Block::Text(block.get("text").and_then(Value::as_str)?.to_owned())),
        "reasoning" => Some(Block::Reasoning(block.get("text").and_then(Value::as_str)?.to_owned())),
        _ => None,
    }
}

/// The text of a `data.message`, joined from its content blocks.
pub(super) fn message_text(data: &Value) -> String {
    content_blocks(data)
        .into_iter()
        .filter_map(|block| match block {
            Block::Text(text) | Block::Reasoning(text) => Some(text),
            Block::Call { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The content blocks of an event's payload.
///
/// **The harness is not uniform about where they live**, and the difference was found
/// by capturing real frames rather than by reading the contract:
///
/// - `user/message` carries them directly: `data.content`;
/// - `assistant/message`, `system/message` and `tool/result` wrap them:
///   `data.message.content`.
///
/// Reading only the wrapped form silently drops every user message — the conversation
/// renders with the questions missing and the answers unexplained. Both are accepted
/// here so that either shape works, and a test pins the user one specifically.
pub(super) fn content_blocks(data: &Value) -> Vec<Block> {
    let content = data
        .get("message")
        .and_then(|message| message.get("content"))
        .or_else(|| data.get("content"))
        .and_then(Value::as_array);
    content.into_iter().flatten().filter_map(block_of).collect()
}

/// One content block.
pub(super) fn block_of(block: &Value) -> Option<Block> {
    match block.get("type").and_then(Value::as_str)? {
        "text" => Some(Block::Text(block.get("text").and_then(Value::as_str)?.to_owned())),
        "reasoning" => Some(Block::Reasoning(block.get("text").and_then(Value::as_str)?.to_owned())),
        "tool-call" => Some(Block::Call {
            name: block.get("name").and_then(Value::as_str).unwrap_or("unknown").to_owned(),
            arguments: block.get("arguments").and_then(Value::as_str).unwrap_or_default().to_owned(),
        }),
        _ => None,
    }
}

/// The tool-call id inside a message, when there is exactly one.
pub(super) fn find_call_id(data: &Value) -> Option<String> {
    data.get("message")
        .and_then(|message| message.get("content"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find_map(|block| block.get("id").and_then(Value::as_str))
        .map(str::to_owned)
}

/// A short description of an unrecognised event, so its line says more than its name.
pub(super) fn summarize(data: &Value) -> String {
    for key in ["title", "text", "message", "reason", "id", "name"] {
        if let Some(value) = data.get(key) {
            if let Some(text) = value.as_str() {
                return text.chars().take(120).collect();
            }
        }
    }
    String::new()
}

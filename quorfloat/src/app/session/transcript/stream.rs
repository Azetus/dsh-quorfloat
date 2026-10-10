//! The answer as it is being written.
//!
//! Separate from the transcript because it is the one part with a lifetime shorter than
//! a record: blocks accumulate here while the model is talking, and the finished record
//! replaces all of it. Its state is private to this file for that reason — nothing else
//! should be able to half-update an answer.
//!
//! **The live frames are not the shape of the recorded replay**, and assuming they are
//! produces a panel that shows nothing until the answer is over. The live delta is
//! `{type:"chunk", chunk:{type:"text-delta", index, text}}` while the replay is
//! `{type:"text-chunks", index, texts:[…]}`. The block index is `chunk.index` live and
//! `frame.index` in a replay, and a live frame's own `index` is a **frame counter** —
//! treating it as a block index builds one placeholder block per token.
//!
//! Only the live shape is handled, because only the live shape is ever sent here: an
//! assistant message is rendered from its finished content blocks, never replayed.

use serde_json::Value;

use super::Block;
use super::records::stream_block;

/// A partially received assistant message.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Live {
    /// Blocks by index.
    blocks: Vec<Block>,
}

impl Live {
    /// The blocks accumulated so far.
    pub(super) fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// Whether anything has arrived yet.
    pub(super) fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Fold one stream frame into the message being generated.
    ///
    /// **The live frames are not the shape of the recorded replay**, and assuming they
    /// are produces a panel that shows nothing until the answer is already over:
    ///
    /// | | live `session/stream` | replay in `assistant/message.data.stream[]` |
    /// |---|---|---|
    /// | deltas | `{type:"chunk", chunk:{type:"text-delta", index, text}}` | `{type:"text-chunks", index, texts:[…]}` |
    /// | block index | `chunk.index` | `frame.index` |
    /// | extra | `start` (with `attemptId`/`revision`), `end` | — |
    ///
    /// Only the live shape is handled, because only the live shape is ever sent here: an
    /// assistant message is rendered from its finished content blocks, never replayed.
    ///
    /// @param frame - one frame from a `session/stream` notification.
    /// Fold one frame in.
    ///
    /// @param frame - one `session/stream` frame.
    /// @returns how many frames carried a kind this build does not know, for the
    ///   transcript to count. Guessing at them would put invented text on screen.
    pub(super) fn fold(&mut self, frame: &Value) -> usize {
        let mut unknown = 0;
        match frame.get("type").and_then(Value::as_str).unwrap_or_default() {
            // A new attempt replaces the one in flight: the previous attempt was
            // abandoned or retried, and its half-written blocks describe an answer that
            // will never be committed.
            "start" => *self = Self::default(),
            // The outcome says the message landed at a sequence; the record that follows
            // is what replaces this stream on screen, so there is nothing to do here.
            "end" => {}
            "chunk" => {
                let chunk = frame.get("chunk").cloned().unwrap_or(Value::Null);
                // `chunk.index` is the block; the frame's own `index` is a counter over
                // frames, and using it would create one placeholder block per token.
                let index = chunk
                    .get("index")
                    .or_else(|| frame.get("index"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                match chunk.get("type").and_then(Value::as_str).unwrap_or_default() {
                    "block-start" => {
                        let block = match chunk.get("blockType").and_then(Value::as_str).unwrap_or_default() {
                            "reasoning" => Block::Reasoning(String::new()),
                            "tool-call" => Block::Call { name: "unknown".to_owned(), arguments: String::new() },
                            _ => Block::Text(String::new()),
                        };
                        self.set_block(index, block);
                    }
                    "text-delta" => {
                        if let Some(text) = chunk.get("text").and_then(Value::as_str) {
                            self.append_delta(index, false, text);
                        }
                    }
                    "reasoning-delta" => {
                        if let Some(text) = chunk.get("text").and_then(Value::as_str) {
                            self.append_delta(index, true, text);
                        }
                    }
                    // The complete block, which is authoritative: the accumulated deltas
                    // are a guess about text that can be corrected here.
                    "block-end" => {
                        if let Some(block) = chunk.get("block").and_then(stream_block) {
                            self.set_block(index, block);
                        }
                    }
                    // Usage and the finish reason describe the attempt, not its text;
                    // the record that follows carries both.
                    "usage" | "finish" => {}
                    // A delta kind this build has never seen. Counted rather than drawn:
                    // a token-by-token line would be noise, and the counter is what sends
                    // someone to a capture to find out what it was.
                    _ => unknown += 1,
                }
            }
            _ => unknown += 1,
        }
        unknown
    }

    /// Put a block in place, replacing whatever placeholder stood there.
    fn set_block(&mut self, index: usize, block: Block) {
        while self.blocks.len() <= index {
            self.blocks.push(Block::Text(String::new()));
        }
        self.blocks[index] = block;
    }

    /// Append incremental text to the live block at `index`.
    ///
    /// Creates the block when the start frame was lost: a delta is better evidence of
    /// what the block is than the absence of a frame is. The placeholder's kind comes
    /// from the delta, which is why the caller says whether this is reasoning.
    fn append_delta(&mut self, index: usize, reasoning: bool, text: &str) {
        let placeholder = if reasoning { Block::Reasoning(String::new()) } else { Block::Text(String::new()) };
        while self.blocks.len() <= index {
            self.blocks.push(placeholder.clone());
        }
        match &mut self.blocks[index] {
            Block::Text(existing) | Block::Reasoning(existing) => existing.push_str(text),
            // A tool call's block holds arguments, not prose.
            Block::Call { .. } => {}
        }
    }

}

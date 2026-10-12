//! NDJSON framing over a byte stream.
//!
//! The host's decoder (`src/protocol.ts`) is the reference: a frame is one JSON
//! text plus a literal `0x0A`, frames may arrive split across reads or several at
//! once, and **a bad frame is never fatal** — it is dropped and counted so one
//! side's bug cannot cost the connection.
//!
//! The host's rule for oversized frames is deliberately mirrored here: bytes are
//! discarded until the next newline, so the stream resynchronises instead of the
//! reader accumulating an unbounded buffer while it waits for a terminator that
//! may never come.
//!
//! The single deviation is what counts as a frame *boundary*. The host splits on
//! `0x0A` alone; this side also treats `0x0D 0x0A` as a terminator and strips the
//! `0x0D`. Both sides emit bare `0x0A`, so the wire is identical — the tolerance
//! exists only so a frame written by a Windows tool (or a human debugging with
//! `printf`) is not silently glued to the next one.

use crate::ipc::protocol::MAX_FRAME_BYTES;

/// What one decoded line turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decoded {
    /// A line that parsed as JSON. The host validates the message shape; this
    /// layer only answers "is it JSON".
    Value(serde_json::Value),
    /// The line was not legal JSON, or was empty.
    Garbage,
    /// The line exceeded [`MAX_FRAME_BYTES`] and was dropped.
    ///
    /// The measured length travels with it because the refusal has to be explicable:
    /// "the panel received nothing" and "the panel received a 1.3 MB frame it could
    /// not take" look identical on screen, and only the second one tells the reader
    /// which side has to change. Kept as `bytes` rather than a bool so the log line
    /// can name the number the reader actually saw.
    Oversized {
        /// Bytes in the refused line, terminator excluded.
        bytes: usize,
    },
}

/// Counters a caller can log or report, matching the host's decoder counters.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FrameStats {
    /// Frames that parsed as JSON.
    pub decoded: u64,
    /// Lines rejected as non-JSON or empty.
    pub garbage: u64,
    /// Lines rejected for exceeding the size limit.
    pub oversized: u64,
}

/// Incremental NDJSON decoder.
///
/// Feed it arbitrary byte chunks; it yields whatever complete lines each chunk
/// finished. Holding partial state is the whole point — a frame split across two
/// reads must not be treated as two broken frames.
#[derive(Debug)]
pub struct FrameReader {
    pending: Vec<u8>,
    /// Set while discarding the tail of an oversized line.
    discarding: bool,
    /// Bytes of the line being read, including any already discarded.
    ///
    /// Kept beside `pending` rather than derived from it because `pending` is
    /// deliberately emptied the moment a line crosses the limit: the length of an
    /// oversized line is exactly the fact the refusal has to report, and it cannot
    /// be recovered after the buffer it was measured from is gone.
    line_bytes: usize,
    stats: FrameStats,
}

impl Default for FrameReader {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameReader {
    /// Create an empty reader.
    #[must_use]
    pub fn new() -> Self {
        Self { pending: Vec::new(), discarding: false, line_bytes: 0, stats: FrameStats::default() }
    }

    /// Counters accumulated so far.
    #[must_use]
    pub fn stats(&self) -> FrameStats {
        self.stats
    }

    /// Feed one chunk and take every line it completed.
    ///
    /// @param chunk - bytes as read; a chunk may end mid-line.
    /// @returns one entry per complete line, in order.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<Decoded> {
        let mut out = Vec::new();
        for &byte in chunk {
            if byte != b'\n' {
                // Counted before the discard check, because a line that never ends
                // still has a length and that length is what the report needs.
                self.line_bytes += 1;
                // Already discarding an oversized line: hold nothing at all until
                // the terminator arrives. Checking this first is what keeps the
                // buffer bounded — accumulating and clearing once would still let
                // a single chunk hold up to the limit before it was dropped.
                if self.discarding {
                    continue;
                }
                // A `\r` immediately before the newline is dropped when the line
                // is finished; a `\r` anywhere else is ordinary data.
                self.pending.push(byte);
                if self.pending.len() > MAX_FRAME_BYTES {
                    // Stop growing. Everything up to the terminator is junk, but
                    // the *next* line must still be readable, so the reader stays
                    // in this state until one arrives.
                    self.discarding = true;
                    self.pending.clear();
                }
                continue;
            }
            let bytes = std::mem::take(&mut self.line_bytes);
            if self.discarding {
                self.discarding = false;
                self.pending.clear();
                self.stats.oversized += 1;
                out.push(Decoded::Oversized { bytes });
                continue;
            }
            if self.pending.last() == Some(&b'\r') {
                self.pending.pop();
            }
            let line = std::mem::take(&mut self.pending);
            out.push(self.decode(&line));
        }
        out
    }

    /// Feed the rest of a stream and take any final unterminated line.
    ///
    /// @param chunk - bytes after which no more will arrive.
    /// @returns one entry per complete line, plus the trailing one if present.
    pub fn finish(&mut self, chunk: &[u8]) -> Vec<Decoded> {
        let mut out = self.push(chunk);
        if self.discarding {
            // The oversized line never ended; the whole tail is junk, and its
            // measured length is all the reader ever learns about it.
            self.discarding = false;
            self.pending.clear();
            self.stats.oversized += 1;
            out.push(Decoded::Oversized { bytes: self.line_bytes });
        } else if !self.pending.is_empty() {
            let line = std::mem::take(&mut self.pending);
            out.push(self.decode(&line));
        }
        self.line_bytes = 0;
        out
    }

    /// Parse one complete line and update the counters.
    fn decode(&mut self, line: &[u8]) -> Decoded {
        if line.is_empty() {
            // A blank line costs a counter but changes nothing: tolerating it
            // keeps a stray newline from looking like a protocol violation.
            self.stats.garbage += 1;
            return Decoded::Garbage;
        }
        match serde_json::from_slice::<serde_json::Value>(line) {
            Ok(value) => {
                self.stats.decoded += 1;
                Decoded::Value(value)
            }
            Err(_) => {
                self.stats.garbage += 1;
                Decoded::Garbage
            }
        }
    }
}

// Protocol contract: NDJSON framing, split reads, bounded buffers, junk frames.
// Category 1 on the wire, with category 3 robustness against anything malformed.
#[cfg(test)]
mod tests {
    use super::*;

    fn values(out: Vec<Decoded>) -> Vec<serde_json::Value> {
        out.into_iter()
            .filter_map(|entry| match entry {
                Decoded::Value(value) => Some(value),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_frame_split_across_two_reads_is_one_frame() {
        // Half a frame must never be judged: this is the failure that makes a
        // peer look like it is speaking garbage under load.
        let mut reader = FrameReader::new();
        assert!(reader.push(b"{\"jsonrpc\":\"2.0\",").is_empty());
        let out = reader.push(b"\"id\":1}\n");
        let values = values(out);
        assert_eq!(values.len(), 1);
        assert_eq!(values[0]["id"], 1);
        assert_eq!(reader.stats().garbage, 0, "a split frame is not garbage");
    }

    #[test]
    fn several_frames_in_one_read_all_arrive() {
        let mut reader = FrameReader::new();
        let values = values(reader.push(b"{\"id\":1}\n{\"id\":2}\n{\"id\":3}\n"));
        assert_eq!(values.len(), 3);
        assert_eq!(values[2]["id"], 3);
    }

    #[test]
    fn a_malformed_frame_does_not_stop_the_next_one() {
        // The host's rule, and ours: a bad frame is dropped and counted.
        let mut reader = FrameReader::new();
        let out = reader.push(b"not json\n{\"id\":7}\n");
        assert!(matches!(out[0], Decoded::Garbage));
        let values = values(out);
        assert_eq!(values, vec![serde_json::json!({"id": 7})]);
        assert_eq!(reader.stats().garbage, 1);
        assert_eq!(reader.stats().decoded, 1);
    }

    #[test]
    fn an_oversized_frame_is_dropped_and_the_stream_resynchronises() {
        // The point of the limit is to refuse the work, not to lose the channel.
        let mut reader = FrameReader::new();
        let mut chunk = vec![b'x'; MAX_FRAME_BYTES + 64];
        chunk.extend_from_slice(b"\n{\"id\":9}\n");
        let out = reader.push(&chunk);
        assert!(out.iter().any(|entry| matches!(entry, Decoded::Oversized { .. })));
        assert_eq!(values(out), vec![serde_json::json!({"id": 9})], "the frame after it still parses");
        assert_eq!(reader.stats().oversized, 1);
    }

    #[test]
    fn an_oversized_frame_reports_its_measured_length() {
        // The number is what separates "a frame was refused" from "a 1.3 MB frame
        // was refused": the reader empties its buffer the moment the limit is
        // crossed, so the length has to be counted as the bytes arrive.
        let mut reader = FrameReader::new();
        let out = reader.push(&vec![b'x'; MAX_FRAME_BYTES + 64]);
        assert!(out.is_empty(), "the line has no terminator yet");
        let out = reader.push(b"\n");
        assert_eq!(out, vec![Decoded::Oversized { bytes: MAX_FRAME_BYTES + 64 }]);
    }

    #[test]
    fn an_oversized_tail_is_measured_at_end_of_stream() {
        // The same fact for a line the peer never terminated: the read loop's
        // `finish` is the only place this length can be reported from.
        let mut reader = FrameReader::new();
        assert!(reader.push(&vec![b'x'; MAX_FRAME_BYTES + 3]).is_empty());
        assert_eq!(reader.finish(b""), vec![Decoded::Oversized { bytes: MAX_FRAME_BYTES + 3 }]);
        assert_eq!(reader.stats().oversized, 1);
    }

    #[test]
    fn an_oversized_frame_does_not_grow_the_buffer_without_bound() {
        // Feed far more than the limit with no terminator: the reader must have
        // discarded it rather than hold it.
        let mut reader = FrameReader::new();
        let chunk = vec![b'x'; MAX_FRAME_BYTES * 3];
        assert!(reader.push(&chunk).is_empty());
        assert_eq!(reader.pending.len(), 0, "nothing is retained while discarding");
        assert!(reader.discarding);
    }

    #[test]
    fn a_blank_line_counts_as_garbage_but_changes_nothing() {
        let mut reader = FrameReader::new();
        let values = values(reader.push(b"\n{\"id\":1}\n\n"));
        assert_eq!(values.len(), 1);
        assert_eq!(reader.stats().garbage, 2);
    }

    #[test]
    fn an_unterminated_tail_is_only_read_at_end_of_stream() {
        let mut reader = FrameReader::new();
        assert!(reader.push(b"{\"id\":1}").is_empty(), "a partial frame waits for an answer");
        let values = values(reader.finish(b""));
        assert_eq!(values, vec![serde_json::json!({"id": 1})]);
    }

    #[test]
    fn a_crlf_terminator_is_tolerated_and_stripped() {
        // The host emits bare `\n`; accepting `\r\n` keeps a hand-written frame
        // from being glued to the next one.
        let mut reader = FrameReader::new();
        let values = values(reader.push(b"{\"id\":1}\r\n"));
        assert_eq!(values, vec![serde_json::json!({"id": 1})]);
    }

    #[test]
    fn a_bare_carriage_return_inside_a_line_is_data() {
        let mut reader = FrameReader::new();
        let values = values(reader.push("{\"s\":\"a\\rb\"}\n".as_bytes()));
        assert_eq!(values[0]["s"], "a\rb");
    }

    #[test]
    fn multibyte_utf8_split_across_reads_survives() {
        // Byte-oriented reading must not corrupt a character split across reads.
        let text = "{\"s\":\"中文\"}\n".as_bytes().to_vec();
        let (head, tail) = text.split_at(8);
        let mut reader = FrameReader::new();
        assert!(reader.push(head).is_empty());
        let values = values(reader.push(tail));
        assert_eq!(values[0]["s"], "中文");
    }
}

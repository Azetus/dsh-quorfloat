//! The real stdin/stdout transport.
//!
//! Two invariants, both enforced by construction rather than by discipline:
//!
//! - **stdout carries protocol frames only.** [`StdioSink`] is the only thing in
//!   this crate that writes to stdout, and it writes nothing but encoded frames.
//!   Logs go to stderr, which is also where the host reads them from.
//! - **A dead peer is not an error.** When the host closes stdin, or a write
//!   fails because the pipe is gone, that is the normal end of a session. Trying
//!   to report it through the same broken channel is how a peer ends up spinning.
//!
//! Reads are line-oriented over a byte stream: [`FrameReader`] keeps the partial
//! tail, so a frame split across two reads is one frame, and a line that is not
//! JSON costs a counter instead of the connection.

use std::io::{BufReader, Read, Write};

use crate::frame::{Decoded, FrameReader};
use crate::marker::Marker;
use crate::rpc::{self, Inbound};
use crate::session::{FrameSink, FrameSource};

/// Writes frames to stdout, log lines to stderr, breadcrumbs to the marker file.
pub struct StdioSink {
    out: std::io::Stdout,
    marker: Marker,
}

impl Default for StdioSink {
    fn default() -> Self {
        Self::new(Marker::disabled())
    }
}

impl StdioSink {
    /// Create a sink over the process's stdout and stderr.
    ///
    /// @param marker - where durable breadcrumbs go; a disabled marker writes none.
    #[must_use]
    pub fn new(marker: Marker) -> Self {
        Self { out: std::io::stdout(), marker }
    }
}

impl FrameSink for StdioSink {
    fn send(&mut self, frame: &serde_json::Value) -> std::io::Result<()> {
        let bytes = rpc::encode(frame);
        let mut lock = self.out.lock();
        lock.write_all(&bytes)?;
        // Flush per frame. The host is waiting on this write to decide whether the
        // process started, so deferring it to buffer pressure would turn a healthy
        // peer into a missed handshake.
        lock.flush()
    }

    fn log(&mut self, line: &str) {
        // Written straight to stderr, never through the frame path. A failure here
        // is not actionable — stderr is allowed to be closed — so it is dropped
        // rather than escalated into the session's control flow.
        let _ = writeln!(std::io::stderr(), "{line}");
    }

    fn mark(&mut self, line: &str) {
        self.marker.write(line);
    }
}

/// Reads frames from stdin.
pub struct StdinSource {
    reader: BufReader<std::io::Stdin>,
    frames: FrameReader,
    /// Complete lines produced by the last read that have not been handed out.
    ready: std::collections::VecDeque<Inbound>,
    eof: bool,
}

impl Default for StdinSource {
    fn default() -> Self {
        Self::new()
    }
}

impl StdinSource {
    /// Create a source over the process's stdin.
    #[must_use]
    pub fn new() -> Self {
        Self {
            reader: BufReader::new(std::io::stdin()),
            frames: FrameReader::new(),
            ready: std::collections::VecDeque::new(),
            eof: false,
        }
    }

    /// Counters of frames seen so far, for the exit summary.
    #[must_use]
    pub fn stats(&self) -> crate::frame::FrameStats {
        self.frames.stats()
    }

    /// Fill `ready` from the next chunk of input.
    ///
    /// @returns false at end of stream.
    fn fill(&mut self) -> bool {
        if self.eof {
            return false;
        }
        let mut buffer = [0_u8; 8192];
        match self.reader.get_mut().read(&mut buffer) {
            Ok(0) => {
                self.eof = true;
                for entry in self.frames.finish(&[]) {
                    self.accept(entry);
                }
                false
            }
            Ok(count) => {
                for entry in self.frames.push(&buffer[..count]) {
                    self.accept(entry);
                }
                true
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => true,
            Err(_) => {
                // A read error means the channel is gone in a way we cannot
                // distinguish from a close, and there is nothing to retry.
                self.eof = true;
                false
            }
        }
    }

    /// Queue one decoded line, classifying it.
    fn accept(&mut self, entry: Decoded) {
        let inbound = match entry {
            Decoded::Value(value) => rpc::classify(value),
            Decoded::Garbage | Decoded::Oversized => Inbound::Malformed,
        };
        self.ready.push_back(inbound);
    }
}

impl StdinSource {
    /// Block until the next frame arrives.
    ///
    /// The inherent method shadows the trait's for direct callers in this crate,
    /// which keeps the reader thread from having to import the trait.
    ///
    /// @returns `None` when the host closed the stream.
    #[must_use]
    pub fn next_frame(&mut self) -> Option<Inbound> {
        <Self as FrameSource>::next_frame(self)
    }
}

impl FrameSource for StdinSource {
    fn next_frame(&mut self) -> Option<Inbound> {
        loop {
            if let Some(frame) = self.ready.pop_front() {
                return Some(frame);
            }
            if !self.fill() {
                return self.ready.pop_front();
            }
        }
    }
}

/// Verify the reader handles a buffered read prefix correctly.
///
/// Kept here rather than in `frame.rs` because it exercises the interaction of
/// `read` chunk boundaries with the decoder, which is where an off-by-one shows
/// up in practice.
#[cfg(test)]
mod tests {
    use crate::frame::Decoded;

    #[test]
    fn a_reader_that_returns_one_byte_at_a_time_still_frames_correctly() {
        let mut frames = crate::frame::FrameReader::new();
        let mut collected = Vec::new();
        for byte in b"{\"id\":1}\n{\"id\":2}\n" {
            for entry in frames.push(&[*byte]) {
                collected.push(entry);
            }
        }
        let values: Vec<_> = collected
            .into_iter()
            .filter_map(|entry| match entry {
                Decoded::Value(value) => Some(value),
                _ => None,
            })
            .collect();
        assert_eq!(values.len(), 2);
        assert_eq!(values[1]["id"], 2);
    }
}

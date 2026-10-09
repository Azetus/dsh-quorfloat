//! The bridge between the frontend and the thread that reads frames.
//!
//! Three things live here because they are one mechanism: the channel that wakes the
//! shell's dispatcher ([`Wake`]), the sink that lets several threads write to one stdout
//! ([`SharedSink`]), and the thread that turns stdin into both ([`Reader`]).
//!
//! The sink is shared rather than owned because the handshake is written by the main
//! thread before the window exists while every later frame is written by the reader
//! thread, and both must serialise onto the same stream. The channel is bounded and
//! `try_send`s, because a reader that blocks on a full queue stops reading stdin — and
//! a process that stops reading stdin is a process the host declares dead.

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use crate::app::session::{FrameSink, Session, SessionExit};
use crate::ipc::transport::StdinSource;

/// Something that needs the shell's attention.
#[derive(Debug, Clone, PartialEq)]
pub enum Wake {
    /// At least one frame arrived. The frames were already answered on the reader
    /// thread; this asks the shell to refresh with any new configuration.
    Frames,
    /// The hotkey was pressed.
    Hotkey,
    /// The frontend reported a new content height (logical px, panel only).
    Height(f32),
    /// The host asked this process to stop, or the channel ended.
    Exit(SessionExit),
}

/// A frame sink that can be written from several threads.
///
/// The handshake is written by the main thread before the window exists, and every
/// later frame is written by the reader thread. Both must serialise onto one
/// stdout, so the sink is shared rather than owned.
pub struct SharedSink {
    inner: Mutex<Box<dyn FrameSink + Send>>,
}

impl SharedSink {
    /// Wrap a sink for cross-thread use.
    ///
    /// @param sink - the underlying sink, normally stdout.
    #[must_use]
    pub fn new(sink: Box<dyn FrameSink + Send>) -> Self {
        Self { inner: Mutex::new(sink) }
    }

    /// Run one operation with the sink locked.
    ///
    /// A poisoned mutex is recovered rather than propagated: a panic in one writer
    /// must not turn the protocol channel into a permanent failure, and the mutex
    /// guards only a file handle, which no panic can leave half-updated.
    ///
    /// @param operation - what to do with the sink.
    /// @returns whatever the operation returned.
    pub fn with<T>(&self, operation: impl FnOnce(&mut dyn FrameSink) -> T) -> T {
        let mut guard = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        operation(guard.as_mut())
    }

    /// Write one frame.
    ///
    /// Inherent rather than only a trait method so it can take `&self`: a shared
    /// sink lives behind an `Arc`, so callers never have `&mut self` to give the
    /// trait's signature. The mutex provides the interior mutability.
    ///
    /// @param frame - the message to encode and send.
    /// @returns an error when the peer is gone.
    pub fn send(&self, frame: &serde_json::Value) -> std::io::Result<()> {
        self.with(|sink| sink.send(frame))
    }

    /// Record one log line, on stderr.
    ///
    /// @param line - text to record.
    pub fn log(&self, line: &str) {
        self.with(|sink| sink.log(line));
    }

    /// Record one durable breadcrumb.
    ///
    /// Forwarded explicitly rather than left to the trait's default: the default is
    /// a no-op, so forgetting this hop makes the marker file exist and stay empty —
    /// which reads as "nothing happened" rather than as "the plumbing is missing".
    ///
    /// @param line - text to record.
    pub fn mark(&self, line: &str) {
        self.with(|sink| sink.mark(line));
    }
}

impl FrameSink for SharedSink {
    fn send(&mut self, frame: &serde_json::Value) -> std::io::Result<()> {
        self.with(|sink| sink.send(frame))
    }

    fn log(&mut self, line: &str) {
        self.with(|sink| sink.log(line));
    }

    fn mark(&mut self, line: &str) {
        self.with(|sink| sink.mark(line));
    }
}

/// The stdin pump, running on its own thread.
pub struct Reader {
    session: Arc<Mutex<Session>>,
    sink: Arc<SharedSink>,
    wake: Sender<Wake>,
    /// Records why the session ended, so the window closing and the host closing
    /// the channel do not have to be told apart afterwards.
    outcome: Arc<Mutex<Option<SessionExit>>>,
    /// Tells the frontend that the state changed. Provided by the shell: the reader
    /// cannot reach the webview, so it emits the shell's event, which the frontend
    /// listens to and re-renders from.
    nudge: Arc<dyn Fn() + Send + Sync>,
}

impl Reader {
    /// Assemble a reader.
    ///
    /// @param session - the session state, shared with the shell.
    /// @param sink - where answers go.
    /// @param wake - how to notify the dispatcher.
    /// @param outcome - where to record why the session ended.
    /// @param nudge - how to tell the frontend the state changed.
    #[must_use]
    pub fn new(
        session: Arc<Mutex<Session>>,
        sink: Arc<SharedSink>,
        wake: Sender<Wake>,
        outcome: Arc<Mutex<Option<SessionExit>>>,
        nudge: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        Self { session, sink, wake, outcome, nudge }
    }

    /// Read frames until the stream ends, answering each one.
    ///
    /// Blocking reads are exactly what is wanted here: this thread owns stdin, and
    /// the thread ends when the host closes it. Nothing about it needs the window
    /// to be visible, which is why the host keeps receiving answers while the panel
    /// is hidden.
    pub fn run(self) {
        let mut source = StdinSource::new();
        loop {
            let Some(frame) = source.next_frame() else {
                self.finish(SessionExit::PeerClosed);
                return;
            };
            let exit = {
                let mut session = match self.session.lock() {
                    Ok(session) => session,
                    Err(poisoned) => poisoned.into_inner(),
                };
                session.on_frame(frame, &mut BorrowedSink(&self.sink))
            };
            self.notify(Wake::Frames);
            if let Some(exit) = exit {
                self.finish(exit);
                return;
            }
        }
    }

    /// Record the ending and tell the main thread to stop.
    fn finish(&self, exit: SessionExit) {
        {
            let mut outcome = match self.outcome.lock() {
                Ok(outcome) => outcome,
                Err(poisoned) => poisoned.into_inner(),
            };
            *outcome = Some(exit.clone());
        }
        self.notify(Wake::Exit(exit));
    }

    /// Tell the dispatcher something happened, and tell the frontend to refresh.
    ///
    /// A send failure means the shell is gone, which ends the process anyway;
    /// there is nothing to recover.
    fn notify(&self, wake: Wake) {
        let _ = self.wake.send(wake);
        // Without this the frontend would not refresh while the window is hidden —
        // and hidden is its normal state, so the configuration the host sends would
        // never be reflected in what the user eventually sees.
        (self.nudge)();
    }
}

/// Hands the shared sink to a `&mut dyn FrameSink` parameter.
///
/// The session takes a trait object, while the shared sink is only reachable
/// behind an `Arc`; this adapter is the smallest way to bridge the two without
/// adding a second locking scheme.
/// A sink that borrows the shared one for the duration of one call.
///
/// The session needs a sink to write through; handing it the shared sink's lock for a
/// whole frame would be handing it the render loop. A borrow keeps the lock to the call
/// that needs it.
pub(super) struct BorrowedSink<'a>(pub(super) &'a SharedSink);

impl FrameSink for BorrowedSink<'_> {
    fn send(&mut self, frame: &serde_json::Value) -> std::io::Result<()> {
        self.0.send(frame)
    }

    fn log(&mut self, line: &str) {
        self.0.log(line);
    }

    fn mark(&mut self, line: &str) {
        self.0.mark(line);
    }
}

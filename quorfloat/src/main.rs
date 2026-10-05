//! Entry point: complete the handshake, then serve the host until it lets go.
//!
//! Nothing here may write to stdout. The host parses stdout as protocol frames,
//! so diagnostics go through [`dsh_quorfloat::session::FrameSink::log`], which
//! writes to stderr — including the panic hook installed below, because a panic
//! message on stdout would corrupt the stream on the way out.

use std::process::ExitCode;

use dsh_quorfloat::session::{FrameSink, HotkeyReport, Identity, Session, SessionExit};
use dsh_quorfloat::transport::{StdinSource, StdioSink};
use dsh_quorfloat::VERSION;

fn main() -> ExitCode {
    install_panic_hook();
    match run() {
        Ok(exit) => match exit {
            SessionExit::ShutdownRequested | SessionExit::PeerClosed => ExitCode::SUCCESS,
        },
        Err(message) => {
            eprintln!("dsh-quorfloat: {message}");
            ExitCode::FAILURE
        }
    }
}

/// The optional lifecycle marker file named by `DSH_QUORFLOAT_RUST_MARKER`.
///
/// Absent unless asked for: a shipped build writes nothing to disk. It exists
/// because the host does not surface its logs, which leaves "did the binary even
/// start" unanswerable from outside when the desktop app owns the process.
struct Marker {
    path: Option<std::path::PathBuf>,
}

impl Marker {
    /// Read the marker path from the environment.
    ///
    /// @returns a marker that writes nothing when the variable is unset.
    fn from_env() -> Self {
        Self {
            path: std::env::var("DSH_QUORFLOAT_RUST_MARKER")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .map(std::path::PathBuf::from),
        }
    }

    /// Append one line, if a marker was requested. Failures are ignored: this is
    /// a diagnostic aid and must never be the reason a session fails.
    ///
    /// @param line - text to append, without a trailing newline.
    fn write(&self, line: &str) {
        let Some(path) = &self.path else { return };
        use std::io::Write as _;
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(file, "{line}");
        }
    }
}

/// Send panic output to stderr so it can never reach the protocol stream.
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("dsh-quorfloat panicked: {info}");
    }));
}

/// Run one session to completion.
///
/// @returns why the session ended, or a message describing a startup failure.
fn run() -> Result<SessionExit, String> {
    let identity = Identity {
        quorfloat_version: VERSION.to_owned(),
        // The host's spelling, not Rust's: it compares these against its own
        // `process.platform` / `process.arch` (see `platform.rs`).
        platform: dsh_quorfloat::platform::host_platform().to_owned(),
        arch: dsh_quorfloat::platform::host_arch().to_owned(),
        // Registering a hotkey needs a window system, which arrives with P1. Until
        // then the honest report is `registered: false`: the host shows that in
        // its settings surface, where a user can see the key is not active, rather
        // than the key silently doing nothing.
        hotkey: HotkeyReport::requested_from_env(),
    };

    let mut session = Session::new(identity);
    let mut source = StdinSource::new();
    let mut sink = StdioSink::new();

    // A lifecycle marker, written only when asked for. The host does not export
    // its own logs, so when this process is started by the desktop app there is
    // otherwise no way to tell "the binary ran and handshook" from "the binary was
    // never spawned" — a distinction that costs an hour every time it comes up.
    // Each line is a fact about this process, appended in order and flushed
    // immediately, so a marker that stops after `start` means the handshake never
    // came back.
    let marker = Marker::from_env();
    marker.write("start");

    let exit = session.run(&mut source, &mut sink);
    if session.is_ready() {
        let host = session.host_config();
        marker.write(&format!(
            "ready host={} session={}",
            host.host_version.as_deref().unwrap_or("unknown"),
            host.session_id.as_deref().unwrap_or("unknown"),
        ));
    } else {
        marker.write("not-ready");
    }
    let stats = source.stats();
    sink.log(&format!(
        "session ended ({exit:?}): frames decoded={} garbage={} oversized={}",
        stats.decoded, stats.garbage, stats.oversized,
    ));
    marker.write(&format!(
        "end {exit:?} decoded={} garbage={} oversized={}",
        stats.decoded, stats.garbage, stats.oversized,
    ));
    Ok(exit)
}

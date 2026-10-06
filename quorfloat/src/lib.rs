//! The Rust half of dsh-quorfloat: the floating conversation window.
//!
//! Layering, from the host inward:
//!
//! - [`ipc`] — the wire. [`ipc::protocol`] holds the constants both sides agree on,
//!   [`ipc::frame`] the framing, [`ipc::rpc`] the message shapes, [`ipc::transport`]
//!   the byte stream, [`ipc::platform`] how this process names itself. This is the one
//!   layer whose changes can break the other half of the system.
//! - [`app`] — the state, and the event loop that drives it. [`app::session`] is the
//!   handshake, the notifications, the interactions and shutdown over an abstract I/O;
//!   [`app::session::follow`] decides which conversation this panel follows (and why
//!   that decides ownership); [`app::session::transcript`] folds records and stream
//!   frames into lines a panel can draw; [`app::sink`] is the bridge to the thread that
//!   reads frames.
//! - [`ui`] — what the panel draws, and the window it draws into:
//!   `ui::conversation` the lines, `ui::cards` the requests waiting for an answer,
//!   [`ui::window`] the viewport, [`ui::fonts`] the bundled font without which Chinese
//!   is a row of boxes. It reads a state snapshot and reports clicks; it cannot reach
//!   the session. Its one entry point is `ui::draw`.
//!
//! Cross-cutting: [`runtime`] holds what belongs to neither the wire nor the panel —
//! [`runtime::hotkey`], the one input the operating system owns, and [`runtime::diag`],
//! the optional marker and raw-frame dump, off unless the environment asks for them
//! (`docs/prototype.md` §21.3, §23.2).
//!
//! The window layer sits *above* the handshake: no viewport is created until the host
//! has answered, because the host's startup budget starts when this process does.

pub mod app;
pub mod ipc;
pub mod runtime;
pub mod ui;

/// Version of this subproject, taken from the crate manifest at build time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

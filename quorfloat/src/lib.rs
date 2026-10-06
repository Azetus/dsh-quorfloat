//! The Rust half of dsh-quorfloat: the floating conversation window.
//!
//! Layering, from the host inward:
//!
//! - [`ipc`] — the wire. [`ipc::protocol`] holds the constants both sides agree on,
//!   [`ipc::frame`] the framing, [`ipc::rpc`] the message shapes, [`ipc::transport`]
//!   the byte stream, [`ipc::platform`] how this process names itself. This is the one
//!   layer whose changes can break the other half of the system.
//! - [`session`] — the handshake, the notifications, the interactions, and shutdown,
//!   over an abstract I/O so all of it can be tested without a host.
//! - [`transcript`] — the conversation, folded from records and stream frames into
//!   lines a panel can draw.
//! - [`follow`] — which conversation this panel follows, and why that decides ownership.
//! - [`fonts`] — the bundled fallback font, so the panel can draw Chinese at all.
//! - [`window`] — the native window, its always-on-top behaviour, and the global hotkey.
//! - [`app`] — the event loop that drives the two of them, including while hidden.
//!
//! Cross-cutting: [`dump`] and [`marker`] are optional diagnostics, off unless the
//! environment asks for them (`docs/prototype.md` §21.3, §23.2).
//!
//! The window layer sits *above* the handshake: no viewport is created until the host
//! has answered, because the host's startup budget starts when this process does.

pub mod app;
pub mod dump;
pub mod follow;
pub mod fonts;
pub mod ipc;
pub mod marker;
pub mod session;
pub mod transcript;
pub mod window;

/// Version of this subproject, taken from the crate manifest at build time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

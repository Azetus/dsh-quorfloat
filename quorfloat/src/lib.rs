//! The Rust half of dsh-quorfloat: the floating conversation window.
//!
//! Layering, from the host inward:
//!
//! - [`protocol`] — constants that are contracts with the TypeScript host plugin.
//! - [`platform`] — platform/arch names in the host's vocabulary.
//! - [`fonts`] — the bundled fallback font, so the panel can draw Chinese.
//! - [`frame`] — NDJSON framing over a byte stream.
//! - [`rpc`] — JSON-RPC message shapes and the inbound method surface.
//! - [`session`] — handshake, notifications, interactions, and shutdown, over
//!   abstract I/O.
//! - [`transport`] — the real stdin/stdout implementation of that I/O.
//! - [`marker`] — the optional breadcrumb file that makes this process observable.
//! - [`window`] — the native window, always-on-top behaviour, and the global hotkey.
//! - [`app`] — the event loop that drives both, including while the panel is hidden.
//!
//! The window, hotkey, and egui layers arrive with P1 and sit *above* all of
//! this: the handshake must complete before a window exists, because the host's
//! startup budget is running from the moment the process does.

pub mod app;
pub mod follow;
pub mod fonts;
pub mod frame;
pub mod marker;
pub mod platform;
pub mod protocol;
pub mod rpc;
pub mod session;
pub mod transport;
pub mod window;

/// Version of this subproject, taken from the crate manifest at build time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

//! The wire: everything that is a contract with the TypeScript host.
//!
//! This is the crate's outermost layer, and the only place where a change can break
//! the other half of the system without breaking this one. Keeping it in one directory
//! is what makes that boundary visible: a reader can see at a glance which code exists
//! to satisfy the host and which code exists to serve the panel.
//!
//! - [`protocol`] — the constants both sides agree on: version, capabilities, error codes.
//! - [`frame`] — NDJSON framing, including the oversized-frame recovery.
//! - [`rpc`] — JSON-RPC shapes and the router that classifies inbound frames.
//! - [`transport`] — the real stdin/stdout implementation of the byte stream.
//! - [`platform`] — how this process spells its platform and architecture to the host.
//!
//! Nothing here draws, owns state, or knows what a conversation is. The next layer in
//! is [`crate::app::session`], which is the first thing that has an opinion about meaning.

pub mod frame;
pub mod platform;
pub mod protocol;
pub mod rpc;
pub mod transport;

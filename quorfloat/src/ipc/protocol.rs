//! Protocol constants shared by both halves of this subproject.
//!
//! Every value here is a contract with the TypeScript host plugin, and each one
//! is asserted against the host's own validator in `tests/`. Nothing in this
//! module may be changed without changing `docs/dsh-quorfloat.md` first: a mismatch
//! shows up as a failed handshake, which is exactly the failure this layer
//! exists to make impossible.

/// The wire protocol version. Independent of Harness' own host protocol.
///
/// The host rejects any other value with `PROTOCOL_MISMATCH`, so this is the
/// first thing that must be right.
pub const PROTOCOL_VERSION: &str = "quorfloat/1";

/// Largest frame accepted, matching the host's `MAX_FRAME_BYTES`.
///
/// An oversized frame is **not** fatal: it is dropped and counted, because one
/// peer bug must not cost the floating window its connection.
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;

/// JSON-RPC error codes used by this protocol, matching the host's `ErrorCode`.
///
/// ```text
/// -32700 ParseError           -32002 RequestTimeout
/// -32600 InvalidRequest       -32003 ChannelClosed
/// -32601 MethodNotFound       -32004 Unavailable
/// -32602 InvalidParams        -32005 Stale
/// -32603 InternalError        -32001 ProtocolMismatch
/// -32006 ProtocolViolation
/// ```
pub mod error_code {
    /// The payload was not legal JSON.
    pub const PARSE_ERROR: i64 = -32700;
    /// Legal JSON, but not a JSON-RPC message.
    pub const INVALID_REQUEST: i64 = -32600;
    /// The method does not exist.
    pub const METHOD_NOT_FOUND: i64 = -32601;
    /// Parameters were rejected by the method.
    pub const INVALID_PARAMS: i64 = -32602;
    /// The handler failed.
    pub const INTERNAL_ERROR: i64 = -32603;
    /// The peer does not accept this protocol version.
    pub const PROTOCOL_MISMATCH: i64 = -32001;
    /// No answer arrived inside the budget. **Not** proof the peer did not act.
    pub const REQUEST_TIMEOUT: i64 = -32002;
    /// The channel is closed or the peer exited.
    pub const CHANNEL_CLOSED: i64 = -32003;
    /// The capability is unavailable, not broken.
    pub const UNAVAILABLE: i64 = -32004;
    /// The caller's view is stale.
    pub const STALE: i64 = -32005;
    /// The peer claimed success but later state contradicts it.
    pub const PROTOCOL_VIOLATION: i64 = -32006;
}

/// Capabilities this **build** implements, reported once in `hello`.
///
/// Read this as a build inventory, not as a promise about this run. `hello` is
/// written before any window exists, so nothing here can be a measurement: on a
/// machine with no display server, `window` is listed and no window will ever
/// appear.
///
/// The measured answer travels separately, in the `window/visibility` report
/// (`capabilities`), which the shell only sends once Tauri has actually created
/// a window. Hosts that need to decide whether a panel can appear must
/// read that one; this list only says which binary is running, which is what a
/// human reads in diagnostics to confirm the version in front of them.
///
/// `approval` is here rather than in the measured list because it has no runtime
/// failure mode to measure: this build either renders approval cards and answers
/// them or it does not, and that is a property of the binary. It is a *separate*
/// claim from `window` on purpose — a build with no display server still speaks the
/// protocol, and naming the two things apart keeps a diagnostic reader from
/// concluding that a missing window means a missing answerer.
pub const CAPABILITIES: &[&str] = &["window", "hotkey", "tauri", "approval"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_version_is_the_one_the_host_accepts() {
        // Pinned literally: the host compares against its own constant, so this
        // test failing means the two halves disagree about the wire contract.
        assert_eq!(PROTOCOL_VERSION, "quorfloat/1");
    }

    #[test]
    fn frame_limit_matches_the_host_budget() {
        assert_eq!(MAX_FRAME_BYTES, 1024 * 1024);
    }

    #[test]
    fn the_build_inventory_names_every_implemented_area() {
        // Read by a human in diagnostics and asserted here so a capability cannot be
        // dropped by accident: a host that trusted `approval` and then saw it vanish
        // would have to guess whether the panel can still answer.
        for expected in ["window", "hotkey", "tauri", "approval"] {
            assert!(CAPABILITIES.contains(&expected), "{expected} is missing from {CAPABILITIES:?}");
        }
    }

    #[test]
    fn error_codes_match_the_documented_table() {
        assert_eq!(error_code::PARSE_ERROR, -32700);
        assert_eq!(error_code::INVALID_REQUEST, -32600);
        assert_eq!(error_code::METHOD_NOT_FOUND, -32601);
        assert_eq!(error_code::INVALID_PARAMS, -32602);
        assert_eq!(error_code::INTERNAL_ERROR, -32603);
        assert_eq!(error_code::PROTOCOL_MISMATCH, -32001);
        assert_eq!(error_code::REQUEST_TIMEOUT, -32002);
        assert_eq!(error_code::CHANNEL_CLOSED, -32003);
        assert_eq!(error_code::UNAVAILABLE, -32004);
        assert_eq!(error_code::STALE, -32005);
        assert_eq!(error_code::PROTOCOL_VIOLATION, -32006);
    }
}

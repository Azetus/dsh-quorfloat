//! JSON-RPC 2.0 message shapes and the router.
//!
//! Standard JSON-RPC, with the project's own narrowing: ids are integers only,
//! and the method set is fixed by `docs/dsh-quorfloat.md`. The host's router is the
//! reference implementation — a method it does not know is answered
//! `METHOD_NOT_FOUND`, and so is one this side does not know, because both ends
//! are expected to tolerate a peer newer than themselves.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::ipc::protocol::{CAPABILITIES, PROTOCOL_VERSION, error_code};

/// One inbound message, classified the way JSON-RPC classifies it.
#[derive(Debug, Clone, PartialEq)]
pub enum Inbound {
    /// Carries both `id` and `method`: the peer expects an answer.
    Request { id: Value, method: String, params: Option<Value> },
    /// Carries only `method`: no answer is expected, ever.
    Notification { method: String, params: Option<Value> },
    /// Carries only `id`: an answer to something this side sent.
    Response { id: Value, outcome: Result<Value, RpcError> },
    /// Legal JSON that is not a JSON-RPC message at all.
    Malformed,
}

/// A JSON-RPC error object.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RpcError {
    /// Numeric code from the protocol's table.
    pub code: i64,
    /// Human-readable reason; the host writes these to be read by a human.
    pub message: String,
    /// Optional structured detail, e.g. `supported` versions on a mismatch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// What a method call produced.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Answer the request with this result.
    Ok(Value),
    /// Answer the request with this error.
    Err(RpcError),
    /// This side handled it and the caller needs to do something extra.
    Accepted,
}

impl Outcome {
    /// Build a successful outcome.
    #[must_use]
    pub fn ok(value: Value) -> Self {
        Self::Ok(value)
    }

    /// Build a `METHOD_NOT_FOUND` error, the answer to any unknown method.
    #[must_use]
    pub fn unknown_method(method: &str) -> Self {
        Self::Err(RpcError {
            code: error_code::METHOD_NOT_FOUND,
            message: format!("unsupported method: {method}"),
            data: None,
        })
    }

    /// Build an `INVALID_PARAMS` error.
    #[must_use]
    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::Err(RpcError { code: error_code::INVALID_PARAMS, message: message.into(), data: None })
    }
}

/// Split one decoded JSON value into a request, notification, response, or junk.
///
/// @param value - a value that already parsed as JSON.
/// @returns the classified message.
#[must_use]
pub fn classify(value: Value) -> Inbound {
    let Some(object) = value.as_object() else {
        return Inbound::Malformed;
    };
    let id = object.get("id").cloned();
    let method = object.get("method").and_then(Value::as_str).map(str::to_owned);
    match (id, method) {
        (Some(id), Some(method)) => Inbound::Request {
            id,
            method,
            params: object.get("params").cloned(),
        },
        (None, Some(method)) => Inbound::Notification {
            method,
            params: object.get("params").cloned(),
        },
        (Some(id), None) => {
            if let Some(error) = object.get("error") {
                match serde_json::from_value::<RpcError>(error.clone()) {
                    Ok(error) => Inbound::Response { id, outcome: Err(error) },
                    // An error object this side cannot read is still an answer:
                    // treating it as garbage would leave the request pending.
                    Err(_) => Inbound::Response {
                        id,
                        outcome: Err(RpcError {
                            code: error_code::INTERNAL_ERROR,
                            message: "peer sent an unreadable error object".to_owned(),
                            data: Some(error.clone()),
                        }),
                    },
                }
            } else {
                Inbound::Response {
                    id,
                    outcome: Ok(object.get("result").cloned().unwrap_or(Value::Null)),
                }
            }
        }
        (None, None) => Inbound::Malformed,
    }
}

/// Encode one frame: the message plus the terminator the host splits on.
///
/// @param message - any JSON value.
/// @returns bytes ready for stdout.
///
/// # Panics
/// Never in practice: `serde_json` only fails here on a map with non-string keys,
/// which this module does not construct.
#[must_use]
pub fn encode(message: &Value) -> Vec<u8> {
    let mut out = serde_json::to_vec(message).expect("protocol messages are always encodable");
    out.push(b'\n');
    out
}

/// Build a success response.
#[must_use]
pub fn success(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// Build an error response.
#[must_use]
pub fn failure(id: &Value, error: &RpcError) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": {
        "code": error.code,
        "message": error.message,
        "data": error.data,
    } })
}

/// Build a notification (no `id`, no answer expected).
#[must_use]
pub fn notification(method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "method": method, "params": params })
}

/// Build a request.
#[must_use]
pub fn request(id: i64, method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

/// The inbound method surface this subproject implements.
///
/// Only two entries for now, and that is the point of the first milestone: the
/// session and interaction traffic arrives with the layers that can act on it, so
/// an unknown method here is a real answer (`METHOD_NOT_FOUND`) rather than a
/// silent success that would make a half-built peer look complete.
#[derive(Debug, Default)]
pub struct Router;

impl Router {
    /// Answer one inbound request.
    ///
    /// @param method - the JSON-RPC method name.
    /// @param params - its parameters, if any.
    /// @returns what to send back.
    #[must_use]
    pub fn handle(&self, method: &str, _params: Option<&Value>) -> Outcome {
        match method {
            // Liveness probe. The host sends it after every `host/heartbeat` and
            // treats a missing answer as a missed beat, so the result body is
            // irrelevant — answering at all is the whole contract.
            "ping" => Outcome::ok(json!({ "t": now_millis() })),
            // An orderly stop. The host sends this while the channel is still
            // usable, waits `shutdownGraceMs` for the answer, and only then
            // escalates to signals — and it records whether it had to escalate.
            // Answering and exiting promptly is the evidence it looks for.
            "shutdown" => Outcome::Accepted,
            // The host answering *our* opening `hello`. Both sides implement this
            // method because either may ask; in practice the peer speaks first, so
            // this branch is the host's reply rather than an unsolicited request.
            "hello" => Outcome::ok(json!({
                "protocol": PROTOCOL_VERSION,
                "quorfloatVersion": crate::VERSION,
                "platform": crate::ipc::platform::host_platform(),
                "arch": crate::ipc::platform::host_arch(),
                "capabilities": CAPABILITIES,
            })),
            _ => Outcome::unknown_method(method),
        }
    }
}

/// Milliseconds since the Unix epoch, saturating instead of panicking on a clock
/// set before 1970.
#[must_use]
pub fn now_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(elapsed) => i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX),
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_message_with_id_and_method_is_a_request() {
        let Inbound::Request { id, method, params } = classify(json!({"jsonrpc":"2.0","id":1,"method":"hello","params":{"a":1}})) else {
            panic!("expected a request");
        };
        assert_eq!(id, json!(1));
        assert_eq!(method, "hello");
        assert_eq!(params, Some(json!({"a": 1})));
    }

    #[test]
    fn a_message_with_only_method_is_a_notification() {
        let Inbound::Notification { method, params } = classify(json!({"jsonrpc":"2.0","method":"host/heartbeat","params":{"t":5}})) else {
            panic!("expected a notification");
        };
        assert_eq!(method, "host/heartbeat");
        assert_eq!(params, Some(json!({"t": 5})));
    }

    #[test]
    fn a_message_with_only_id_is_a_response() {
        let Inbound::Response { id, outcome } = classify(json!({"jsonrpc":"2.0","id":4,"result":{"protocol":"quorfloat/1"}})) else {
            panic!("expected a response");
        };
        assert_eq!(id, json!(4));
        assert_eq!(outcome, Ok(json!({"protocol": "quorfloat/1"})));
    }

    #[test]
    fn an_error_response_keeps_its_code_and_detail() {
        // The mismatch case is the one that matters: `data.supported` is what a
        // human needs to see to know which version to build.
        let Inbound::Response { outcome, .. } = classify(json!({
            "jsonrpc": "2.0", "id": 1,
            "error": { "code": -32001, "message": "unsupported protocol version: x",
                       "data": { "supported": ["quorfloat/1"] } }
        })) else {
            panic!("expected a response");
        };
        let Err(error) = outcome else { panic!("expected an error") };
        assert_eq!(error.code, error_code::PROTOCOL_MISMATCH);
        assert_eq!(error.data.unwrap()["supported"][0], "quorfloat/1");
    }

    #[test]
    fn an_unreadable_error_object_still_settles_the_request() {
        // Otherwise a malformed error leaves a request pending forever, which is
        // the hang this protocol spends the most effort avoiding elsewhere.
        let Inbound::Response { outcome, .. } =
            classify(json!({"jsonrpc":"2.0","id":2,"error":"not an object"}))
        else {
            panic!("expected a response");
        };
        assert!(matches!(outcome, Err(error) if error.code == error_code::INTERNAL_ERROR));
    }

    #[test]
    fn junk_is_malformed_rather_than_a_wrong_kind() {
        assert_eq!(classify(json!([1, 2, 3])), Inbound::Malformed);
        assert_eq!(classify(json!({"jsonrpc": "2.0"})), Inbound::Malformed);
        assert_eq!(classify(json!("a string")), Inbound::Malformed);
    }

    #[test]
    fn ping_is_answered_and_anything_else_is_method_not_found() {
        let router = Router;
        assert!(matches!(router.handle("ping", None), Outcome::Ok(_)));
        // A half-built peer that answered `Ok` here would look finished to the
        // host's diagnostics while being unable to do any of the work.
        assert_eq!(
            router.handle("session/create", None),
            Outcome::unknown_method("session/create"),
        );
    }

    #[test]
    fn shutdown_is_accepted_so_the_host_does_not_escalate() {
        // If this returned `unknown_method`, the host would answer its own
        // `shutdown` with a failure, time out, and SIGTERM us — turning every
        // orderly stop into a kill it then reports as unclean.
        let router = Router;
        assert_eq!(router.handle("shutdown", None), Outcome::Accepted);
    }

    #[test]
    fn a_frame_ends_with_exactly_one_newline() {
        let bytes = encode(&success(&json!(1), json!({})));
        assert_eq!(bytes.last(), Some(&b'\n'));
        assert_eq!(bytes.iter().filter(|byte| **byte == b'\n').count(), 1);
    }

    #[test]
    fn an_encoded_error_carries_null_data_rather_than_omitting_it() {
        // The host's validator reads `data` as optional, so both shapes work;
        // pinning one keeps the wire stable for anything that diffs frames.
        let error = RpcError { code: -32601, message: "nope".to_owned(), data: None };
        let value = failure(&json!(9), &error);
        assert_eq!(value["error"]["code"], -32601);
        assert_eq!(value["error"]["data"], Value::Null);
    }
}

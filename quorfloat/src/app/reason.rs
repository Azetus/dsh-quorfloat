//! Structured reasons for panel status and errors.
//!
//! The sidecar never writes the panel's sentences. A reason is a stable code plus the
//! values the frontend's dictionary interpolates; the same JSON shape reaches the WebView
//! from every state path. Adding a code without its dictionary entry is a frontend bug,
//! not a reason to fall back to a sentence written here.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

/// A machine-readable reason with the values its sentence needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Reason {
    /// The stable dictionary code, e.g. `delivery.createFailed`.
    code: &'static str,
    /// Values for the dictionary's `{placeholders}`; always strings on this wire.
    params: BTreeMap<String, String>,
}

impl Reason {
    /// Start a reason with no parameters.
    ///
    /// @param code - the dictionary key.
    /// @returns the reason.
    #[must_use]
    pub fn new(code: &'static str) -> Self {
        Self { code, params: BTreeMap::new() }
    }

    /// Add one template parameter.
    ///
    /// @param key - the placeholder name.
    /// @param value - the value to interpolate.
    /// @returns the reason with the parameter set.
    #[must_use]
    pub fn with(mut self, key: &str, value: impl Into<String>) -> Self {
        self.params.insert(key.to_owned(), value.into());
        self
    }

    /// The dictionary code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        self.code
    }

    /// One parameter, usually `detail`, for markers and logs.
    ///
    /// @param key - the parameter name.
    /// @returns the value, when present.
    #[must_use]
    pub fn param(&self, key: &str) -> Option<&str> {
        self.params.get(key).map(String::as_str)
    }

    /// The JSON shape the frontend reads.
    #[must_use]
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::Reason;
    use serde_json::json;

    #[test]
    fn a_reason_is_a_code_and_its_template_values() {
        let reason = Reason::new("delivery.writeFailed").with("detail", "broken pipe");
        assert_eq!(reason.code(), "delivery.writeFailed");
        assert_eq!(reason.param("detail"), Some("broken pipe"));
        assert_eq!(
            reason.to_json(),
            json!({ "code": "delivery.writeFailed", "params": { "detail": "broken pipe" } }),
        );
    }
}

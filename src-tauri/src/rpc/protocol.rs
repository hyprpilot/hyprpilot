use serde::{Deserialize, Serialize};

/// Marker type that serializes / deserializes as the literal `"2.0"`. Any
/// other JSON-RPC version on the wire is a `-32600` invalid request.
///
/// `Serialize` / `Deserialize` are hand-rolled rather than derived because a
/// derived impl cannot enforce the literal value — deserializing
/// `{"jsonrpc":"1.0",...}` would succeed silently. This is the one
/// intentional exception to the "derive everything in `protocol.rs`" rule.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JsonRpcVersion;

impl Serialize for JsonRpcVersion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str("2.0")
    }
}

impl<'de> Deserialize<'de> for JsonRpcVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error as _;

        let s = String::deserialize(deserializer)?;
        if s != "2.0" {
            return Err(D::Error::custom(format!(
                "unsupported jsonrpc version {s:?}, expected \"2.0\""
            )));
        }

        Ok(JsonRpcVersion)
    }
}

/// JSON-RPC request ids are either a number or a string. We accept both on
/// the wire and echo the same shape back on the response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum RequestId {
    Number(u64),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: JsonRpcVersion,
    pub id: RequestId,
    #[serde(flatten)]
    pub call: Call,
}

/// Method + params. `#[serde(tag = "method", content = "params")]` makes
/// each variant serialize as `{"method": "...", "params": {...}}`. Unit
/// variants omit `params` entirely, and the spec allows that — tests assert
/// deserialization of `{"method":"toggle"}` (no `params` key) succeeds.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "kebab-case")]
pub enum Call {
    Submit { text: String },
    Cancel,
    Toggle,
    Kill,
    SessionInfo,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: JsonRpcVersion,
    pub id: Option<RequestId>,
    #[serde(flatten)]
    pub outcome: Outcome,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Outcome {
    Success { result: serde_json::Value },
    Error { error: RpcError },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl RpcError {
    pub fn parse_error() -> Self {
        Self {
            code: -32700,
            message: "parse error".into(),
            data: None,
        }
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self {
            code: -32600,
            message: message.into(),
            data: None,
        }
    }

    pub fn method_not_found(method: &str) -> Self {
        Self {
            code: -32601,
            message: format!("method not found: {method}"),
            data: None,
        }
    }

    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self {
            code: -32602,
            message: message.into(),
            data: None,
        }
    }

    pub fn internal_error(message: impl Into<String>) -> Self {
        Self {
            code: -32603,
            message: message.into(),
            data: None,
        }
    }
}

impl Response {
    pub fn success(id: Option<RequestId>, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: JsonRpcVersion,
            id,
            outcome: Outcome::Success { result },
        }
    }

    pub fn error(id: Option<RequestId>, error: RpcError) -> Self {
        Self {
            jsonrpc: JsonRpcVersion,
            id,
            outcome: Outcome::Error { error },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_request_without_params_deserializes() {
        let raw = r#"{"jsonrpc":"2.0","id":1,"method":"toggle"}"#;
        let req: Request = serde_json::from_str(raw).expect("toggle with no params");

        assert_eq!(req.id, RequestId::Number(1));
        assert!(matches!(req.call, Call::Toggle));
    }

    #[test]
    fn submit_request_with_params() {
        let raw = r#"{"jsonrpc":"2.0","id":"x","method":"submit","params":{"text":"hi"}}"#;
        let req: Request = serde_json::from_str(raw).expect("submit");

        assert_eq!(req.id, RequestId::String("x".into()));
        match req.call {
            Call::Submit { text } => assert_eq!(text, "hi"),
            other => panic!("unexpected call: {other:?}"),
        }
    }

    #[test]
    fn session_info_is_kebab_case() {
        let raw = r#"{"jsonrpc":"2.0","id":1,"method":"session-info"}"#;
        let req: Request = serde_json::from_str(raw).expect("session-info");
        assert!(matches!(req.call, Call::SessionInfo));
    }

    #[test]
    fn jsonrpc_version_rejects_other_values() {
        let raw = r#"{"jsonrpc":"1.0","id":1,"method":"toggle"}"#;
        let err = serde_json::from_str::<Request>(raw).expect_err("bad version");
        assert!(err.to_string().contains("unsupported jsonrpc version"));
    }

    #[test]
    fn version_serializes_as_literal_two_point_zero() {
        let v = serde_json::to_string(&JsonRpcVersion).unwrap();
        assert_eq!(v, "\"2.0\"");
    }

    #[test]
    fn error_response_serializes_without_result_key() {
        let resp = Response::error(Some(RequestId::Number(7)), RpcError::parse_error());
        let encoded = serde_json::to_string(&resp).unwrap();
        assert!(encoded.contains("\"error\""));
        assert!(!encoded.contains("\"result\""));
    }
}

use serde::{Deserialize, Serialize};

/// Agent lifecycle state. ACP will drive transitions in K-239; for now every
/// code path reports `Idle`. `#[serde(rename_all = "kebab-case")]` maps variants
/// to the wire strings: `idle`, `streaming`, `awaiting`, `error`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentState {
    Idle,
    Streaming,
    Awaiting,
    Error,
}

/// Snapshot returned by `status/get` and `status/subscribe`, and carried as
/// params in `status/changed` notifications.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusResult {
    pub state: AgentState,
    pub visible: bool,
    pub active_session: Option<String>,
}

/// Server-push notification. Sent to all subscribers on every state
/// transition. Not a `Response` — it has no `id` and uses `method` + `params`
/// like a JSON-RPC notification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusChangedNotification {
    pub jsonrpc: JsonRpcVersion,
    /// Always the literal string `"status/changed"`.
    pub method: StatusChangedMethod,
    pub params: StatusResult,
}

/// Declare a zero-sized marker type that serializes / deserializes as a
/// single wire literal. Used for method-name literals embedded in structured
/// notification types, where a derived `Serialize` cannot enforce the exact
/// string.
///
/// Future namespaces (`session/*`, `window/*`, `daemon/*`) will reuse this
/// macro as soon as they land — drop in a new `wire_method!(FooBarMethod,
/// "foo/bar")` and embed it in the notification struct.
macro_rules! wire_method {
    ($name:ident, $literal:literal) => {
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
        pub struct $name;

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str($literal)
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                use serde::de::Error as _;
                let s = String::deserialize(d)?;
                if s != $literal {
                    return Err(D::Error::custom(format!("expected {:?}, got {s:?}", $literal)));
                }
                Ok($name)
            }
        }
    };
}

wire_method!(StatusChangedMethod, "status/changed");

impl StatusChangedNotification {
    pub fn new(params: StatusResult) -> Self {
        Self {
            jsonrpc: JsonRpcVersion,
            method: StatusChangedMethod,
            params,
        }
    }
}

wire_method!(EventsNotifyMethod, "events/notify");

/// Server-push notification routed to one `events/subscribe` consumer.
/// `subscriptionId` lets multiplexed clients route incoming lines to
/// the right local consumer; `topic` is the dot-separated wire string
/// (matches `InstanceEvent::topic()` for instance-bound topics);
/// `payload` is the raw event JSON (`adapters::InstanceEvent`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventsNotifyParams {
    pub subscription_id: String,
    pub topic: crate::rpc::topic::WireTopic,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EventsNotifyNotification {
    pub jsonrpc: JsonRpcVersion,
    pub method: EventsNotifyMethod,
    pub params: EventsNotifyParams,
}

impl EventsNotifyNotification {
    pub fn new(params: EventsNotifyParams) -> Self {
        Self {
            jsonrpc: JsonRpcVersion,
            method: EventsNotifyMethod,
            params,
        }
    }
}

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

    pub const CODE_METHOD_NOT_FOUND: i32 = -32601;
    pub const CODE_INVALID_PARAMS: i32 = -32602;
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
    fn version_serializes_as_literal_two_point_zero() {
        let v = serde_json::to_string(&JsonRpcVersion).unwrap();
        assert_eq!(v, "\"2.0\"");
    }

    #[test]
    fn jsonrpc_version_deserialize_rejects_other_strings() {
        // The `JsonRpcVersion` marker is the one typed barrier between
        // the wire envelope and a loose `Value` — verify it rejects
        // anything other than "2.0". `dispatch_line` feeds this check
        // with `serde_json::from_value::<JsonRpcVersion>(v.clone())`
        // and surfaces `-32600 invalid_request` on the resulting error.
        let err = serde_json::from_str::<JsonRpcVersion>(r#""1.0""#).expect_err("bad version");
        assert!(err.to_string().contains("unsupported jsonrpc version"));

        assert!(serde_json::from_str::<JsonRpcVersion>(r#""2""#).is_err());
        assert!(serde_json::from_str::<JsonRpcVersion>(r#""2.0.0""#).is_err());
        assert!(serde_json::from_str::<JsonRpcVersion>(r#""2.0""#).is_ok());
    }

    #[test]
    fn error_response_serializes_without_result_key() {
        let resp = Response::error(Some(RequestId::Number(7)), RpcError::parse_error());
        let encoded = serde_json::to_string(&resp).unwrap();
        assert!(encoded.contains("\"error\""));
        assert!(!encoded.contains("\"result\""));
    }

    #[test]
    fn agent_state_serializes_kebab_case() {
        assert_eq!(serde_json::to_string(&AgentState::Idle).unwrap(), "\"idle\"");
        assert_eq!(serde_json::to_string(&AgentState::Streaming).unwrap(), "\"streaming\"");
        assert_eq!(serde_json::to_string(&AgentState::Awaiting).unwrap(), "\"awaiting\"");
        assert_eq!(serde_json::to_string(&AgentState::Error).unwrap(), "\"error\"");
    }

    #[test]
    fn status_result_round_trips() {
        let sr = StatusResult {
            state: AgentState::Idle,
            visible: true,
            active_session: None,
        };
        let encoded = serde_json::to_string(&sr).unwrap();
        let decoded: StatusResult = serde_json::from_str(&encoded).unwrap();
        assert_eq!(sr, decoded);
    }

    #[test]
    fn status_changed_notification_round_trips() {
        let n = StatusChangedNotification::new(StatusResult {
            state: AgentState::Streaming,
            visible: true,
            active_session: Some("sess-1".into()),
        });
        let encoded = serde_json::to_string(&n).unwrap();
        let decoded: StatusChangedNotification = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded.params.state, AgentState::Streaming);
        assert_eq!(decoded.params.active_session.as_deref(), Some("sess-1"));

        // Method literal must be the exact string.
        let v: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(v["method"], "status/changed");
    }

    #[test]
    fn status_changed_method_rejects_other_strings() {
        let bad = r#""status-changed""#;
        assert!(serde_json::from_str::<StatusChangedMethod>(bad).is_err());
    }

    #[test]
    fn request_id_string_uuid_round_trips() {
        // UUID v4 ids are what `CtlConnection` emits per call; verify the
        // `RequestId` untagged enum serializes and deserializes them
        // verbatim as strings (not numbers).
        let id_str = "550e8400-e29b-41d4-a716-446655440000";
        let id = RequestId::String(id_str.into());
        let encoded = serde_json::to_string(&id).unwrap();
        assert_eq!(encoded, format!("\"{id_str}\""));

        let decoded: RequestId = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, RequestId::String(id_str.into()));
    }

    #[test]
    fn request_id_round_trips_both_numeric_and_uuid() {
        let numeric = RequestId::Number(42);
        assert_eq!(serde_json::to_string(&numeric).unwrap(), "42");
        let back: RequestId = serde_json::from_str("42").unwrap();
        assert_eq!(back, RequestId::Number(42));

        let uuid_like = uuid::Uuid::new_v4().to_string();
        let id = RequestId::String(uuid_like.clone());
        let wire = serde_json::to_string(&id).unwrap();
        let back: RequestId = serde_json::from_str(&wire).unwrap();
        assert_eq!(back, RequestId::String(uuid_like));
    }
}

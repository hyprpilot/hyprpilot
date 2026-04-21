use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::rpc::handler::{HandlerCtx, HandlerOutcome, RpcHandler};
use crate::rpc::protocol::RpcError;

/// Params for `session/submit`. Deserialized per-call from the raw
/// JSON-RPC `params` value. `deny_unknown_fields` mirrors the pattern
/// used throughout `config::*` — typos in a client payload surface as
/// `-32602 invalid_params` instead of being silently ignored.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SubmitParams {
    text: String,
    /// Optional — when omitted, the daemon uses `agents.active_agent`.
    /// K-239 wires this through `AcpSessions`; the current scaffold
    /// echoes the value back unchanged.
    #[serde(default)]
    #[allow(dead_code)]
    agent_id: Option<String>,
}

/// `session/*` namespace — `session/submit`, `session/cancel`,
/// `session/info`.
///
/// Today these are stubs that mirror the pre-K-239 behaviour of the
/// removed `CoreHandler` (echo-back submit, empty session list, no-op
/// cancel). K-239's ACP bridge replaces each `match` arm with a call
/// into `AcpSessions`; the method names, params, and result shapes stay
/// stable across that upgrade.
pub struct SessionHandler;

#[async_trait]
impl RpcHandler for SessionHandler {
    fn namespace(&self) -> &'static str {
        "session"
    }

    async fn handle(&self, method: &str, params: Value, _ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
        match method {
            "session/submit" => {
                let SubmitParams { text, .. } = serde_json::from_value(params)
                    .map_err(|e| RpcError::invalid_params(format!("session/submit params: {e}")))?;
                Ok(HandlerOutcome::Reply(json!({ "accepted": true, "text": text })))
            }
            "session/cancel" => Ok(HandlerOutcome::Reply(json!({
                "cancelled": false,
                "reason": "no active session",
            }))),
            "session/info" => Ok(HandlerOutcome::Reply(json!({ "sessions": [] }))),
            other => Err(RpcError::method_not_found(other)),
        }
    }
}

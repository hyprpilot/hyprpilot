use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

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
    /// Optional — when omitted, the daemon resolves the agent via the
    /// addressed profile (or `[agent] default` when no profile is set).
    #[serde(default)]
    agent_id: Option<String>,
    /// Optional — names a `[[profiles]]` entry whose model +
    /// system-prompt overlay applies to this submission.
    #[serde(default)]
    profile_id: Option<String>,
}

/// Optional `{ agent_id }` wrapper shared by `session/cancel`. Defaulted
/// to `{}` so `{"method":"session/cancel"}` (no `params` key) parses
/// cleanly.
#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct AgentAddress {
    agent_id: Option<String>,
}

/// `session/*` namespace — `session/submit`, `session/cancel`,
/// `session/info`.
///
/// Delegates every method into `AcpSessions` (Tauri managed state).
/// Today `AcpSessions` returns the pre-K-239 stubbed shapes; the live
/// ACP plumbing swaps those bodies in without touching this handler.
pub struct SessionHandler;

#[async_trait]
impl RpcHandler for SessionHandler {
    fn namespace(&self) -> &'static str {
        "session"
    }

    async fn handle(&self, method: &str, params: Value, ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
        let sessions = ctx
            .sessions
            .as_ref()
            .ok_or_else(|| RpcError::internal_error("AcpSessions not in managed state"))?;

        match method {
            "session/submit" => {
                let SubmitParams {
                    text,
                    agent_id,
                    profile_id,
                } = serde_json::from_value(params)
                    .map_err(|e| RpcError::invalid_params(format!("session/submit params: {e}")))?;
                let v = sessions
                    .submit(&text, agent_id.as_deref(), profile_id.as_deref())
                    .await?;
                Ok(HandlerOutcome::Reply(v))
            }
            "session/cancel" => {
                let AgentAddress { agent_id } = params_or_default::<AgentAddress>(params, method)?;
                let v = sessions.cancel(agent_id.as_deref()).await?;
                Ok(HandlerOutcome::Reply(v))
            }
            "session/info" => {
                let v = sessions.info().await?;
                Ok(HandlerOutcome::Reply(v))
            }
            other => Err(RpcError::method_not_found(other)),
        }
    }
}

/// Treat `Value::Null` as an empty `{}` for types that derive
/// `#[serde(default)]`. The `session/cancel` / `session/info` method
/// surface intentionally accepts no `params` key at all — which the
/// server hands us as `Null` — and users shouldn't have to type
/// `"params": {}` just to get past the deserializer.
fn params_or_default<T: serde::de::DeserializeOwned + Default>(params: Value, method: &str) -> Result<T, RpcError> {
    if params.is_null() {
        return Ok(T::default());
    }
    serde_json::from_value::<T>(params).map_err(|e| RpcError::invalid_params(format!("{method} params: {e}")))
}

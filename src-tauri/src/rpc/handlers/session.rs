use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use crate::adapters::{Attachment, UserTurnInput};
use crate::rpc::handler::{HandlerCtx, HandlerOutcome, RpcHandler};
use crate::rpc::handlers::util::{map_adapter_err, params_or_default};
use crate::rpc::protocol::RpcError;

/// Params for `session/submit`. Deserialized per-call from the raw
/// JSON-RPC `params` value. `deny_unknown_fields` mirrors the pattern
/// used throughout `config::*` — typos in a client payload surface as
/// `-32602 invalid_params` instead of being silently ignored.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SubmitParams {
    text: String,
    /// Palette-picked attachments for this turn. Each entry projects
    /// onto an ACP `ContentBlock::Resource` prepended before the
    /// text block (see `adapters::acp::mapping::build_prompt_blocks`).
    #[serde(default)]
    attachments: Vec<Attachment>,
    /// Optional — addresses a specific live instance by UUID. When
    /// omitted, a fresh UUID is minted and a new instance is spawned;
    /// when provided but not yet in the registry, the backend adopts
    /// the id (lets the webview push its user-turn optimistically
    /// against the known id before the RPC round-trip completes).
    #[serde(default)]
    instance_id: Option<String>,
    /// Optional — when omitted, the daemon resolves the agent via the
    /// addressed profile (or `[agent] default` when no profile is set).
    #[serde(default)]
    agent_id: Option<String>,
    /// Optional — names a `[[profiles]]` entry whose model +
    /// system-prompt overlay applies to this submission.
    #[serde(default)]
    profile_id: Option<String>,
}

/// Optional address wrapper shared by `session/cancel`. `instance_id`
/// addresses a specific live instance by UUID; `agent_id` is the
/// legacy fallback that cancels the first live instance for that
/// agent. Defaulted to `{}` so `{"method":"session/cancel"}` (no
/// `params` key) parses cleanly.
#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct CancelAddress {
    instance_id: Option<String>,
    agent_id: Option<String>,
}

/// `session/*` namespace — `session/submit`, `session/cancel`,
/// `session/info`. Delegates every method through the `Adapter`
/// trait.
pub struct SessionHandler;

#[async_trait]
impl RpcHandler for SessionHandler {
    fn namespace(&self) -> &'static str {
        "session"
    }

    async fn handle(&self, method: &str, params: Value, ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
        let adapter = ctx
            .adapter
            .as_ref()
            .ok_or_else(|| RpcError::internal_error("adapter not in managed state"))?;

        match method {
            "session/submit" => {
                let SubmitParams {
                    text,
                    attachments,
                    instance_id,
                    agent_id,
                    profile_id,
                } = serde_json::from_value(params)
                    .map_err(|e| RpcError::invalid_params(format!("session/submit params: {e}")))?;
                let v = adapter
                    .submit(
                        UserTurnInput::with_attachments(text, attachments),
                        instance_id.as_deref(),
                        agent_id.as_deref(),
                        profile_id.as_deref(),
                    )
                    .await
                    .map_err(map_adapter_err)?;
                Ok(HandlerOutcome::Reply(v))
            }
            "session/cancel" => {
                let CancelAddress { instance_id, agent_id } = params_or_default::<CancelAddress>(params, method)?;
                let v = adapter
                    .cancel(instance_id.as_deref(), agent_id.as_deref())
                    .await
                    .map_err(map_adapter_err)?;
                Ok(HandlerOutcome::Reply(v))
            }
            "session/info" => {
                let v = adapter.info().await.map_err(map_adapter_err)?;
                Ok(HandlerOutcome::Reply(v))
            }
            other => Err(RpcError::method_not_found(other)),
        }
    }
}

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::Manager;

use crate::daemon::WindowRenderer;
use crate::rpc::handler::{HandlerCtx, HandlerOutcome, RpcHandler};
use crate::rpc::protocol::RpcError;

/// Params for the `submit` method. Deserialized per-call from the raw
/// JSON-RPC `params` value. `deny_unknown_fields` mirrors the pattern
/// used throughout `config::*` — typos in a client payload surface as
/// `-32602 invalid_params` instead of being silently ignored.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SubmitParams {
    text: String,
}

/// Scaffold RPC methods — un-namespaced legacy names from the original
/// daemon contract: `submit`, `cancel`, `toggle`, `kill`, `session-info`.
///
/// These predate the `<namespace>/<verb>` convention, so the handler
/// reports `namespace() = ""`. Methods match on the full literal. If a
/// future major version moves them under a namespace (e.g. `agent/submit`,
/// `window/toggle`, `daemon/kill`), split them into dedicated handlers
/// and drop this one.
pub struct CoreHandler;

#[async_trait]
impl RpcHandler for CoreHandler {
    fn namespace(&self) -> &'static str {
        ""
    }

    async fn handle(&self, method: &str, params: Value, ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
        match method {
            "submit" => {
                let SubmitParams { text } = serde_json::from_value(params)
                    .map_err(|e| RpcError::invalid_params(format!("submit params: {e}")))?;
                Ok(HandlerOutcome::Reply(json!({ "accepted": true, "text": text })))
            }
            "cancel" => Ok(HandlerOutcome::Reply(json!({
                "cancelled": false,
                "reason": "no active session",
            }))),
            "session-info" => Ok(HandlerOutcome::Reply(json!({ "sessions": [] }))),
            "kill" => Ok(HandlerOutcome::Reply(json!({ "exiting": true }))),
            "toggle" => toggle_window(&ctx),
            other => Err(RpcError::method_not_found(other)),
        }
    }
}

fn toggle_window(ctx: &HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
    let app = ctx
        .app
        .ok_or_else(|| RpcError::internal_error("no app handle available"))?;
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| RpcError::internal_error("main window not available"))?;

    // Route show/hide through `WindowRenderer` so percent dimensions re-resolve
    // against the current monitor on every show transition (K-237). Falls back
    // to `-32603` if the renderer isn't in managed state — the daemon always
    // registers it, so this branch only fires in stripped-down test builds.
    let renderer = app
        .try_state::<WindowRenderer>()
        .ok_or_else(|| RpcError::internal_error("WindowRenderer not in managed state"))?;

    let visible = window
        .is_visible()
        .map_err(|e| RpcError::internal_error(format!("is_visible failed: {e}")))?;

    if visible {
        renderer
            .hide(&window)
            .map_err(|e| RpcError::internal_error(format!("hide failed: {e}")))?;
        ctx.status.set_visible(false);
        Ok(HandlerOutcome::Reply(json!({ "visible": false })))
    } else {
        renderer
            .show(&window)
            .map_err(|e| RpcError::internal_error(format!("show failed: {e}")))?;
        ctx.status.set_visible(true);
        Ok(HandlerOutcome::Reply(json!({ "visible": true })))
    }
}

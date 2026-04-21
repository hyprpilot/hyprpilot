use async_trait::async_trait;
use serde_json::{json, Value};
use tauri::Manager;

use crate::daemon::WindowRenderer;
use crate::rpc::handler::{HandlerCtx, HandlerOutcome, RpcHandler};
use crate::rpc::protocol::RpcError;

/// `window/*` namespace — overlay lifecycle.
///
/// Today only `window/toggle` ships; future methods under this
/// namespace include `window/show`, `window/hide`, `window/focus`.
pub struct WindowHandler;

#[async_trait]
impl RpcHandler for WindowHandler {
    fn namespace(&self) -> &'static str {
        "window"
    }

    async fn handle(&self, method: &str, _params: Value, ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
        match method {
            "window/toggle" => toggle_window(&ctx),
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

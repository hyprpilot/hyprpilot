use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::Manager;

use crate::daemon::WindowRenderer;
use crate::rpc::handler::{HandlerCtx, HandlerOutcome, RpcHandler};
use crate::rpc::handlers::util::{map_adapter_err, params_or_default};
use crate::rpc::protocol::RpcError;

/// `overlay/*` namespace — the surface a hyprland keybind talks to
/// (`bind = SUPER, space, exec, hyprpilot ctl overlay toggle`).
/// Distinct from `window/toggle` because every method here serializes
/// through `WindowRenderer::lock_present` and accepts an
/// `instanceId` to focus alongside the show.
pub struct OverlayHandler;

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ShowParams {
    instance_id: Option<String>,
}

#[async_trait]
impl RpcHandler for OverlayHandler {
    fn namespace(&self) -> &'static str {
        "overlay"
    }

    async fn handle(&self, method: &str, params: Value, ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
        match method {
            "overlay/show" => {
                let ShowParams { instance_id } = params_or_default::<ShowParams>(params, method)?;
                show(&ctx, instance_id.as_deref()).await
            }
            "overlay/hide" => hide(&ctx).await,
            "overlay/toggle" => toggle(&ctx).await,
            other => Err(RpcError::method_not_found(other)),
        }
    }
}

fn renderer_and_window(ctx: &HandlerCtx<'_>) -> Result<(WindowRenderer, tauri::WebviewWindow), RpcError> {
    let app = ctx
        .app
        .ok_or_else(|| RpcError::internal_error("no app handle available"))?;
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| RpcError::internal_error("main window not available"))?;
    let renderer = app
        .try_state::<WindowRenderer>()
        .ok_or_else(|| RpcError::internal_error("WindowRenderer not in managed state"))?
        .inner()
        .clone();
    Ok((renderer, window))
}

async fn show(ctx: &HandlerCtx<'_>, instance_id: Option<&str>) -> Result<HandlerOutcome, RpcError> {
    // Resolve up front so a bogus `instanceId` reports `-32602
    // invalid_params` even when the renderer step would otherwise
    // succeed — keeps the error model deterministic. Accepts UUID OR
    // captain-set name via `resolve_token`.
    let parsed_key = match instance_id {
        Some(id) => Some(
            ctx.adapter
                .resolve_token(id)
                .await
                .ok_or_else(|| RpcError::invalid_params(format!("instance '{id}' not found")))?,
        ),
        None => None,
    };

    let app = ctx
        .app
        .ok_or_else(|| RpcError::internal_error("no app handle available"))?;
    let (renderer, window) = renderer_and_window(ctx)?;
    let _guard = renderer.lock_present().await;

    let visible = window
        .is_visible()
        .map_err(|err| RpcError::internal_error(format!("is_visible failed: {err}")))?;
    if !visible {
        renderer
            .show_on_main(app, &window)
            .await
            .map_err(|err| RpcError::internal_error(format!("show failed: {err}")))?;
        ctx.status.set_visible(true);
    }
    let _ = window.set_focus();

    let focused = match parsed_key {
        Some(key) => {
            let key = ctx.adapter.focus(key).await.map_err(map_adapter_err)?;
            Some(key.as_string())
        }
        None => None,
    };

    Ok(HandlerOutcome::Reply(json!({
        "visible": true,
        "focusedInstanceId": focused,
    })))
}

async fn hide(ctx: &HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
    let app = ctx
        .app
        .ok_or_else(|| RpcError::internal_error("no app handle available"))?;
    let (renderer, window) = renderer_and_window(ctx)?;
    let _guard = renderer.lock_present().await;

    let visible = window
        .is_visible()
        .map_err(|err| RpcError::internal_error(format!("is_visible failed: {err}")))?;
    if visible {
        renderer
            .hide_on_main(app, &window)
            .await
            .map_err(|err| RpcError::internal_error(format!("hide failed: {err}")))?;
        ctx.status.set_visible(false);
    }
    Ok(HandlerOutcome::Reply(json!({ "visible": false })))
}

async fn toggle(ctx: &HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
    let app = ctx
        .app
        .ok_or_else(|| RpcError::internal_error("no app handle available"))?;
    let (renderer, window) = renderer_and_window(ctx)?;
    let _guard = renderer.lock_present().await;

    let visible = window
        .is_visible()
        .map_err(|err| RpcError::internal_error(format!("is_visible failed: {err}")))?;
    if visible {
        renderer
            .hide_on_main(app, &window)
            .await
            .map_err(|err| RpcError::internal_error(format!("hide failed: {err}")))?;
        ctx.status.set_visible(false);
        Ok(HandlerOutcome::Reply(json!({ "visible": false })))
    } else {
        renderer
            .show_on_main(app, &window)
            .await
            .map_err(|err| RpcError::internal_error(format!("show failed: {err}")))?;
        ctx.status.set_visible(true);
        let _ = window.set_focus();
        Ok(HandlerOutcome::Reply(json!({ "visible": true })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::permission::{DefaultPermissionController, PermissionController};
    use crate::adapters::{AcpAdapter, Adapter};
    use crate::config::Config;
    use crate::rpc::handler::HandlerCtx;
    use crate::rpc::handlers::overlay::OverlayHandler;

    use crate::rpc::status::StatusBroadcast;
    use std::sync::{Arc, RwLock};

    /// Drive the handler without a real Tauri app — `app: None` makes
    /// every show/hide branch surface `-32603 internal_error`. That's
    /// enough to assert routing, param parsing, and the focus
    /// delegation, which is the only branch that ever reaches the
    /// adapter.
    async fn dispatch(method: &str, params: Value) -> Value {
        let shared = Arc::new(RwLock::new(Config::default()));
        let adapter = Arc::new(AcpAdapter::with_shared_config(
            shared.clone(),
            Arc::new(StatusBroadcast::new(true)),
            Arc::new(DefaultPermissionController::new()) as Arc<dyn PermissionController>,
        ));
        let status = StatusBroadcast::new(true);
        let dyn_adapter: Arc<dyn Adapter> = adapter.clone();
        let ctx = HandlerCtx {
            app: None,
            status: &status,
            adapter: dyn_adapter,
            config: Some(shared),
            skills: None,
            mcps: None,
            already_subscribed: false,
            started_at: None,
            socket_path: None,
        };
        match OverlayHandler.handle(method, params, ctx).await {
            Ok(HandlerOutcome::Reply(v)) => v,
            Ok(HandlerOutcome::StatusSubscribed(v, _)) => v,
            Err(err) => json!({ "code": err.code, "message": err.message }),
        }
    }

    /// `overlay/hide` with `app: None` surfaces `-32603` (no panic).
    /// The production path returns `{"visible": false}` even when the
    /// window is already hidden — the test only pins the no-panic
    /// invariant; the visible-flip happy path is exercised by the
    /// renderer race-safety test below.
    #[tokio::test]
    async fn overlay_hide_without_app_is_internal_error_not_panic() {
        let v = dispatch("overlay/hide", Value::Null).await;
        assert_eq!(v["code"], -32603);
    }

    #[tokio::test]
    async fn overlay_show_without_app_is_internal_error() {
        let v = dispatch("overlay/show", Value::Null).await;
        assert_eq!(v["code"], -32603);
    }

    #[tokio::test]
    async fn overlay_toggle_without_app_is_internal_error() {
        let v = dispatch("overlay/toggle", Value::Null).await;
        assert_eq!(v["code"], -32603);
    }

    /// Unknown `instanceId` rejects with `-32602 invalid_params` —
    /// `InstanceKey::parse` fails on a malformed UUID. With `app: None`
    /// the renderer-and-window lookup fails first (`-32603`), so this
    /// test pins the param-shape side: a non-uuid string is the user
    /// error mode the adapter reports, not the missing-app one.
    #[tokio::test]
    async fn overlay_show_rejects_unknown_field() {
        let v = dispatch("overlay/show", json!({ "instance_id": "x" })).await;
        // `instance_id` (snake_case) doesn't match the camelCase serde
        // rename — `deny_unknown_fields` rejects it as -32602.
        assert_eq!(v["code"], -32602);
    }

    /// `overlay/<bogus>` falls through to method_not_found.
    #[tokio::test]
    async fn overlay_unknown_verb_is_method_not_found() {
        let v = dispatch("overlay/bogus", Value::Null).await;
        assert_eq!(v["code"], -32601);
    }

    /// `overlay/show` with a malformed `instanceId` rejects with
    /// `-32602 invalid_params` before touching the renderer — the
    /// uuid parse runs first so the error model stays deterministic
    /// regardless of window state.
    #[tokio::test]
    async fn overlay_show_with_unparseable_instance_id_is_invalid_params() {
        let v = dispatch("overlay/show", json!({ "instanceId": "not-a-uuid" })).await;
        assert_eq!(v["code"], -32602);
    }

    /// `overlay/show` with a well-formed but unknown `instanceId`
    /// rejects with `-32602` — `resolve_token` walks the registry
    /// (UUID parse OK, but the key isn't a known instance and no
    /// captain-set name matches) and returns None before the
    /// renderer step runs. Earlier behavior returned -32603 because
    /// the renderer lookup failed first; the new contract lifts
    /// instance-resolution above the renderer for a deterministic
    /// error model.
    #[tokio::test]
    async fn overlay_show_with_unknown_instance_id_is_invalid_params() {
        let v = dispatch(
            "overlay/show",
            json!({ "instanceId": "550e8400-e29b-41d4-a716-446655440000" }),
        )
        .await;
        assert_eq!(v["code"], -32602);
    }
}

//! `prompts/*` namespace — `prompts/send`, `prompts/cancel`.
//!
//! `prompts/send` is the seamlessly-scriptable surface. `instance_id`
//! is overloaded:
//!
//! - UUID or existing captain-set name → target that instance.
//! - Slug-shaped value that doesn't resolve → auto-spawn under the
//!   supplied spawn-flag bag (`agent_id`, `profile_id`, `cwd`,
//!   `mode`, `model`) and rename the new instance to that slug.
//! - Anything else → error.
//!
//! When `instance_id` is omitted, falls back to the focused instance;
//! if none, auto-spawns unnamed under the spawn-flag bag.
//!
//! `prompts/cancel` is the same resolve-or-focused shape minus the
//! spawn — you can't cancel an instance that doesn't exist.
//!
//! Attachment plumbing is intentionally absent from this surface;
//! palette-picked skills attach via `session/submit` instead.

use std::path::PathBuf;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::adapters::{validate_instance_name, InstanceKey, SpawnSpec, UserTurnInput};
use crate::rpc::handler::{HandlerCtx, HandlerOutcome, RpcHandler};
use crate::rpc::handlers::util::{map_adapter_err, parse_params};
use crate::rpc::protocol::RpcError;

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct SendParams {
    /// Overloaded: UUID or existing captain-set name → target that
    /// instance; slug-shaped value that doesn't resolve → auto-spawn
    /// and rename to that slug; None → fall back to focused, then
    /// auto-spawn unnamed under the spawn-flag bag.
    instance_id: Option<String>,
    text: String,
    /// Spawn-flag bag. Used only when no instance resolves (no
    /// `instance_id` AND no focused). Mirrors `instances/spawn`.
    agent_id: Option<String>,
    profile_id: Option<String>,
    cwd: Option<PathBuf>,
    mode: Option<String>,
    model: Option<String>,
    /// Append the text into the resolved instance's composer instead
    /// of dispatching it. Captain edits + submits at their own pace.
    /// Resolution flow is identical (instance_id → focused →
    /// auto-spawn) so `--draft` against an empty daemon still spawns
    /// and the new instance lands with the prompt staged in its
    /// composer.
    draft: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct CancelParams {
    /// UUID or captain-set name. None → fall back to focused.
    instance_id: Option<String>,
}

pub struct PromptsHandler;

#[async_trait]
impl RpcHandler for PromptsHandler {
    fn namespace(&self) -> &'static str {
        "prompts"
    }

    async fn handle(&self, method: &str, params: Value, ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
        let adapter = &ctx.adapter;

        match method {
            "prompts/send" => {
                let p: SendParams = parse_params(params, method)?;
                if p.text.is_empty() {
                    return Err(RpcError::invalid_params("prompts/send: text must not be empty"));
                }

                // Resolution: `instance_id` is overloaded.
                //  1. If it resolves to a live instance (UUID or
                //     existing name) → target that instance, no rename.
                //  2. Else if it slug-validates → auto-spawn under the
                //     supplied spawn-flag bag and rename the new
                //     instance to that slug. Captain types `ctl prompts
                //     send --instance feat-xyz "build it"` against an
                //     empty daemon and the new instance lands carrying
                //     `feat-xyz` as its addressable name.
                //  3. Else (UUID-shaped or otherwise not resolvable
                //     and not slug-shaped) → error.
                // None → fall back to focused → spawn unnamed.
                let mut spawn_name: Option<String> = None;
                let resolved = match &p.instance_id {
                    Some(token) => match adapter.resolve_token(token).await {
                        Some(k) => k,
                        None => {
                            let validated = validate_instance_name(token).map_err(|err| {
                                RpcError::invalid_params(format!(
                                    "instance '{token}' not found and not a valid name slug: {err}"
                                ))
                            })?;
                            spawn_name = Some(validated);
                            let spec = SpawnSpec {
                                profile_id: p.profile_id.clone(),
                                agent_id: p.agent_id.clone(),
                                cwd: p.cwd.clone(),
                                mode: p.mode.clone(),
                                model: p.model.clone(),
                            };
                            adapter.spawn(spec).await.map_err(map_adapter_err)?
                        }
                    },
                    None => match adapter.focused_id().await {
                        Some(k) => k,
                        None => {
                            // Auto-spawn path. Empty registry + no
                            // focused — spawn with the supplied flags
                            // (defaults fall through inside the adapter).
                            let spec = SpawnSpec {
                                profile_id: p.profile_id.clone(),
                                agent_id: p.agent_id.clone(),
                                cwd: p.cwd.clone(),
                                mode: p.mode.clone(),
                                model: p.model.clone(),
                            };
                            adapter.spawn(spec).await.map_err(map_adapter_err)?
                        }
                    },
                };

                // Apply the slug-as-name rename right after the new
                // instance lands. Errors (collision / bad-slug) propagate.
                if let Some(name) = spawn_name {
                    adapter.rename(resolved, Some(name)).await.map_err(map_adapter_err)?;
                }

                // Draft path: emit a `composer:draft-append` Tauri
                // event addressed to the resolved instance and return
                // without dispatching. UI's composer listens, appends
                // the text with a blank-line separator if there's
                // already content. Resolution went all the way through
                // so `ctl prompts send --draft --instance feat-xyz` on
                // an empty daemon spawns + names the instance, and the
                // new overlay lands with the prompt staged.
                if p.draft {
                    if let Some(app) = ctx.app.as_ref() {
                        use tauri::Emitter;
                        let payload = json!({
                            "instanceId": resolved.as_string(),
                            "text": p.text,
                        });
                        if let Err(err) = app.emit("composer:draft-append", payload) {
                            tracing::warn!(%err, "prompts/send: failed to emit composer:draft-append");
                        }
                    }
                    return Ok(HandlerOutcome::Reply(json!({
                        "accepted": false,
                        "drafted": true,
                        "instanceId": resolved.as_string(),
                        "turnId": Value::Null,
                        "sessionId": Value::Null,
                    })));
                }

                let v = adapter
                    .submit(
                        UserTurnInput::with_attachments(p.text, Vec::new()),
                        Some(resolved.as_string().as_str()),
                        None,
                        None,
                    )
                    .await
                    .map_err(map_adapter_err)?;

                let accepted = v.get("accepted").and_then(Value::as_bool).unwrap_or(false);
                let session_id = v.get("sessionId").cloned().unwrap_or(Value::Null);
                let resolved_instance_id = v
                    .get("instanceId")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| resolved.as_string());

                // Server-assigned turn ids ride a different path (K-281); the
                // existing actor stamps a turn_id internally but it isn't
                // surfaced through the submit reply. Returning null here
                // keeps the wire shape stable; the UI can correlate via
                // `acp:turn-started` events in the meantime.
                Ok(HandlerOutcome::Reply(json!({
                    "accepted": accepted,
                    "instanceId": resolved_instance_id,
                    "turnId": Value::Null,
                    "sessionId": session_id,
                })))
            }
            "prompts/cancel" => {
                let p: CancelParams = parse_params(params, method)?;
                let key = resolve_or_focused(adapter.as_ref(), p.instance_id.as_deref()).await?;
                let v = adapter
                    .cancel(Some(key.as_string().as_str()), None)
                    .await
                    .map_err(map_adapter_err)?;
                Ok(HandlerOutcome::Reply(v))
            }
            other => Err(RpcError::method_not_found(other)),
        }
    }
}

/// Shared resolve-or-fall-back helper for handlers whose target is an
/// existing instance (i.e. NOT `prompts/send`'s spawn path). Token →
/// `resolve_token`; None → focused; neither → `-32602`.
pub(crate) async fn resolve_or_focused(
    adapter: &dyn crate::adapters::Adapter,
    token: Option<&str>,
) -> Result<InstanceKey, RpcError> {
    match token {
        Some(t) => adapter
            .resolve_token(t)
            .await
            .ok_or_else(|| RpcError::invalid_params(format!("instance '{t}' not found"))),
        None => adapter
            .focused_id()
            .await
            .ok_or_else(|| RpcError::invalid_params("no focused instance and --instance not supplied")),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, RwLock};

    use serde_json::json;

    use super::*;
    use crate::adapters::permission::{DefaultPermissionController, PermissionController};
    use crate::adapters::{AcpAdapter, Adapter};
    use crate::config::Config;
    use crate::rpc::handler::HandlerCtx;

    use crate::rpc::status::StatusBroadcast;

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
            already_subscribed: false,
            started_at: None,
            socket_path: None,
        };
        match PromptsHandler.handle(method, params, ctx).await {
            Ok(HandlerOutcome::Reply(v)) => v,
            Ok(HandlerOutcome::StatusSubscribed(v, _)) => v,
            Err(err) => json!({ "code": err.code, "message": err.message }),
        }
    }

    #[tokio::test]
    async fn send_missing_instance_id_is_invalid_params() {
        let v = dispatch("prompts/send", json!({ "text": "hi" })).await;
        assert_eq!(v["code"], -32602);
    }

    #[tokio::test]
    async fn send_missing_text_is_invalid_params() {
        let v = dispatch("prompts/send", json!({ "instanceId": "x" })).await;
        assert_eq!(v["code"], -32602);
    }

    #[tokio::test]
    async fn cancel_missing_instance_id_is_invalid_params() {
        let v = dispatch("prompts/cancel", json!({})).await;
        assert_eq!(v["code"], -32602);
    }

    #[tokio::test]
    async fn unknown_verb_is_method_not_found() {
        let v = dispatch("prompts/bogus", Value::Null).await;
        assert_eq!(v["code"], -32601);
    }

    /// Unknown wire fields (e.g. a stale client shipping `attachments`)
    /// reject at parse time via `deny_unknown_fields`.
    #[tokio::test]
    async fn send_rejects_unknown_field() {
        let v = dispatch(
            "prompts/send",
            json!({
                "instanceId": "550e8400-e29b-41d4-a716-446655440000",
                "text": "hi",
                "attachments": [],
            }),
        )
        .await;
        assert_eq!(v["code"], -32602);
        let msg = v["message"].as_str().unwrap_or_default();
        assert!(msg.contains("prompts/send params:"), "shape error expected: {v}");
    }
}

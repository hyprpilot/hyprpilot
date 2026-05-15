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
//! Attachments piggyback on the same `Attachment` struct
//! `tauri/session_submit` accepts. The field is optional — captains
//! who send bare `text` keep working unchanged.

use std::path::PathBuf;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::adapters::transcript::Attachment;
use crate::adapters::{validate_instance_name, InstanceKey, SpawnSpec, UserTurnInput};
use crate::completion::hydration::TokenHydrators;
use crate::rpc::handler::{HandlerCtx, HandlerOutcome, RpcHandler};
use crate::rpc::handlers::util::{map_adapter_err, parse_params, spawn_or_restore};
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
    /// On the auto-spawn path, try to resume the most recently-updated
    /// session matching `(agent_id, profile_id, cwd)` instead of
    /// spawning fresh. Falls through to a fresh spawn when no
    /// matching session exists or the agent doesn't support session
    /// listing. Captain's "open my last session under this profile +
    /// cwd" shortcut.
    restore: bool,
    /// Attachments staged by the caller — same shape as
    /// `tauri/session_submit` accepts. Flow into
    /// `UserTurnInput::with_attachments`; the daemon's encoder emits
    /// them as ACP content blocks (resource / image per
    /// `build_prompt_blocks`). Optional, default empty.
    attachments: Vec<Attachment>,
    /// Kustomize-style overlay patches for the auto-spawn path.
    /// Ignored when `instance_id` resolves to a live instance (the
    /// instance's config is already frozen). Applied in declaration
    /// order; stored on the spawned instance for restart replay.
    with_config: Vec<Value>,
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
                let mut p: SendParams = parse_params(params, method)?;
                if p.text.is_empty() {
                    return Err(RpcError::invalid_params("prompts/send: text must not be empty"));
                }

                // **Inline-token hydration — daemon-side, transport-
                // agnostic.** When the caller's text contains
                // `#{<scheme>://<value>}` tokens (`#{skills://git-commit}`
                // and friends), resolve each match against the daemon's
                // `TokenHydrators` registry and append the resulting
                // `Attachment`s to whatever the caller supplied. This
                // is the same registry the Tauri `session_submit`
                // command + `tauri/session_submit` proxy already use —
                // hoisting the call here means `ctl`, the nvim plugin,
                // any future RPC client speaking `prompts/send` get
                // attachment hydration for free WITHOUT shipping the
                // hydrator surface into every frontend. Captain's
                // requirement: hydration is a daemon concern; clients
                // send raw text.
                //
                // Idempotent with the existing palette path: the Vue
                // overlay pre-resolves attachments client-side via
                // `useAttachments` and ships both `text` (with token
                // still visible) AND `attachments`. The hydrator
                // re-resolves the token and appends — slug duplicates
                // are accepted today; future work can dedup on slug
                // if it bites. Token text stays in the prompt so the
                // captain reads the reference in the rendered chat.
                //
                // `ctx.app` is the Tauri `AppHandle` that owns the
                // `TokenHydrators` state. When absent (unit-test
                // dispatch with no Tauri host), skip hydration —
                // captain-supplied `attachments` flow through
                // unchanged.
                if let Some(app) = ctx.app.as_ref() {
                    use tauri::Manager;
                    if let Some(hydrators) = app.try_state::<TokenHydrators>() {
                        let hydrated = hydrators.hydrate_all(&p.text).await;

                        if !hydrated.is_empty() {
                            tracing::debug!(count = hydrated.len(), "prompts/send: hydrated tokens");
                            p.attachments.extend(hydrated);
                        }
                    }
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
                // `restore` flag short-circuits the spawn path. When
                // set, both the slug-spawn and the bare-no-instance
                // branches first try `restore_latest_session` against
                // the same SpawnSpec; on a hit the resumed instance's
                // key is the resolved one. On a miss we fall through
                // to fresh-spawn via the existing path.
                let spec = SpawnSpec {
                    profile_id: p.profile_id.clone(),
                    agent_id: p.agent_id.clone(),
                    cwd: p.cwd.clone(),
                    mode: p.mode.clone(),
                    model: p.model.clone(),
                    config_patches: p.with_config.clone(),
                };

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
                            spawn_or_restore(adapter.as_ref(), spec.clone(), p.restore).await?
                        }
                    },
                    None => match adapter.focused_id().await {
                        Some(k) => k,
                        None => {
                            // Auto-spawn path. Empty registry + no
                            // focused — spawn (or restore + spawn-on-miss)
                            // with the supplied flags.
                            spawn_or_restore(adapter.as_ref(), spec.clone(), p.restore).await?
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
                        "disposition": "drafted",
                        "drafted": true,
                        "wasBusy": false,
                        "instanceId": resolved.as_string(),
                        "turnId": Value::Null,
                        "sessionId": Value::Null,
                    })));
                }

                let v = adapter
                    .submit(
                        UserTurnInput::with_attachments(p.text, p.attachments),
                        Some(resolved.as_string().as_str()),
                        None,
                        None,
                    )
                    .await
                    .map_err(map_adapter_err)?;

                let accepted = v.get("accepted").and_then(Value::as_bool).unwrap_or(false);
                let disposition = v
                    .get("disposition")
                    .and_then(Value::as_str)
                    .unwrap_or("sent")
                    .to_string();
                let was_busy = v.get("wasBusy").and_then(Value::as_bool).unwrap_or(false);
                let session_id = v.get("sessionId").cloned().unwrap_or(Value::Null);
                let resolved_instance_id = v
                    .get("instanceId")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| resolved.as_string());

                // `disposition` distinguishes immediate-send from
                // landed-behind-an-active-turn so external frontends
                // (nvim plugin, ws remote) can render queue-ish UI on
                // the latter without re-implementing busy detection.
                // `wasBusy` is the same signal as a typed bool for
                // callers that prefer to branch on it. The existing
                // actor stamps a turn_id internally but it isn't
                // surfaced through the submit reply — returning null
                // here keeps the wire shape stable; clients correlate
                // via `acp:turn-started` events.
                Ok(HandlerOutcome::Reply(json!({
                    "accepted": accepted,
                    "disposition": disposition,
                    "wasBusy": was_busy,
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
            mcps: None,
            already_subscribed: false,
            already_events_subscribed: false,
            started_at: None,
            socket_path: None,
        };
        match PromptsHandler.handle(method, params, ctx).await {
            Ok(HandlerOutcome::Reply(v)) => v,
            Ok(HandlerOutcome::StatusSubscribed(v, _)) => v,
            Ok(HandlerOutcome::EventsSubscribed(v, _, _)) => v,
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

    /// Unknown wire fields reject at parse time via
    /// `deny_unknown_fields`. `attachments` is now a known field —
    /// see `send_accepts_empty_attachments` / `send_accepts_attachments`
    /// — so the stray here is a different stale field.
    #[tokio::test]
    async fn send_rejects_unknown_field() {
        let v = dispatch(
            "prompts/send",
            json!({
                "instanceId": "550e8400-e29b-41d4-a716-446655440000",
                "text": "hi",
                "bogusField": true,
            }),
        )
        .await;
        assert_eq!(v["code"], -32602);
        let msg = v["message"].as_str().unwrap_or_default();
        assert!(msg.contains("prompts/send params:"), "shape error expected: {v}");
    }

    /// Empty `attachments` parses and falls through to the same
    /// resolve-or-spawn path as a bare prompt. The instance id is
    /// longer than the 64-char slug ceiling, so the call rejects at
    /// the resolution step with the slug-validation error — not at
    /// the parse step. Pinning the resolution-error path is what
    /// proves `attachments: []` is accepted by the parser.
    #[tokio::test]
    async fn send_accepts_empty_attachments() {
        let v = dispatch(
            "prompts/send",
            json!({
                "instanceId": "this-name-is-deliberately-longer-than-the-sixty-four-character-slug-ceiling",
                "text": "hi",
                "attachments": [],
            }),
        )
        .await;
        assert_eq!(v["code"], -32602);
        let msg = v["message"].as_str().unwrap_or_default();
        assert!(
            msg.contains("not a valid name slug"),
            "expected resolution-step error, got: {v}"
        );
    }

    /// Populated `attachments` parses (same field shape as
    /// `tauri/session_submit`). Reaches the resolution step, where
    /// the over-cap slug id rejects.
    #[tokio::test]
    async fn send_accepts_attachments() {
        let v = dispatch(
            "prompts/send",
            json!({
                "instanceId": "this-name-is-deliberately-longer-than-the-sixty-four-character-slug-ceiling",
                "text": "see attached",
                "attachments": [
                    {
                        "slug": "diagram-png",
                        "path": "/tmp/diagram.png",
                        "title": "Architecture",
                        "data": "iVBORw0KGgo=",
                        "mime": "image/png",
                    }
                ],
            }),
        )
        .await;
        assert_eq!(v["code"], -32602);
        let msg = v["message"].as_str().unwrap_or_default();
        assert!(
            msg.contains("not a valid name slug"),
            "expected resolution-step error, got: {v}"
        );
    }

    /// Dispatch helper for tests that need a real adapter (not the
    /// default zero-config one). Pairs with the dead-child agent
    /// pattern from `adapters/acp/instances.rs:spawn_threads_mode_…`
    /// so the actor reaches `Error` immediately without depending on
    /// a real ACP vendor binary.
    async fn dispatch_with_adapter(adapter: Arc<AcpAdapter>, method: &str, params: Value) -> Value {
        let status = StatusBroadcast::new(true);
        let dyn_adapter: Arc<dyn Adapter> = adapter.clone();
        let ctx = HandlerCtx {
            app: None,
            status: &status,
            adapter: dyn_adapter,
            config: None,
            mcps: None,
            already_subscribed: false,
            already_events_subscribed: false,
            started_at: None,
            socket_path: None,
        };
        match PromptsHandler.handle(method, params, ctx).await {
            Ok(HandlerOutcome::Reply(v)) => v,
            Ok(HandlerOutcome::StatusSubscribed(v, _)) => v,
            Ok(HandlerOutcome::EventsSubscribed(v, _, _)) => v,
            Err(err) => json!({ "code": err.code, "message": err.message }),
        }
    }

    fn dead_child_adapter() -> Arc<AcpAdapter> {
        let cfg: Config = toml::from_str(
            r#"
[agent]
default = "dead"

[[agents]]
id = "dead"
provider = "acp-claude-code"
command = "/bin/false"
"#,
        )
        .expect("config parses");
        Arc::new(AcpAdapter::new(cfg, Arc::new(StatusBroadcast::new(true))))
    }

    /// `prompts/send` against a slug-shaped instance id with an empty
    /// registry auto-spawns AND renames. Reply carries the spawned
    /// instance id + `disposition: "sent"` + `wasBusy: false` (no
    /// prior turn). Pinning the `disposition` shape matters for
    /// second-frontends (nvim, ws bridge) that gate "queue strip"
    /// UI on whether the prompt landed behind a running turn.
    #[tokio::test]
    async fn send_slug_spawns_and_renames_with_sent_disposition() {
        let adapter = dead_child_adapter();
        let v = dispatch_with_adapter(
            adapter,
            "prompts/send",
            json!({ "instanceId": "feat-xyz", "text": "build it" }),
        )
        .await;
        assert_eq!(v["disposition"], "sent", "fresh spawn must report sent: {v}");
        assert_eq!(v["wasBusy"], false, "empty registry is never busy: {v}");
        assert_eq!(v["accepted"], true, "actor channel accepted the prompt: {v}");
        assert!(
            v["instanceId"].as_str().is_some_and(|s| !s.is_empty()),
            "minted instance id must surface on the reply: {v}"
        );
    }

    /// `prompts/send` with `draft: true` short-circuits the adapter
    /// submit and replies with `disposition: "drafted"`. Resolution
    /// still happens (the new instance is spawned + named), so the
    /// captain can hit `ctl prompts send --draft --instance feat-xyz`
    /// on an empty daemon and the overlay lands with the prompt
    /// staged in the composer.
    #[tokio::test]
    async fn send_draft_path_reports_drafted_disposition() {
        let adapter = dead_child_adapter();
        let v = dispatch_with_adapter(
            adapter,
            "prompts/send",
            json!({ "instanceId": "feat-draft", "text": "staged", "draft": true }),
        )
        .await;
        assert_eq!(v["disposition"], "drafted", "draft path must surface drafted: {v}");
        assert_eq!(v["drafted"], true);
        assert_eq!(
            v["accepted"], false,
            "draft does not dispatch — accepted stays false: {v}"
        );
        assert_eq!(v["wasBusy"], false);
        assert!(v["instanceId"].as_str().is_some_and(|s| !s.is_empty()));
    }

    /// Malformed `attachments` entry (missing required `slug`) rejects
    /// at parse time — pins that the field is typed, not opaque JSON.
    #[tokio::test]
    async fn send_rejects_malformed_attachment() {
        let v = dispatch(
            "prompts/send",
            json!({
                "instanceId": "550e8400-e29b-41d4-a716-446655440000",
                "text": "hi",
                "attachments": [{ "path": "/tmp/x" }],
            }),
        )
        .await;
        assert_eq!(v["code"], -32602);
        let msg = v["message"].as_str().unwrap_or_default();
        assert!(msg.contains("prompts/send params:"), "parse-error expected: {v}");
    }
}

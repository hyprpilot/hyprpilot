//! `tauri/<command>` namespace handler — bridges the daemon's
//! Tauri-command surface onto the JSON-RPC dispatcher so the
//! browser-side `remoteInvoke()` can drive the same surface the
//! embedded webview does over Tauri's IPC.
//!
//! The Tauri commands themselves are `#[tauri::command]`-decorated
//! free functions that receive `tauri::State<'_, T>` extractors.
//! Calling them programmatically through Tauri's own dispatcher
//! requires the full `Invoke<R>` machinery, which has no public
//! constructor. Instead, this proxy fetches the same managed state
//! via `app.try_state::<T>()` and replicates each command's body —
//! a duplication of logic, but the bodies are mostly tiny
//! delegations to adapter / registry methods so the cost is small.
//!
//! Coverage is the boot path + core interactions: theme / keymaps /
//! window state / daemon cwd + home, agents / profiles / instances,
//! session_submit / session_cancel / session_load / session_fork, permission_reply,
//! skills + completion + mcps. Commands the captain will hit before
//! the SPA paints. Less-common commands fall through to
//! `method_not_found` and the UI surfaces them as toast errors —
//! follow-up commits add coverage as captains find the gaps.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::adapters::transcript::Attachment;
use crate::adapters::{AcpAdapter, Adapter};
use crate::completion::dispatch as completion_dispatch;
use crate::completion::hydration::TokenHydrators;
use crate::completion::{CompletionCancellations, CompletionRegistry};
use crate::config::Config;
use crate::rpc::handler::{HandlerCtx, HandlerOutcome, RpcHandler};
use crate::rpc::handlers::util::{params_or_default, parse_params};
use crate::rpc::protocol::RpcError;
use crate::skills::{SkillSlug, SkillSummary, SkillsRegistry};

pub struct TauriProxyHandler;

#[async_trait]
impl RpcHandler for TauriProxyHandler {
    fn namespace(&self) -> &'static str {
        "tauri"
    }

    async fn handle(&self, method: &str, params: Value, ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
        let cmd = method
            .strip_prefix("tauri/")
            .ok_or_else(|| RpcError::method_not_found(method))?;

        let app = ctx
            .app
            .ok_or_else(|| RpcError::internal_error("tauri proxy requires a live AppHandle"))?;

        let value = dispatch(app, cmd, params, &ctx).await?;
        Ok(HandlerOutcome::Reply(value))
    }
}

/// Single match on `cmd`. Each arm fetches the state it needs from
/// `app.try_state` (or the dispatcher ctx, when shared), parses
/// params, calls the underlying logic, and returns a `Value`.
async fn dispatch(app: &tauri::AppHandle, cmd: &str, params: Value, ctx: &HandlerCtx<'_>) -> Result<Value, RpcError> {
    use tauri::Manager;

    match cmd {
        // ── boot path: aggregate snapshot (preferred) ────────────────
        // One round-trip replaces theme + keymaps + window_state +
        // home_dir + daemon_cwd + completion_config + agents + profiles
        // + instances. The granular handlers below stay for ad-hoc
        // refresh paths (palette reload, theme swap, etc.). Build via
        // the shared `daemon::build_boot_snapshot` so the JSON-RPC
        // mirror and the Tauri command can't drift.
        "boot_snapshot" => {
            let theme = app
                .try_state::<crate::config::Theme>()
                .ok_or_else(|| RpcError::internal_error("theme state not managed"))?;
            let keymaps = app
                .try_state::<crate::config::KeymapsConfig>()
                .ok_or_else(|| RpcError::internal_error("keymaps state not managed"))?;
            let window_state = app
                .try_state::<crate::daemon::WindowState>()
                .ok_or_else(|| RpcError::internal_error("window state not managed"))?;
            let config_state = app
                .try_state::<Arc<std::sync::RwLock<Config>>>()
                .ok_or_else(|| RpcError::internal_error("config state not managed"))?;
            let adapter = adapter_arc(app)?;
            let snap = crate::daemon::build_boot_snapshot(
                theme.inner(),
                keymaps.inner(),
                window_state.inner(),
                config_state.inner(),
                adapter.as_ref(),
            )
            .await
            .map_err(RpcError::internal_error)?;
            serde_json::to_value(snap).map_err(|e| RpcError::internal_error(format!("serialize boot snapshot: {e}")))
        }

        // ── boot path: theme + chrome ────────────────────────────────
        "get_theme" => {
            let theme = app
                .try_state::<crate::config::Theme>()
                .ok_or_else(|| RpcError::internal_error("theme state not managed"))?;
            Ok(serde_json::to_value(theme.inner().clone())
                .map_err(|e| RpcError::internal_error(format!("serialize theme: {e}")))?)
        }
        "get_keymaps" => {
            let keymaps = app
                .try_state::<crate::config::KeymapsConfig>()
                .ok_or_else(|| RpcError::internal_error("keymaps state not managed"))?;
            Ok(serde_json::to_value(keymaps.inner().clone())
                .map_err(|e| RpcError::internal_error(format!("serialize keymaps: {e}")))?)
        }
        "get_window_state" => {
            // `WindowState` is private to `daemon::mod`; serialize via
            // dynamic Value lookup. The frontend reads `mode` +
            // `anchorEdge`, both of which serialize as strings.
            let state = app
                .try_state::<crate::daemon::WindowState>()
                .ok_or_else(|| RpcError::internal_error("window state not managed"))?;
            Ok(serde_json::to_value(state.inner().clone())
                .map_err(|e| RpcError::internal_error(format!("serialize window state: {e}")))?)
        }
        "get_completion_config" => {
            let config = app
                .try_state::<Arc<std::sync::RwLock<Config>>>()
                .ok_or_else(|| RpcError::internal_error("config state not managed"))?;
            let cfg = config
                .read()
                .map_err(|e| RpcError::internal_error(format!("config rwlock poisoned: {e}")))?;
            let rg = &cfg.completion.ripgrep;
            Ok(json!({
                "ripgrep": {
                    "auto": rg.auto.unwrap_or(true),
                    "debounceMs": rg.debounce_ms.unwrap_or(250),
                    "minPrefix": rg.min_prefix.unwrap_or(3),
                }
            }))
        }
        "paths_resolve" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                raw: String,
                cwd_base: Option<String>,
            }
            let args: Args = parse_params(params, "tauri/paths_resolve")?;
            let home = crate::paths::home_dir();
            let home_str = home.to_string_lossy();
            let resolved = crate::tools::path::resolve_absolute(&args.raw, &home_str, args.cwd_base.as_deref());
            Ok(serde_json::to_value(resolved).unwrap_or(Value::Null))
        }

        // ── adapter surface (uses ctx.adapter, no app state needed) ──
        "agents_list" => {
            let adapter = adapter_arc(app)?;
            Ok(json!({ "agents": adapter.list_agents() }))
        }
        "profiles_list" => {
            let adapter = adapter_arc(app)?;
            Ok(json!({ "profiles": adapter.list_profiles() }))
        }
        "instances_list" => {
            let adapter = adapter_arc(app)?;
            let items = adapter.list().await;
            let focused_id = adapter.focused_id().await.map(|k| k.as_string());
            // Typed wire shape — same `InstanceListEntry` the
            // `instances/list` RPC + Tauri command + boot snapshot
            // ship. New fields (like `cwd`) flow uniformly across
            // every transport.
            let wire: Vec<crate::adapters::InstanceListEntry> =
                items.iter().map(crate::adapters::InstanceListEntry::from).collect();
            // See `instances_list` Tauri command — omit the
            // `focusedId` key when None so consumers see `undefined`
            // instead of `null`.
            let mut payload = serde_json::Map::with_capacity(2);
            payload.insert(
                "instances".into(),
                serde_json::to_value(&wire)
                    .map_err(|e| RpcError::internal_error(format!("serialize instances: {e}")))?,
            );
            if let Some(id) = focused_id {
                payload.insert("focusedId".into(), Value::String(id));
            }
            Ok(Value::Object(payload))
        }

        // ── core interactions ────────────────────────────────────────
        "session_submit" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                text: String,
                attachments: Option<Vec<Attachment>>,
                instance_id: Option<String>,
                agent_id: Option<String>,
                profile_id: Option<String>,
            }
            let args: Args = parse_params(params, "tauri/session_submit")?;
            let adapter = adapter_arc(app)?;
            let hydrators = app
                .try_state::<TokenHydrators>()
                .ok_or_else(|| RpcError::internal_error("hydrators state not managed"))?;
            let mut attachments = args.attachments.unwrap_or_default();
            let hydrated = hydrators.hydrate_all(&args.text).await;
            attachments.extend(hydrated);
            adapter
                .submit_prompt(
                    &args.text,
                    &attachments,
                    args.instance_id.as_deref(),
                    args.agent_id.as_deref(),
                    args.profile_id.as_deref(),
                )
                .await
                .map_err(|e| RpcError::internal_error(e.message))
        }
        "session_cancel" => {
            #[derive(Default, Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                instance_id: Option<String>,
                agent_id: Option<String>,
            }
            let args: Args = params_or_default(params, "tauri/session_cancel")?;
            let adapter = adapter_arc(app)?;
            adapter
                .cancel_active(args.instance_id.as_deref(), args.agent_id.as_deref())
                .await
                .map_err(|e| RpcError::internal_error(e.message))
        }
        "session_load" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Args {
                #[serde(default)]
                instance_id: Option<String>,
                #[serde(default)]
                agent_id: Option<String>,
                #[serde(default)]
                profile_id: Option<String>,
                session_id: String,
                #[serde(default)]
                cwd: Option<PathBuf>,
                #[serde(default)]
                with_config: Vec<Value>,
            }
            let args: Args = parse_params(params, "tauri/session_load")?;
            if args.session_id.trim().is_empty() {
                return Err(RpcError::invalid_params(
                    "tauri/session_load: sessionId must not be empty",
                ));
            }
            let adapter = adapter_arc(app)?;
            adapter
                .load_session(
                    args.instance_id.as_deref(),
                    args.agent_id.as_deref(),
                    args.profile_id.as_deref(),
                    args.session_id,
                    args.cwd,
                    args.with_config,
                )
                .await?;
            Ok(Value::Null)
        }
        "session_fork" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Args {
                #[serde(default)]
                instance_id: Option<String>,
                #[serde(default)]
                agent_id: Option<String>,
                #[serde(default)]
                profile_id: Option<String>,
                session_id: String,
                #[serde(default)]
                cwd: Option<PathBuf>,
                #[serde(default)]
                with_config: Vec<Value>,
            }
            let args: Args = parse_params(params, "tauri/session_fork")?;
            if args.session_id.trim().is_empty() {
                return Err(RpcError::invalid_params(
                    "tauri/session_fork: sessionId must not be empty",
                ));
            }
            let adapter = adapter_arc(app)?;
            let key = adapter
                .fork_session(
                    args.instance_id.as_deref(),
                    args.agent_id.as_deref(),
                    args.profile_id.as_deref(),
                    args.session_id,
                    args.cwd,
                    args.with_config,
                )
                .await?;
            Ok(json!({ "instanceId": key.as_string() }))
        }
        "instances_focus" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                instance_id: String,
            }
            let args: Args = parse_params(params, "tauri/instances_focus")?;
            let adapter = adapter_arc(app)?;
            let key = crate::adapters::InstanceKey::parse(&args.instance_id)
                .map_err(|e| RpcError::invalid_params(e.to_string()))?;
            let key = adapter
                .focus(key)
                .await
                .map_err(|e| RpcError::internal_error(e.to_string()))?;
            Ok(json!({ "instanceId": key.as_string() }))
        }
        "instances_shutdown" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                instance_id: String,
            }
            let args: Args = parse_params(params, "tauri/instances_shutdown")?;
            let adapter = adapter_arc(app)?;
            let key = crate::adapters::InstanceKey::parse(&args.instance_id)
                .map_err(|e| RpcError::invalid_params(e.to_string()))?;
            let key = adapter
                .shutdown_one(key)
                .await
                .map_err(|e| RpcError::internal_error(e.to_string()))?;
            Ok(json!({ "instanceId": key.as_string() }))
        }
        "instances_rename" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                instance_id: String,
                name: Option<String>,
            }
            let args: Args = parse_params(params, "tauri/instances_rename")?;
            let adapter = adapter_arc(app)?;
            let key = adapter
                .resolve_token(&args.instance_id)
                .await
                .ok_or_else(|| RpcError::invalid_params(format!("instance '{}' not found", args.instance_id)))?;
            adapter
                .rename(key, args.name.clone())
                .await
                .map_err(|e| RpcError::internal_error(e.to_string()))?;
            Ok(json!({ "instanceId": key.as_string(), "name": args.name }))
        }
        "instance_snapshot_meta" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                instance_id: String,
            }
            let args: Args = parse_params(params, "tauri/instance_snapshot_meta")?;
            let adapter = adapter_arc(app)?;
            let key = crate::adapters::InstanceKey::parse(&args.instance_id)
                .map_err(|e| RpcError::invalid_params(e.to_string()))?;
            let mirror = (adapter.as_ref() as &dyn Adapter)
                .instance_mirror(key)
                .await
                .ok_or_else(|| {
                    RpcError::invalid_params(format!("instance '{}' not found in registry", args.instance_id))
                })?;
            let snap = mirror.meta_snapshot().await;
            serde_json::to_value(snap).map_err(|e| RpcError::internal_error(format!("serialize meta snapshot: {e}")))
        }
        "instance_snapshot_chat" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                instance_id: String,
                #[serde(default)]
                before: Option<u64>,
                /// Delta-replay cursor — returns items strictly newer
                /// than the supplied seq, oldest-first. Mutually
                /// exclusive with `before`.
                #[serde(default)]
                after: Option<u64>,
                #[serde(default)]
                limit: Option<usize>,
            }
            let args: Args = parse_params(params, "tauri/instance_snapshot_chat")?;

            if args.before.is_some() && args.after.is_some() {
                return Err(RpcError::invalid_params(
                    "instance_snapshot_chat: `before` and `after` are mutually exclusive",
                ));
            }
            let adapter = adapter_arc(app)?;
            let key = crate::adapters::InstanceKey::parse(&args.instance_id)
                .map_err(|e| RpcError::invalid_params(e.to_string()))?;
            let mirror = (adapter.as_ref() as &dyn Adapter)
                .instance_mirror(key)
                .await
                .ok_or_else(|| {
                    RpcError::invalid_params(format!("instance '{}' not found in registry", args.instance_id))
                })?;
            let snap = mirror
                .chat_snapshot(args.before, args.after, args.limit.unwrap_or(0))
                .await;
            serde_json::to_value(snap).map_err(|e| RpcError::internal_error(format!("serialize chat snapshot: {e}")))
        }
        "instance_snapshot_terminals" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                instance_id: String,
            }
            let args: Args = parse_params(params, "tauri/instance_snapshot_terminals")?;
            let adapter = adapter_arc(app)?;
            let key = crate::adapters::InstanceKey::parse(&args.instance_id)
                .map_err(|e| RpcError::invalid_params(e.to_string()))?;
            let mirror = (adapter.as_ref() as &dyn Adapter)
                .instance_mirror(key)
                .await
                .ok_or_else(|| {
                    RpcError::invalid_params(format!("instance '{}' not found in registry", args.instance_id))
                })?;
            let snap = mirror.terminals_snapshot().await;
            serde_json::to_value(snap)
                .map_err(|e| RpcError::internal_error(format!("serialize terminals snapshot: {e}")))
        }
        "instance_meta" => {
            #[derive(Default, Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                instance_id: Option<String>,
                ensure: Option<bool>,
                agent_id: Option<String>,
                profile_id: Option<String>,
            }
            let args: Args = params_or_default(params, "tauri/instance_meta")?;
            let adapter = adapter_arc(app)?;
            if args.ensure.unwrap_or(false) {
                return adapter
                    .instance_meta_or_ensure(
                        args.instance_id.as_deref(),
                        args.agent_id.as_deref(),
                        args.profile_id.as_deref(),
                    )
                    .await
                    .map_err(|e| RpcError::internal_error(e.message));
            }
            let id = args
                .instance_id
                .ok_or_else(|| RpcError::invalid_params("instance_id required when ensure=false"))?;
            adapter
                .instance_meta(&id)
                .await
                .map_err(|e| RpcError::internal_error(e.message))
        }
        "permission_reply" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                request_id: String,
                option_id: String,
            }
            let args: Args = parse_params(params, "tauri/permission_reply")?;
            let controller = app
                .try_state::<Arc<dyn crate::adapters::permission::PermissionController>>()
                .ok_or_else(|| RpcError::internal_error("permission controller state not managed"))?;
            controller.resolve_if_pending(&args.request_id, &args.option_id).await;
            Ok(Value::Null)
        }
        // `tauri/models_set` / `tauri/modes_set` /
        // `tauri/config_option_set` were dropped — the canonical
        // wire is `instances/setModel` / `instances/setMode` /
        // `instances/setOption` (see `rpc/handlers/instances.rs`).
        // The Tauri commands themselves stay (the SPA invokes them
        // directly via the webview bridge); only the proxy
        // duplication is gone.
        "instance_restart" => {
            #[derive(Default, Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                instance_id: Option<String>,
                cwd: Option<PathBuf>,
                ensure: Option<bool>,
                agent_id: Option<String>,
                profile_id: Option<String>,
            }
            let args: Args = params_or_default(params, "tauri/instance_restart")?;
            let adapter = adapter_arc(app)?;
            let key = match args.instance_id.as_deref() {
                Some(id) => {
                    Some(crate::adapters::InstanceKey::parse(id).map_err(|e| RpcError::invalid_params(e.to_string()))?)
                }
                None => None,
            };
            let key = adapter
                .restart_instance(
                    key,
                    args.cwd,
                    args.ensure.unwrap_or(false),
                    args.agent_id.as_deref(),
                    args.profile_id.as_deref(),
                )
                .await
                .map_err(|e| RpcError::internal_error(e.message))?;
            Ok(json!({ "instanceId": key.as_string() }))
        }

        // ── session lookup by id (palette preview row) ──────────────
        "sessions_info" => {
            #[derive(Deserialize)]
            struct Args {
                id: String,
            }
            let args: Args = parse_params(params, "tauri/sessions_info")?;
            let adapter = adapter_arc(app)?;
            // Mirror `adapter_commands::sessions_info`: list + filter
            // (no ACP `session/get` verb). Default agent/profile fall
            // back to the resolved daemon defaults.
            let resp = adapter
                .list_sessions(None, None, None, None)
                .await
                .map_err(|e| RpcError::internal_error(e.message))?;
            let info = resp
                .sessions
                .iter()
                .find(|s| s.session_id.0.as_ref() == args.id.as_str())
                .ok_or_else(|| RpcError::invalid_params(format!("no session '{}'", args.id)))?;
            let (agent_id, profile_id) = {
                let cfg = adapter.config.read().expect("AcpAdapter config lock poisoned");
                // Every spawn flows through a profile — pick `[profile]
                // default`, fall back to the first entry, then read
                // its agent. Config-load validation ensures at least
                // one profile exists.
                let profile = cfg
                    .profile
                    .default
                    .as_deref()
                    .and_then(|id| cfg.profiles.iter().find(|p| p.id == id))
                    .or_else(|| cfg.profiles.first());
                let agent_id = profile.map(|p| p.agent.clone()).unwrap_or_default();
                let profile_id = profile.map(|p| p.id.clone());
                (agent_id, profile_id)
            };
            Ok(json!({
                "id": info.session_id.0.to_string(),
                "title": info.title,
                "cwd": info.cwd.display().to_string(),
                "lastTurnAt": info.updated_at,
                "messageCount": Value::Null,
                "agentId": agent_id,
                "profileId": profile_id,
            }))
        }

        // ── file attachment hydrator ────────────────────────────────
        "read_file_for_attachment" => {
            #[derive(Deserialize)]
            struct Args {
                path: String,
            }
            let args: Args = parse_params(params, "tauri/read_file_for_attachment")?;
            crate::completion::hydration::file::read_file_for_attachment(args.path)
                .await
                .map_err(RpcError::internal_error)
        }

        // ── desktop palette's daemon-ops bridge ─────────────────────
        // Recursive: routes a JSON-RPC method through the same
        // dispatcher we're already inside. Safe — palette ships a
        // hardcoded method whitelist (reload / shutdown / status /
        // version / diag-snapshot / window-toggle); none of those
        // are in the `tauri/` namespace, so no infinite-loop risk.
        "daemon_rpc" => {
            #[derive(Deserialize)]
            struct Args {
                method: String,
                params: Option<Value>,
            }
            let args: Args = parse_params(params, "tauri/daemon_rpc")?;
            let dispatcher = app
                .try_state::<Arc<crate::rpc::RpcDispatcher>>()
                .ok_or_else(|| RpcError::internal_error("dispatcher state not managed"))?;
            let nested_ctx = HandlerCtx {
                app: ctx.app,
                status: ctx.status,
                adapter: ctx.adapter.clone(),
                config: ctx.config.clone(),
                mcps: ctx.mcps.clone(),
                already_subscribed: ctx.already_subscribed,
                already_events_subscribed: ctx.already_events_subscribed,
                started_at: ctx.started_at,
                socket_path: ctx.socket_path,
            };
            match dispatcher
                .dispatch(&args.method, args.params.unwrap_or(Value::Null), nested_ctx)
                .await?
            {
                HandlerOutcome::Reply(v) => Ok(v),
                HandlerOutcome::StatusSubscribed(_, _) => Err(RpcError::internal_error(
                    "status/subscribe not supported via daemon_rpc",
                )),
                HandlerOutcome::EventsSubscribed(_, _, _) => Err(RpcError::internal_error(
                    "events/subscribe not supported via daemon_rpc",
                )),
            }
        }

        // ── remote-bridge management surface ────────────────────────
        // Most of the time these are desktop-only (the desktop modal
        // calls remote_confirm_pair / remote_reject_pair). Proxied
        // for completeness — a remote SPA running on the daemon's
        // own loopback would render the desktop modal too.
        "remote_confirm_pair" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                pending_id: String,
                code: String,
            }
            let args: Args = parse_params(params, "tauri/remote_confirm_pair")?;
            let pairs = app
                .try_state::<crate::remote::pair::PairStore>()
                .ok_or_else(|| RpcError::internal_error("pair store not managed"))?;
            let id = uuid::Uuid::parse_str(&args.pending_id)
                .map_err(|e| RpcError::invalid_params(format!("invalid pending_id: {e}")))?;
            match pairs.confirm(&id, &args.code, crate::remote::pair::ConfirmSide::Desktop) {
                Ok(()) => Ok(json!({ "confirmed": true })),
                Err(err) => Err(RpcError::internal_error(err.to_string())),
            }
        }
        "remote_reject_pair" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                pending_id: String,
            }
            let args: Args = parse_params(params, "tauri/remote_reject_pair")?;
            let pairs = app
                .try_state::<crate::remote::pair::PairStore>()
                .ok_or_else(|| RpcError::internal_error("pair store not managed"))?;
            let id = uuid::Uuid::parse_str(&args.pending_id)
                .map_err(|e| RpcError::invalid_params(format!("invalid pending_id: {e}")))?;
            pairs.reject(&id);
            Ok(Value::Null)
        }
        "remote_pending_pairs" => {
            let pairs = app
                .try_state::<crate::remote::pair::PairStore>()
                .ok_or_else(|| RpcError::internal_error("pair store not managed"))?;
            Ok(serde_json::to_value(pairs.snapshot()).unwrap_or(Value::Array(vec![])))
        }

        // ── session listing ──────────────────────────────────────────
        "session_list" => {
            #[derive(Default, Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                instance_id: Option<String>,
                agent_id: Option<String>,
                profile_id: Option<String>,
                cwd: Option<PathBuf>,
            }
            let args: Args = params_or_default(params, "tauri/session_list")?;
            let adapter = adapter_arc(app)?;
            adapter
                .list_sessions(
                    args.instance_id.as_deref(),
                    args.agent_id.as_deref(),
                    args.profile_id.as_deref(),
                    args.cwd,
                )
                .await
                .map(|r| serde_json::to_value(r).unwrap_or(Value::Null))
                .map_err(|e| RpcError::internal_error(e.message))
        }

        // ── skills (per-instance after #25) ──────────────────────────
        // Skills are owned by the addressed instance — `instance_id`
        // (when supplied) targets a specific live instance; unset →
        // focused instance. No live registry → empty list / 404 on
        // get, mirrors the desktop Tauri command's behaviour. See
        // `crate::skills::commands` for the canonical impl.
        "skills_list" | "skills_reload" => {
            #[derive(Default, Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                instance_id: Option<String>,
            }
            let args: Args = params_or_default(params, "tauri/skills_list")?;
            let adapter = adapter_arc(app)?;
            let registry = resolve_instance_skills(&adapter, args.instance_id.as_deref()).await;
            if cmd == "skills_reload" {
                let Some(reg) = registry else {
                    return Ok(json!({ "count": 0, "skills": [] }));
                };
                reg.reload()
                    .map_err(|e| RpcError::internal_error(format!("skills reload failed: {e:#}")))?;
                let list: Vec<SkillSummary> = reg.list().iter().map(SkillSummary::from).collect();
                Ok(json!({ "count": list.len(), "skills": list }))
            } else {
                let list: Vec<SkillSummary> = match registry {
                    Some(reg) => reg.list().iter().map(SkillSummary::from).collect(),
                    None => Vec::new(),
                };
                Ok(json!({ "skills": list }))
            }
        }
        "skills_get" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                instance_id: Option<String>,
                slug: String,
            }
            let args: Args = parse_params(params, "tauri/skills_get")?;
            let parsed = SkillSlug::parse(&args.slug)
                .map_err(|e| RpcError::invalid_params(format!("invalid slug '{}': {e}", args.slug)))?;
            let adapter = adapter_arc(app)?;
            let Some(reg) = resolve_instance_skills(&adapter, args.instance_id.as_deref()).await else {
                return Err(RpcError::invalid_params(format!(
                    "no live skills registry for slug '{}'",
                    args.slug
                )));
            };
            let skill = reg
                .get(&parsed)
                .ok_or_else(|| RpcError::invalid_params(format!("unknown skill '{}'", args.slug)))?;
            Ok(json!({
                "slug": skill.slug,
                "title": skill.title,
                "description": skill.description,
                "body": skill.body,
                "path": skill.path.display().to_string(),
                "references": skill.references,
            }))
        }

        // ── completion ───────────────────────────────────────────────
        "completion_query" => {
            #[derive(Default, Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                text: String,
                cursor: usize,
                cwd: Option<PathBuf>,
                manual: Option<bool>,
                sources: Option<Vec<String>>,
            }
            let args: Args = parse_params(params, "tauri/completion_query")?;
            let registry = app
                .try_state::<Arc<CompletionRegistry>>()
                .ok_or_else(|| RpcError::internal_error("completion registry state not managed"))?;
            let cancellations = app
                .try_state::<Arc<CompletionCancellations>>()
                .ok_or_else(|| RpcError::internal_error("completion cancellations state not managed"))?;
            completion_dispatch::run_query(
                registry.inner(),
                cancellations.inner(),
                &args.text,
                args.cursor,
                args.cwd.as_deref(),
                args.manual.unwrap_or(false),
                args.sources.as_deref(),
            )
            .await
        }
        "completion_resolve" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                resolve_id: String,
                source_id: String,
            }
            let args: Args = parse_params(params, "tauri/completion_resolve")?;
            let registry = app
                .try_state::<Arc<CompletionRegistry>>()
                .ok_or_else(|| RpcError::internal_error("completion registry state not managed"))?;
            completion_dispatch::run_resolve(registry.inner(), &args.resolve_id, &args.source_id).await
        }
        "completion_cancel" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                request_id: String,
            }
            let args: Args = parse_params(params, "tauri/completion_cancel")?;
            let cancellations = app
                .try_state::<Arc<CompletionCancellations>>()
                .ok_or_else(|| RpcError::internal_error("completion cancellations state not managed"))?;
            Ok(completion_dispatch::run_cancel(cancellations.inner(), &args.request_id))
        }
        "completion_rank" => {
            #[derive(Deserialize)]
            struct Args {
                query: String,
                candidates: Vec<crate::completion::source::candidates::CandidateItem>,
            }
            let args: Args = parse_params(params, "tauri/completion_rank")?;
            Ok(completion_dispatch::run_rank(&args.query, &args.candidates))
        }

        // ── mcps ─────────────────────────────────────────────────────
        "mcps_list" => {
            #[derive(Default, Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                instance_id: Option<String>,
            }
            let args: Args = params_or_default(params, "tauri/mcps_list")?;
            let adapter = adapter_arc(app)?;
            let catalog = adapter.resolve_mcp_catalog(args.instance_id.as_deref()).await;
            let items: Vec<Value> = catalog
                .iter()
                .map(|m| {
                    json!({
                        "name": m.name,
                        "raw": m.raw,
                        "hyprpilot": {
                            "autoAcceptTools": m.hyprpilot.auto_accept_tools,
                            "autoRejectTools": m.hyprpilot.auto_reject_tools,
                        },
                        "source": crate::tools::path::display_cwd(&m.source.to_string_lossy()),
                    })
                })
                .collect();
            Ok(json!({ "mcps": items }))
        }

        // ── window toggle (overlay-side has its own RPC, but the
        // webview commonly calls the Tauri version) ──────────────────
        "window_toggle" => {
            // Rather than re-implementing the renderer plumbing, route
            // via the existing `overlay/toggle` JSON-RPC handler we
            // already have. Same effect, single code path. Params
            // (`instanceId`) pass straight through; null/empty maps
            // to a bare toggle.
            let dispatcher = app
                .try_state::<Arc<crate::rpc::RpcDispatcher>>()
                .ok_or_else(|| RpcError::internal_error("dispatcher state not managed"))?;
            let nested_ctx = HandlerCtx {
                app: ctx.app,
                status: ctx.status,
                adapter: ctx.adapter.clone(),
                config: ctx.config.clone(),
                mcps: ctx.mcps.clone(),
                already_subscribed: ctx.already_subscribed,
                already_events_subscribed: ctx.already_events_subscribed,
                started_at: ctx.started_at,
                socket_path: ctx.socket_path,
            };
            match dispatcher.dispatch("overlay/toggle", params, nested_ctx).await? {
                HandlerOutcome::Reply(v) => Ok(v),
                HandlerOutcome::StatusSubscribed(_, _) => {
                    Err(RpcError::internal_error("overlay/toggle returned subscribe"))
                }
                HandlerOutcome::EventsSubscribed(_, _, _) => {
                    Err(RpcError::internal_error("overlay/toggle returned subscribe"))
                }
            }
        }

        // ── unknown ──────────────────────────────────────────────────
        other => Err(RpcError::method_not_found(&format!("tauri/{other}"))),
    }
}

fn adapter_arc(app: &tauri::AppHandle) -> Result<Arc<AcpAdapter>, RpcError> {
    use tauri::Manager;
    let state = app
        .try_state::<Arc<AcpAdapter>>()
        .ok_or_else(|| RpcError::internal_error("AcpAdapter state not managed"))?;
    Ok(state.inner().clone())
}

/// Mirror of `crate::skills::commands::resolve_registry` — `instance_id`
/// (when supplied) addresses a specific instance; an invalid / shut-
/// down id collapses to `None` so the palette stays silent rather
/// than erroring at the captain mid-typing. With no id the focused
/// instance serves the call.
async fn resolve_instance_skills(adapter: &Arc<AcpAdapter>, instance_id: Option<&str>) -> Option<Arc<SkillsRegistry>> {
    if let Some(raw) = instance_id {
        let key = crate::adapters::InstanceKey::parse(raw).ok()?;
        return adapter.instance_skills(key).await;
    }
    adapter.focused_skills().await
}

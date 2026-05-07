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
//! session_submit / session_cancel / session_load, permission_reply,
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
use crate::completion::hydration::TokenHydrators;
use crate::completion::{CompletionCancellations, CompletionRegistry, ReplacementRange};
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
        "get_home_dir" => Ok(json!(crate::paths::home_dir().to_string_lossy())),
        "get_daemon_cwd" => Ok(json!(std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "/".to_string()))),
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
        "get_git_status" => {
            #[derive(Deserialize)]
            struct Args {
                path: String,
            }
            let args: Args = parse_params(params, "tauri/get_git_status")?;
            crate::tools::git::snapshot(std::path::Path::new(&args.path))
                .map(|s| serde_json::to_value(s).unwrap_or(Value::Null))
                .map_err(|e| RpcError::internal_error(format!("git status failed: {e:#}")))
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
            let wire: Vec<Value> = items
                .iter()
                .map(|i| {
                    json!({
                        "agentId": i.agent_id,
                        "profileId": i.profile_id,
                        "instanceId": i.id,
                        "sessionId": i.session_id,
                        "name": i.name,
                        "mode": i.mode,
                    })
                })
                .collect();
            Ok(json!({ "instances": wire }))
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
            let hydrated = hydrators.hydrate_all(&args.text);
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
            #[serde(rename_all = "camelCase")]
            struct Args {
                instance_id: Option<String>,
                agent_id: Option<String>,
                profile_id: Option<String>,
                session_id: String,
                cwd: Option<PathBuf>,
            }
            let args: Args = parse_params(params, "tauri/session_load")?;
            let adapter = adapter_arc(app)?;
            adapter
                .load_session(
                    args.instance_id.as_deref(),
                    args.agent_id.as_deref(),
                    args.profile_id.as_deref(),
                    args.session_id,
                    args.cwd,
                )
                .await
                .map_err(|e| RpcError::internal_error(e.message))?;
            Ok(Value::Null)
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
        "models_set" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                instance_id: String,
                model_id: String,
            }
            let args: Args = parse_params(params, "tauri/models_set")?;
            let adapter = adapter_arc(app)?;
            adapter
                .set_session_model(&args.instance_id, &args.model_id)
                .await
                .map_err(|e| RpcError::internal_error(e.message))
        }
        "modes_set" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                instance_id: String,
                mode_id: String,
            }
            let args: Args = parse_params(params, "tauri/modes_set")?;
            let adapter = adapter_arc(app)?;
            adapter
                .set_session_mode(&args.instance_id, &args.mode_id)
                .await
                .map_err(|e| RpcError::internal_error(e.message))
        }
        "config_option_set" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                instance_id: String,
                config_id: String,
                value: String,
            }
            let args: Args = parse_params(params, "tauri/config_option_set")?;
            let adapter = adapter_arc(app)?;
            adapter
                .set_session_config_option(&args.instance_id, &args.config_id, &args.value)
                .await
                .map_err(|e| RpcError::internal_error(e.message))
        }
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

        // ── skills ───────────────────────────────────────────────────
        "skills_list" | "skills_reload" => {
            let skills = app
                .try_state::<Arc<SkillsRegistry>>()
                .ok_or_else(|| RpcError::internal_error("skills registry state not managed"))?;
            if cmd == "skills_reload" {
                skills
                    .reload()
                    .map_err(|e| RpcError::internal_error(format!("skills reload failed: {e:#}")))?;
            }
            let list: Vec<SkillSummary> = skills.list().iter().map(SkillSummary::from).collect();
            if cmd == "skills_reload" {
                Ok(json!({ "count": list.len(), "skills": list }))
            } else {
                Ok(json!({ "skills": list }))
            }
        }
        "skills_get" => {
            #[derive(Deserialize)]
            struct Args {
                slug: String,
            }
            let args: Args = parse_params(params, "tauri/skills_get")?;
            let skills = app
                .try_state::<Arc<SkillsRegistry>>()
                .ok_or_else(|| RpcError::internal_error("skills registry state not managed"))?;
            let parsed = SkillSlug::parse(&args.slug)
                .map_err(|e| RpcError::invalid_params(format!("invalid slug '{}': {e}", args.slug)))?;
            let skill = skills
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
                instance_id: Option<String>,
                sources: Option<Vec<String>>,
            }
            let args: Args = parse_params(params, "tauri/completion_query")?;
            let _ = args.instance_id;
            let registry = app
                .try_state::<Arc<CompletionRegistry>>()
                .ok_or_else(|| RpcError::internal_error("completion registry state not managed"))?;
            let cancellations = app
                .try_state::<Arc<CompletionCancellations>>()
                .ok_or_else(|| RpcError::internal_error("completion cancellations state not managed"))?;
            let request_id = uuid::Uuid::new_v4().to_string();
            let detected = registry.detect_filtered(
                &args.text,
                args.cursor,
                args.manual.unwrap_or(false),
                args.sources.as_deref(),
            );
            let (source, cctx) = match detected {
                Some(d) => d,
                None => {
                    return Ok(json!({
                        "requestId": request_id,
                        "sourceId": null,
                        "replacementRange": null,
                        "items": [],
                    }));
                }
            };
            let cancel = cancellations.new_token(&request_id);
            let range = ReplacementRange {
                start: cctx.trigger_offset,
                end: cctx.cursor,
            };
            let source_id = source.id();
            let result = source.fetch(cctx, args.cwd.as_deref(), cancel).await;
            cancellations.forget(&request_id);
            let items = result.map_err(|e| RpcError::internal_error(format!("completion/query: {e}")))?;
            Ok(json!({
                "requestId": request_id,
                "sourceId": source_id,
                "replacementRange": range,
                "items": items,
            }))
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
            let source = registry
                .source_by_id(&args.source_id)
                .ok_or_else(|| RpcError::invalid_params(format!("unknown source_id: {}", args.source_id)))?;
            let documentation = source
                .resolve(&args.resolve_id)
                .await
                .map_err(|e| RpcError::internal_error(format!("completion/resolve: {e}")))?;
            Ok(json!({ "documentation": documentation }))
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
            let cancelled = cancellations.cancel(&args.request_id);
            Ok(json!({ "cancelled": cancelled }))
        }
        "completion_rank" => {
            #[derive(Deserialize)]
            struct Args {
                query: String,
                candidates: Vec<crate::completion::source::candidates::CandidateItem>,
            }
            let args: Args = parse_params(params, "tauri/completion_rank")?;
            let request_id = uuid::Uuid::new_v4().to_string();
            let items = crate::completion::source::candidates::rank_candidates(&args.query, &args.candidates);
            Ok(json!({
                "requestId": request_id,
                "sourceId": "candidates",
                "replacementRange": null,
                "items": items,
            }))
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
                        "source": m.source.display().to_string(),
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
            // already have. Same effect, single code path.
            let dispatcher = app
                .try_state::<Arc<crate::rpc::RpcDispatcher>>()
                .ok_or_else(|| RpcError::internal_error("dispatcher state not managed"))?;
            let nested_ctx = HandlerCtx {
                app: ctx.app,
                status: ctx.status,
                adapter: ctx.adapter.clone(),
                config: ctx.config.clone(),
                skills: ctx.skills.clone(),
                mcps: ctx.mcps.clone(),
                already_subscribed: ctx.already_subscribed,
                started_at: ctx.started_at,
                socket_path: ctx.socket_path,
            };
            match dispatcher.dispatch("overlay/toggle", Value::Null, nested_ctx).await? {
                HandlerOutcome::Reply(v) => Ok(v),
                HandlerOutcome::StatusSubscribed(_, _) => {
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

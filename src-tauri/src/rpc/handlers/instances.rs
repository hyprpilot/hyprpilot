use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};

use crate::adapters::{validate_instance_name, InstanceKey, InstanceListEntry, SpawnSpec};
use crate::rpc::handler::{HandlerCtx, HandlerOutcome, RpcHandler};
use crate::rpc::handlers::util::{map_adapter_err, params_or_default, parse_params, spawn_or_restore};
use crate::rpc::protocol::RpcError;

/// `instances/shutdown` / `instances/info` — `instanceId` accepts
/// UUID OR captain-set name. None falls back to the daemon's
/// focused-instance pointer; an empty string rejects at the serde
/// layer with a clean message.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct InstanceParams {
    #[serde(deserialize_with = "deserialize_optional_non_empty_string")]
    instance_id: Option<String>,
}

/// `instances/focus` — same `instanceId` rules as `InstanceParams`,
/// plus optional auto-spawn behaviour. When `ensure: true` AND the
/// supplied `instanceId` is a slug-shaped name that doesn't resolve
/// to a live instance, the handler spawns one (using the supplied
/// spawn spec), renames it to the slug, then focuses. Mirrors
/// `prompts/send`'s resolve-or-spawn dance so a single keybind can
/// act as "open this named conversation, creating it if needed".
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct FocusParams {
    #[serde(deserialize_with = "deserialize_optional_non_empty_string")]
    instance_id: Option<String>,
    /// When `true` AND `instanceId` is supplied AND `instanceId`
    /// doesn't resolve to a live instance: spawn-and-rename instead
    /// of erroring.
    ensure: bool,
    profile_id: Option<String>,
    agent_id: Option<String>,
    cwd: Option<PathBuf>,
    mode: Option<String>,
    model: Option<String>,
    /// On the ensure-spawn path, try to resume the latest session
    /// matching `(agent_id, profile_id, cwd)` first. Falls through to
    /// a fresh spawn when no match exists.
    restore: bool,
    /// Kustomize-style overlay patches the daemon folds onto its
    /// resolved `Config` before the spawn proceeds. Applied in
    /// declaration order. Ignored on the no-spawn focus path.
    /// See `config::patch` for `$patch` directive semantics.
    with_config: Vec<serde_json::Value>,
}

/// `instances/restart` — `instanceId` optional (falls back to
/// focused), plus an optional `cwd` override. Missing / null `cwd`
/// preserves the resolved agent cwd.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct RestartParams {
    #[serde(deserialize_with = "deserialize_optional_non_empty_string")]
    instance_id: Option<String>,
    cwd: Option<PathBuf>,
}

/// `instances/rename` — change the addressable name on a live
/// instance. `name == None` clears the name; the slug rule is
/// enforced inside `Adapter::rename`. `instanceId` falls back to
/// focused when omitted (rename-the-current-one ergonomics).
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct RenameParams {
    #[serde(deserialize_with = "deserialize_optional_non_empty_string")]
    instance_id: Option<String>,
    name: Option<String>,
}

fn deserialize_optional_non_empty_string<'de, D>(de: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error;
    let opt = Option::<String>::deserialize(de)?;
    match opt {
        Some(s) if s.is_empty() => Err(D::Error::custom("instance id cannot be empty")),
        other => Ok(other),
    }
}

/// `instances/spawn` — every field is optional. Missing profile +
/// agent fall through to the adapter's default-chain, which rejects
/// with `-32602 invalid_params` when nothing resolves.
#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct SpawnParams {
    profile_id: Option<String>,
    agent_id: Option<String>,
    cwd: Option<PathBuf>,
    mode: Option<String>,
    model: Option<String>,
    /// Try to resume the latest session matching
    /// `(agent_id, profile_id, cwd)` first. Falls through to a fresh
    /// spawn when no matching session exists.
    restore: bool,
    /// Kustomize-style overlay patches the daemon folds onto its
    /// resolved `Config` before the spawn proceeds. Applied in
    /// declaration order. Stored on the spawned instance so
    /// `instances/restart` replays them against the daemon's
    /// then-current config.
    with_config: Vec<serde_json::Value>,
}

/// `instances/setMode` — switch active operational mode on a live
/// instance. Mode ids are wire ids advertised on
/// `MetaSnapshot::available_modes[].id` (i.e. the
/// `acp:instance-meta` / `instance/snapshot/meta` payload).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SetModeParams {
    instance_id: String,
    mode_id: String,
}

/// `instances/setModel` — switch active model on a live instance.
/// Model ids come from `MetaSnapshot::available_models[].id`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SetModelParams {
    instance_id: String,
    model_id: String,
}

/// `instances/setOption` — set an ACP `configOptions[]` value
/// (e.g. `effort = "high"`). `configId` is the option id; `value`
/// is one of the option's advertised `options[].value` strings.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SetOptionParams {
    instance_id: String,
    config_id: String,
    value: String,
}

/// `instances/setProfile` — swap the active profile on a live
/// instance under the SAME `instance_id`. Plugin / overlay chrome
/// keyed by `instance_id` (chat buffers, window state, queue strip,
/// permission row) stays addressable across the swap. The actor is
/// torn down + re-spawned under the new profile in `Bootstrap::ListOnly`
/// — no session is bound, the agent process serves `sessions/list`
/// for the new profile's history, and the captain picks a session
/// via `session_load` (or a fresh prompt requires binding a session
/// first).
///
/// `withConfig` is `Option<Vec<Value>>`: `None` (field absent) → keep
/// the captain's stored overlays from the original spawn / last
/// `instances/restart`; `Some(vec)` (even an empty list) → replace
/// the overlays with exactly that set. Captains who want to wipe
/// existing overlays pass `withConfig: []`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SetProfileParams {
    instance_id: String,
    profile_id: String,
    #[serde(default)]
    with_config: Option<Vec<serde_json::Value>>,
}

/// `instances/*` namespace. Registry-level operations on the
/// adapter: list, spawn, focus, restart, shutdown, info, plus
/// per-session setters (mode / model / config option). Delegates
/// every method through the `Adapter` trait; param validation +
/// error-mapping only.
pub struct InstancesHandler;

#[async_trait]
impl RpcHandler for InstancesHandler {
    fn namespace(&self) -> &'static str {
        "instances"
    }

    async fn handle(&self, method: &str, params: Value, ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
        let adapter = &ctx.adapter;

        match method {
            "instances/list" => {
                let items = adapter.list().await;
                // Typed wire shape — same `InstanceListEntry` the
                // Tauri `instances_list` command and `boot_snapshot`
                // ship. Centralising on the typed projection keeps
                // every transport (unix-socket RPC, Tauri webview,
                // remote WS) emitting the same field set; new
                // additions (like `cwd`) flow automatically.
                let wire: Vec<InstanceListEntry> = items.iter().map(InstanceListEntry::from).collect();
                Ok(HandlerOutcome::Reply(json!({ "instances": wire })))
            }
            "instances/focus" => {
                let FocusParams {
                    instance_id,
                    ensure,
                    profile_id,
                    agent_id,
                    cwd,
                    mode,
                    model,
                    restore,
                    with_config,
                } = params_or_default::<FocusParams>(params, method)?;

                let key = match (instance_id.as_deref(), ensure) {
                    // Ensure-mode with a token: try to resolve, else
                    // spawn-and-rename to that slug. Mirrors
                    // `prompts/send`'s overload. With `restore=true`
                    // the spawn step prefers `restore_latest_session`
                    // over a fresh spawn — falls through on a miss.
                    (Some(token), true) => match adapter.resolve_token(token).await {
                        Some(k) => k,
                        None => {
                            let slug = validate_instance_name(token).map_err(|err| {
                                RpcError::invalid_params(format!(
                                    "instance '{token}' not found and not a valid name slug: {err}"
                                ))
                            })?;
                            let spec = SpawnSpec {
                                profile_id,
                                agent_id,
                                cwd,
                                mode,
                                model,
                                config_patches: with_config,
                            };
                            let spawned = spawn_or_restore(adapter.as_ref(), spec, restore).await?;
                            adapter.rename(spawned, Some(slug)).await.map_err(map_adapter_err)?;
                            spawned
                        }
                    },
                    _ => resolve_or_focused(adapter.as_ref(), instance_id.as_deref()).await?,
                };
                let key = adapter.focus(key).await.map_err(map_adapter_err)?;
                Ok(HandlerOutcome::Reply(json!({ "instanceId": key.as_string() })))
            }
            "instances/spawn" => {
                let SpawnParams {
                    profile_id,
                    agent_id,
                    cwd,
                    mode,
                    model,
                    restore,
                    with_config,
                } = params_or_default::<SpawnParams>(params, method)?;
                let spec = SpawnSpec {
                    profile_id,
                    agent_id,
                    cwd,
                    mode,
                    model,
                    config_patches: with_config,
                };
                let key = spawn_or_restore(adapter.as_ref(), spec, restore).await?;
                Ok(HandlerOutcome::Reply(json!({ "instanceId": key.as_string() })))
            }
            "instances/restart" => {
                let RestartParams { instance_id, cwd } = params_or_default::<RestartParams>(params, method)?;
                let key = resolve_or_focused(adapter.as_ref(), instance_id.as_deref()).await?;
                let key = adapter.restart(key, cwd).await.map_err(map_adapter_err)?;
                Ok(HandlerOutcome::Reply(json!({ "instanceId": key.as_string() })))
            }
            "instances/shutdown" => {
                let InstanceParams { instance_id } = params_or_default::<InstanceParams>(params, method)?;
                let key = resolve_or_focused(adapter.as_ref(), instance_id.as_deref()).await?;
                let key = adapter.shutdown_one(key).await.map_err(map_adapter_err)?;
                Ok(HandlerOutcome::Reply(json!({ "instanceId": key.as_string() })))
            }
            "instances/info" => {
                let InstanceParams { instance_id } = params_or_default::<InstanceParams>(params, method)?;
                let key = resolve_or_focused(adapter.as_ref(), instance_id.as_deref()).await?;
                let info = adapter.info_for(key).await.map_err(map_adapter_err)?;
                // Same typed projection as `instances/list` so the
                // single-instance read shape stays consistent with
                // the list shape — including the new `cwd` field.
                let wire = serde_json::to_value(InstanceListEntry::from(&info))
                    .map_err(|e| RpcError::internal_error(format!("serialize instance info: {e}")))?;
                Ok(HandlerOutcome::Reply(wire))
            }
            "instances/rename" => {
                let RenameParams { instance_id, name } = params_or_default::<RenameParams>(params, method)?;
                let key = resolve_or_focused(adapter.as_ref(), instance_id.as_deref()).await?;
                adapter.rename(key, name.clone()).await.map_err(map_adapter_err)?;
                Ok(HandlerOutcome::Reply(json!({
                    "instanceId": key.as_string(),
                    "name": name,
                })))
            }
            "instances/setMode" => {
                let SetModeParams { instance_id, mode_id } = parse_params::<SetModeParams>(params, method)?;
                adapter
                    .set_session_mode(&instance_id, &mode_id)
                    .await
                    .map(HandlerOutcome::Reply)
                    .map_err(map_adapter_err)
            }
            "instances/setModel" => {
                let SetModelParams { instance_id, model_id } = parse_params::<SetModelParams>(params, method)?;
                adapter
                    .set_session_model(&instance_id, &model_id)
                    .await
                    .map(HandlerOutcome::Reply)
                    .map_err(map_adapter_err)
            }
            "instances/setOption" => {
                let SetOptionParams {
                    instance_id,
                    config_id,
                    value,
                } = parse_params::<SetOptionParams>(params, method)?;
                adapter
                    .set_session_config_option(&instance_id, &config_id, &value)
                    .await
                    .map(HandlerOutcome::Reply)
                    .map_err(map_adapter_err)
            }
            "instances/setProfile" => {
                let SetProfileParams {
                    instance_id,
                    profile_id,
                    with_config,
                } = parse_params::<SetProfileParams>(params, method)?;
                adapter
                    .set_session_profile(&instance_id, &profile_id, with_config)
                    .await
                    .map(HandlerOutcome::Reply)
                    .map_err(map_adapter_err)
            }
            other => Err(RpcError::method_not_found(other)),
        }
    }
}

use super::prompts::resolve_or_focused;

/// `instance/snapshot/*` namespace — read-only snapshots off each
/// per-instance [`crate::adapters::InstanceMirror`]. Three flavours:
///
/// - `instance/snapshot/meta` — header chrome + pending permissions +
///   usage tally + `latestSeq` cursor. Cheap; pulled on focus-switch.
/// - `instance/snapshot/chat` — windowed transcript page anchored at
///   `before` (cursor) with a default `limit` of 50. Caller paginates
///   backward by chaining `before = oldestSeq` from the previous page.
/// - `instance/snapshot/terminals` — full per-`terminal_id` map.
///
/// Coexists with the existing `instance_meta` Tauri command +
/// `permissions/pending` RPC: those return the "live actor" view via
/// `MetaSnapshot::meta_snapshot()` (actor command roundtrip) /
/// the controller's waiter map. The mirror snapshots are the
/// brim-sync surface — same underlying data, different read path
/// (no actor roundtrip, no waiter map). UI uses `instance/snapshot/*`
/// for focus-switch hydration; the legacy commands still serve their
/// existing call sites (palette pickers, permission stack listing).
pub struct InstanceSnapshotHandler;

/// `instance/snapshot/meta` params. `instanceId` is a UUID;
/// captain-set names go through `resolve_token` first if we ever
/// need them, but the snapshot surface deliberately addresses by
/// canonical key only — UI already knows the UUID it's hydrating.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct InstanceSnapshotMetaParams {
    instance_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct InstanceSnapshotChatParams {
    instance_id: String,
    /// Cursor: returned items are strictly older than this `seq`.
    /// Unset → return the latest `limit` entries (head-anchored).
    #[serde(default)]
    before: Option<u64>,
    /// Delta-replay cursor: returned items are strictly newer than
    /// this `seq`, oldest-first. Used by remote frontends on
    /// reconnect to catch up on missed events without re-fetching
    /// the entire transcript. Mutually exclusive with `before`.
    #[serde(default)]
    after: Option<u64>,
    /// Page size. `0` or unset → mirror's default page size (50).
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct InstanceSnapshotTerminalsParams {
    instance_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct InstanceSnapshotQueueParams {
    instance_id: String,
}

#[async_trait]
impl RpcHandler for InstanceSnapshotHandler {
    fn namespace(&self) -> &'static str {
        "instance"
    }

    async fn handle(&self, method: &str, params: Value, ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
        let adapter = &ctx.adapter;
        match method {
            "instance/snapshot/meta" => {
                let p: InstanceSnapshotMetaParams = parse_snapshot_params(params, method)?;
                let mirror = require_mirror(adapter.as_ref(), &p.instance_id).await?;
                let snap = mirror.meta_snapshot().await;
                serde_json::to_value(snap)
                    .map(HandlerOutcome::Reply)
                    .map_err(|e| RpcError::internal_error(format!("serialize meta snapshot: {e}")))
            }
            "instance/snapshot/chat" => {
                let p: InstanceSnapshotChatParams = parse_snapshot_params(params, method)?;

                if p.before.is_some() && p.after.is_some() {
                    return Err(RpcError::invalid_params(
                        "instance/snapshot/chat: `before` and `after` are mutually exclusive",
                    ));
                }
                let mirror = require_mirror(adapter.as_ref(), &p.instance_id).await?;
                let limit = p.limit.unwrap_or(0);
                let snap = mirror.chat_snapshot(p.before, p.after, limit).await;
                serde_json::to_value(snap)
                    .map(HandlerOutcome::Reply)
                    .map_err(|e| RpcError::internal_error(format!("serialize chat snapshot: {e}")))
            }
            "instance/snapshot/terminals" => {
                let p: InstanceSnapshotTerminalsParams = parse_snapshot_params(params, method)?;
                let mirror = require_mirror(adapter.as_ref(), &p.instance_id).await?;
                let snap = mirror.terminals_snapshot().await;
                serde_json::to_value(snap)
                    .map(HandlerOutcome::Reply)
                    .map_err(|e| RpcError::internal_error(format!("serialize terminals snapshot: {e}")))
            }
            "instance/snapshot/queue" => {
                let p: InstanceSnapshotQueueParams = parse_snapshot_params(params, method)?;
                let mirror = require_mirror(adapter.as_ref(), &p.instance_id).await?;
                let items = mirror.queue_snapshot().await;
                Ok(HandlerOutcome::Reply(json!({ "items": items })))
            }
            other => Err(RpcError::method_not_found(other)),
        }
    }
}

/// Snapshot params share a tighter shape than the rest of the
/// `instances/*` family — `instanceId` is required (no
/// fall-through to focused), so a missing field is `-32602`. The
/// inner parser is the standard `serde_json::from_value` against the
/// param struct; null/empty params reject explicitly because the
/// `deny_unknown_fields` derive doesn't catch a missing required
/// field on its own.
fn parse_snapshot_params<T: serde::de::DeserializeOwned>(params: Value, method: &str) -> Result<T, RpcError> {
    if params.is_null() {
        return Err(RpcError::invalid_params(format!("{method}: instanceId required")));
    }
    serde_json::from_value::<T>(params).map_err(|e| RpcError::invalid_params(format!("{method} params: {e}")))
}

/// Resolve `instance_id` → `Arc<InstanceMirror>`. `-32602` on either
/// a malformed UUID (parse failure) or an unknown id (no live
/// actor / no mirror). Mirrors the not-found shape used elsewhere
/// in the `instances/*` family.
async fn require_mirror(
    adapter: &dyn crate::adapters::Adapter,
    instance_id: &str,
) -> Result<std::sync::Arc<crate::adapters::InstanceMirror>, RpcError> {
    let key = InstanceKey::parse(instance_id).map_err(map_adapter_err)?;
    adapter
        .instance_mirror(key)
        .await
        .ok_or_else(|| RpcError::invalid_params(format!("instance '{instance_id}' not found in registry")))
}

#[cfg(test)]
mod snapshot_tests {
    use std::sync::{Arc, RwLock};

    use serde_json::json;

    use super::*;
    use crate::adapters::permission::{DefaultPermissionController, PermissionController};
    use crate::adapters::transcript::TranscriptItem;
    use crate::adapters::{AcpAdapter, Adapter, InstanceEvent, InstanceMirror, TerminalChunk, TerminalStream};
    use crate::config::Config;
    use crate::rpc::handler::HandlerCtx;
    use crate::rpc::status::StatusBroadcast;

    /// Spin a fresh `AcpAdapter` + dispatch one snapshot call against
    /// it. Returns the wire JSON (success) or a `{ code, message }`
    /// projection on error. Mirrors the helper in `permissions.rs`
    /// tests.
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
        match InstanceSnapshotHandler.handle(method, params, ctx).await {
            Ok(HandlerOutcome::Reply(v)) => v,
            Ok(HandlerOutcome::StatusSubscribed(v, _)) => v,
            Ok(HandlerOutcome::EventsSubscribed(v, _, _)) => v,
            Err(err) => json!({ "code": err.code, "message": err.message }),
        }
    }

    fn fresh_adapter() -> Arc<AcpAdapter> {
        let shared = Arc::new(RwLock::new(Config::default()));
        Arc::new(AcpAdapter::with_shared_config(
            shared,
            Arc::new(StatusBroadcast::new(true)),
            Arc::new(DefaultPermissionController::new()) as Arc<dyn PermissionController>,
        ))
    }

    fn transcript_event(text: &str) -> InstanceEvent {
        InstanceEvent::Transcript {
            agent_id: "claude-code".into(),
            instance_id: "i-1".into(),
            session_id: "s-1".into(),
            turn_id: None,
            item: TranscriptItem::AgentText { text: text.into() },
            // Placeholder; mirror.apply mints the real value.
            seq: 0,
            meta: None,
        }
    }

    /// `instance/snapshot/meta` happy path: seed an `InstanceMeta`
    /// event, expect the cwd / mode / model / mcpsCount fields to
    /// round-trip through the wire shape.
    #[tokio::test]
    async fn snapshot_meta_returns_seeded_state() {
        let adapter = fresh_adapter();
        let mirror = Arc::new(InstanceMirror::new());
        mirror
            .apply(&InstanceEvent::InstanceMeta {
                agent_id: "claude-code".into(),
                instance_id: "i-1".into(),
                profile_id: Some("strict".into()),
                session_id: Some("sess-meta".into()),
                cwd: "/tmp/proj".into(),
                current_mode_id: Some("plan".into()),
                current_model_id: Some("sonnet".into()),
                available_modes: Vec::new(),
                available_models: Vec::new(),
                mcps_count: 4,
            })
            .await;
        let key = adapter.test_install_mirror(mirror).await;

        let v = dispatch_with_adapter(
            adapter,
            "instance/snapshot/meta",
            json!({ "instanceId": key.as_string() }),
        )
        .await;
        assert_eq!(v["profileId"], "strict");
        assert_eq!(v["sessionId"], "sess-meta");
        assert_eq!(v["cwd"], "/tmp/proj");
        assert_eq!(v["currentModeId"], "plan");
        assert_eq!(v["currentModelId"], "sonnet");
        assert_eq!(v["mcpsCount"], 4);
        assert!(v["latestSeq"].is_null(), "no transcript yet");
    }

    /// `instance/snapshot/chat` paginates backwards through 200
    /// seeded items in 50-item windows. Pin every page boundary's
    /// `oldestSeq` / `latestSeq` / `hasMore`. Sanity-check `before`
    /// returns strictly-older items.
    #[tokio::test]
    async fn snapshot_chat_paginates_backwards() {
        let adapter = fresh_adapter();
        let mirror = Arc::new(InstanceMirror::with_cap(1_000));
        for i in 0..200 {
            mirror.apply(&transcript_event(&format!("msg-{i}"))).await;
        }
        let key = adapter.test_install_mirror(mirror).await;
        let id = key.as_string();

        // Page 1: latest 50 — seqs 150..=199.
        let p1 = dispatch_with_adapter(
            adapter.clone(),
            "instance/snapshot/chat",
            json!({ "instanceId": id, "limit": 50 }),
        )
        .await;
        assert_eq!(p1["items"].as_array().unwrap().len(), 50);
        assert_eq!(p1["oldestSeq"], 150);
        assert_eq!(p1["latestSeq"], 199);
        assert_eq!(p1["hasMore"], true);

        // Page 2: before=150 → seqs 100..=149.
        let p2 = dispatch_with_adapter(
            adapter.clone(),
            "instance/snapshot/chat",
            json!({ "instanceId": id, "before": 150, "limit": 50 }),
        )
        .await;
        assert_eq!(p2["items"].as_array().unwrap().len(), 50);
        assert_eq!(p2["oldestSeq"], 100);
        assert_eq!(p2["latestSeq"], 149);
        assert_eq!(p2["hasMore"], true);

        // Page 3: before=100 → seqs 50..=99.
        let p3 = dispatch_with_adapter(
            adapter.clone(),
            "instance/snapshot/chat",
            json!({ "instanceId": id, "before": 100, "limit": 50 }),
        )
        .await;
        assert_eq!(p3["items"].as_array().unwrap().len(), 50);
        assert_eq!(p3["oldestSeq"], 50);
        assert_eq!(p3["latestSeq"], 99);
        assert_eq!(p3["hasMore"], true);

        // Page 4: before=50 → seqs 0..=49 — last page.
        let p4 = dispatch_with_adapter(
            adapter,
            "instance/snapshot/chat",
            json!({ "instanceId": id, "before": 50, "limit": 50 }),
        )
        .await;
        assert_eq!(p4["items"].as_array().unwrap().len(), 50);
        assert_eq!(p4["oldestSeq"], 0);
        assert_eq!(p4["latestSeq"], 49);
        assert_eq!(p4["hasMore"], false, "exhausted the buffer");
    }

    /// `instance/snapshot/terminals` happy path: seed two terminal
    /// streams, expect both keyed entries with concatenated stdout
    /// and the running flag flipped on `Exit`.
    #[tokio::test]
    async fn snapshot_terminals_returns_seeded_state() {
        let adapter = fresh_adapter();
        let mirror = Arc::new(InstanceMirror::new());
        mirror
            .apply(&InstanceEvent::Terminal {
                agent_id: "claude-code".into(),
                instance_id: "i-1".into(),
                session_id: "s-1".into(),
                turn_id: None,
                terminal_id: "t-running".into(),
                chunk: TerminalChunk::Output {
                    stream: TerminalStream::Stdout,
                    data: "hello".into(),
                },
            })
            .await;
        mirror
            .apply(&InstanceEvent::Terminal {
                agent_id: "claude-code".into(),
                instance_id: "i-1".into(),
                session_id: "s-1".into(),
                turn_id: None,
                terminal_id: "t-done".into(),
                chunk: TerminalChunk::Output {
                    stream: TerminalStream::Stdout,
                    data: "ok\n".into(),
                },
            })
            .await;
        mirror
            .apply(&InstanceEvent::Terminal {
                agent_id: "claude-code".into(),
                instance_id: "i-1".into(),
                session_id: "s-1".into(),
                turn_id: None,
                terminal_id: "t-done".into(),
                chunk: TerminalChunk::Exit {
                    exit_code: Some(0),
                    signal: None,
                },
            })
            .await;
        let key = adapter.test_install_mirror(mirror).await;

        let v = dispatch_with_adapter(
            adapter,
            "instance/snapshot/terminals",
            json!({ "instanceId": key.as_string() }),
        )
        .await;
        let terms = v["terminals"].as_object().expect("terminals object");
        assert_eq!(terms.len(), 2);
        assert_eq!(terms["t-running"]["stdout"], "hello");
        assert_eq!(terms["t-running"]["running"], true);
        assert_eq!(terms["t-done"]["stdout"], "ok\n");
        assert_eq!(terms["t-done"]["running"], false);
        assert_eq!(terms["t-done"]["exitCode"], 0);
    }

    /// Missing `instanceId` field → `-32602 invalid_params` for each
    /// of the three verbs.
    #[tokio::test]
    async fn snapshot_missing_instance_id_is_invalid_params() {
        let adapter = fresh_adapter();
        for method in [
            "instance/snapshot/meta",
            "instance/snapshot/chat",
            "instance/snapshot/terminals",
        ] {
            let v = dispatch_with_adapter(adapter.clone(), method, Value::Null).await;
            assert_eq!(v["code"], -32602, "{method} with null params: {v}");

            let v = dispatch_with_adapter(adapter.clone(), method, json!({})).await;
            assert_eq!(v["code"], -32602, "{method} with empty params: {v}");
        }
    }

    /// Unknown `instanceId` → `-32602` from the `require_mirror`
    /// not-found path.
    #[tokio::test]
    async fn snapshot_unknown_instance_id_is_invalid_params() {
        let adapter = fresh_adapter();
        let ghost = "550e8400-e29b-41d4-a716-446655440000";
        for method in [
            "instance/snapshot/meta",
            "instance/snapshot/chat",
            "instance/snapshot/terminals",
        ] {
            let v = dispatch_with_adapter(adapter.clone(), method, json!({ "instanceId": ghost })).await;
            assert_eq!(v["code"], -32602, "{method} unknown id: {v}");
            assert!(
                v["message"].as_str().unwrap().contains("not found"),
                "{method} message: {v}"
            );
        }
    }

    /// Malformed UUID → `-32602` from the `InstanceKey::parse` path.
    #[tokio::test]
    async fn snapshot_malformed_instance_id_is_invalid_params() {
        let adapter = fresh_adapter();
        let v = dispatch_with_adapter(adapter, "instance/snapshot/meta", json!({ "instanceId": "not-a-uuid" })).await;
        assert_eq!(v["code"], -32602);
    }

    /// Unknown verb in the `instance/` namespace → `-32601`.
    #[tokio::test]
    async fn snapshot_unknown_verb_is_method_not_found() {
        let adapter = fresh_adapter();
        let v = dispatch_with_adapter(adapter, "instance/snapshot/bogus", json!({ "instanceId": "x" })).await;
        assert_eq!(v["code"], -32601);
    }
}

/// Setter-verb coverage on the `instances/*` namespace. Happy-path
/// dispatch requires a live actor (the inherent setters call
/// `require_instance`), so unit coverage here pins the wire-shape
/// validation lanes only: missing-required-field → `-32602`,
/// unknown-instance → `-32602` (via `AdapterError::InvalidRequest`),
/// unknown verb under the namespace → `-32601`. End-to-end happy
/// path is exercised by the manual smoke documented in the PR.
#[cfg(test)]
mod setters_tests {
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
            shared,
            Arc::new(StatusBroadcast::new(true)),
            Arc::new(DefaultPermissionController::new()) as Arc<dyn PermissionController>,
        ));
        let status = StatusBroadcast::new(true);
        let dyn_adapter: Arc<dyn Adapter> = adapter;
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
        match InstancesHandler.handle(method, params, ctx).await {
            Ok(HandlerOutcome::Reply(v)) => v,
            Ok(HandlerOutcome::StatusSubscribed(v, _)) => v,
            Ok(HandlerOutcome::EventsSubscribed(v, _, _)) => v,
            Err(err) => json!({ "code": err.code, "message": err.message }),
        }
    }

    /// `instances/focus { ensure: true }` against an unknown slug
    /// spawns + names the instance to that slug, then focuses it.
    /// Pins T2 from the audit: the existing dispatcher had no test
    /// for this branch, so a regression in spawn-and-rename ordering
    /// (a near miss in PR #43) wouldn't have surfaced.
    #[tokio::test]
    async fn focus_ensure_spawns_and_names_unknown_slug() {
        let cfg: Config = toml::from_str(
            r#"
[[agents]]
id = "dead"
provider = "acp-claude-code"
command = "/bin/false"

[profile]
default = "dead"

[[profiles]]
id = "dead"
agent = "dead"
"#,
        )
        .expect("config parses");
        let adapter = Arc::new(AcpAdapter::new(cfg, Arc::new(StatusBroadcast::new(true))));
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
        let outcome = InstancesHandler
            .handle(
                "instances/focus",
                json!({ "instanceId": "feat-test", "ensure": true }),
                ctx,
            )
            .await
            .expect("focus ensure ok");
        let v = match outcome {
            HandlerOutcome::Reply(v) => v,
            _ => panic!("expected Reply"),
        };
        let minted = v["instanceId"].as_str().expect("instanceId on reply").to_string();
        assert!(!minted.is_empty(), "ensure spawn must mint an instance id: {v}");

        // The spawned instance must be reachable by its newly-set
        // slug name, not just its mint UUID — proves the rename
        // happened after spawn.
        let by_name = adapter.resolve_token("feat-test").await;
        assert!(
            by_name.is_some(),
            "spawn-and-rename: 'feat-test' must resolve post-ensure"
        );
    }

    #[tokio::test]
    async fn set_mode_missing_instance_id_is_invalid_params() {
        let v = dispatch("instances/setMode", json!({ "modeId": "plan" })).await;
        assert_eq!(v["code"], -32602, "{v}");
    }

    #[tokio::test]
    async fn set_mode_missing_mode_id_is_invalid_params() {
        let v = dispatch(
            "instances/setMode",
            json!({ "instanceId": "550e8400-e29b-41d4-a716-446655440000" }),
        )
        .await;
        assert_eq!(v["code"], -32602, "{v}");
    }

    #[tokio::test]
    async fn set_mode_unknown_instance_is_invalid_params() {
        let ghost = "550e8400-e29b-41d4-a716-446655440000";
        let v = dispatch("instances/setMode", json!({ "instanceId": ghost, "modeId": "plan" })).await;
        assert_eq!(v["code"], -32602, "{v}");
        assert!(v["message"].as_str().unwrap_or_default().contains("not found"), "{v}");
    }

    #[tokio::test]
    async fn set_model_missing_model_id_is_invalid_params() {
        let v = dispatch(
            "instances/setModel",
            json!({ "instanceId": "550e8400-e29b-41d4-a716-446655440000" }),
        )
        .await;
        assert_eq!(v["code"], -32602, "{v}");
    }

    #[tokio::test]
    async fn set_model_unknown_instance_is_invalid_params() {
        let ghost = "550e8400-e29b-41d4-a716-446655440000";
        let v = dispatch(
            "instances/setModel",
            json!({ "instanceId": ghost, "modelId": "sonnet" }),
        )
        .await;
        assert_eq!(v["code"], -32602, "{v}");
    }

    #[tokio::test]
    async fn set_option_missing_value_is_invalid_params() {
        let v = dispatch(
            "instances/setOption",
            json!({
                "instanceId": "550e8400-e29b-41d4-a716-446655440000",
                "configId": "effort"
            }),
        )
        .await;
        assert_eq!(v["code"], -32602, "{v}");
    }

    #[tokio::test]
    async fn set_option_unknown_instance_is_invalid_params() {
        let ghost = "550e8400-e29b-41d4-a716-446655440000";
        let v = dispatch(
            "instances/setOption",
            json!({ "instanceId": ghost, "configId": "effort", "value": "high" }),
        )
        .await;
        assert_eq!(v["code"], -32602, "{v}");
    }

    #[tokio::test]
    async fn set_option_unknown_field_is_invalid_params() {
        let v = dispatch(
            "instances/setOption",
            json!({
                "instanceId": "550e8400-e29b-41d4-a716-446655440000",
                "configId": "effort",
                "value": "high",
                "stray": true,
            }),
        )
        .await;
        assert_eq!(v["code"], -32602, "{v}");
    }

    #[tokio::test]
    async fn instances_unknown_verb_is_method_not_found() {
        let v = dispatch("instances/setBogus", json!({})).await;
        assert_eq!(v["code"], -32601, "{v}");
    }

    #[tokio::test]
    async fn set_profile_missing_profile_id_is_invalid_params() {
        let v = dispatch(
            "instances/setProfile",
            json!({ "instanceId": "550e8400-e29b-41d4-a716-446655440000" }),
        )
        .await;
        assert_eq!(v["code"], -32602, "{v}");
    }

    #[tokio::test]
    async fn set_profile_missing_instance_id_is_invalid_params() {
        let v = dispatch("instances/setProfile", json!({ "profileId": "strict" })).await;
        assert_eq!(v["code"], -32602, "{v}");
    }

    #[tokio::test]
    async fn set_profile_unknown_instance_is_invalid_params() {
        let ghost = "550e8400-e29b-41d4-a716-446655440000";
        let v = dispatch(
            "instances/setProfile",
            json!({ "instanceId": ghost, "profileId": "strict" }),
        )
        .await;
        assert_eq!(v["code"], -32602, "{v}");
        assert!(v["message"].as_str().unwrap_or_default().contains("not found"), "{v}");
    }

    #[tokio::test]
    async fn set_profile_unknown_field_is_invalid_params() {
        let v = dispatch(
            "instances/setProfile",
            json!({
                "instanceId": "550e8400-e29b-41d4-a716-446655440000",
                "profileId": "strict",
                "stray": true,
            }),
        )
        .await;
        assert_eq!(v["code"], -32602, "{v}");
    }

    /// `instances/list` ships the per-instance `cwd` on every entry —
    /// `hyprpilot.nvim`'s palette filter (`item.cwd == vim.fn.getcwd()`)
    /// keys off it. Seed a stub instance via `test_install_mirror`
    /// (cwd defaults to the stub sentinel `/tmp/test-stub`) and assert
    /// the wire payload carries the field.
    #[tokio::test]
    async fn instances_list_ships_cwd_on_each_entry() {
        use crate::adapters::InstanceMirror;
        let shared = Arc::new(RwLock::new(Config::default()));
        let adapter = Arc::new(AcpAdapter::with_shared_config(
            shared,
            Arc::new(StatusBroadcast::new(true)),
            Arc::new(DefaultPermissionController::new()) as Arc<dyn PermissionController>,
        ));
        let _key = adapter.test_install_mirror(Arc::new(InstanceMirror::new())).await;
        let status = StatusBroadcast::new(true);
        let dyn_adapter: Arc<dyn Adapter> = adapter;
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
        let outcome = InstancesHandler
            .handle("instances/list", json!({}), ctx)
            .await
            .expect("list ok");
        let v = match outcome {
            HandlerOutcome::Reply(v) => v,
            _ => panic!("expected Reply"),
        };
        let entries = v["instances"].as_array().expect("instances array");
        assert_eq!(entries.len(), 1, "{v}");
        assert_eq!(entries[0]["cwd"], "/tmp/test-stub", "{v}");
    }

    /// `instances/info` carries the same typed `InstanceListEntry`
    /// projection as the list shape — including `cwd`.
    #[tokio::test]
    async fn instances_info_ships_cwd() {
        use crate::adapters::InstanceMirror;
        let shared = Arc::new(RwLock::new(Config::default()));
        let adapter = Arc::new(AcpAdapter::with_shared_config(
            shared,
            Arc::new(StatusBroadcast::new(true)),
            Arc::new(DefaultPermissionController::new()) as Arc<dyn PermissionController>,
        ));
        let key = adapter.test_install_mirror(Arc::new(InstanceMirror::new())).await;
        let status = StatusBroadcast::new(true);
        let dyn_adapter: Arc<dyn Adapter> = adapter;
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
        let outcome = InstancesHandler
            .handle("instances/info", json!({ "instanceId": key.as_string() }), ctx)
            .await
            .expect("info ok");
        let v = match outcome {
            HandlerOutcome::Reply(v) => v,
            _ => panic!("expected Reply"),
        };
        assert_eq!(v["cwd"], "/tmp/test-stub", "{v}");
        assert_eq!(v["instanceId"], key.as_string(), "{v}");
    }
}

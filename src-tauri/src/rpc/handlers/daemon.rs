use async_trait::async_trait;
use serde_json::{json, Value};
use tauri::Manager;

use crate::rpc::handler::{HandlerCtx, HandlerOutcome, RpcHandler};
use crate::rpc::protocol::RpcError;

/// `daemon/*` namespace (daemon lifecycle / introspection).
///
/// `daemon/kill` returns `{"killed": true}` and `daemon/shutdown`
/// returns `{"exiting": true}`; the server inspects either marker
/// after the response flush and runs `daemon::shutdown`.
///
/// `daemon/reload` rescans the on-disk skills catalogue and
/// publishes a `DaemonReloaded` event so subscribers (UI palette)
/// re-fetch their lists. Config + MCP catalogues stay static after
/// daemon start — restart-to-reconfigure for those.
pub struct DaemonHandler;

#[async_trait]
impl RpcHandler for DaemonHandler {
    fn namespace(&self) -> &'static str {
        "daemon"
    }

    async fn handle(&self, method: &str, params: Value, ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
        match method {
            "daemon/kill" => Ok(HandlerOutcome::Reply(json!({ "killed": true }))),
            "daemon/status" => status(&ctx).await,
            "daemon/version" => Ok(HandlerOutcome::Reply(version_payload())),
            "daemon/shutdown" => shutdown(&ctx, params).await,
            "daemon/reload" => reload(&ctx).await,
            "daemon/boot_snapshot" => boot_snapshot(&ctx).await,
            other => Err(RpcError::method_not_found(other)),
        }
    }
}

/// Aggregate boot-time payload — theme + keymaps + window state +
/// daemon cwd + completion config + agents + profiles + instances
/// in one round-trip. Mirrors the `tauri/boot_snapshot` proxy arm
/// (which the SPA hits via the webview bridge); promoting to a
/// public `daemon/*` verb lets second-frontends (nvim, remote ws,
/// etc.) hydrate on connect without round-tripping through
/// transport-coupled `tauri/<cmd>` namespacing.
async fn boot_snapshot(ctx: &HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
    let app = ctx
        .app
        .ok_or_else(|| RpcError::internal_error("daemon/boot_snapshot requires a tauri AppHandle"))?;
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
        .try_state::<std::sync::Arc<std::sync::RwLock<crate::config::Config>>>()
        .ok_or_else(|| RpcError::internal_error("config state not managed"))?;
    let adapter_state = app
        .try_state::<std::sync::Arc<crate::adapters::AcpAdapter>>()
        .ok_or_else(|| RpcError::internal_error("adapter state not managed"))?;

    let snap = crate::daemon::build_boot_snapshot(
        theme.inner(),
        keymaps.inner(),
        window_state.inner(),
        config_state.inner(),
        adapter_state.inner().as_ref(),
    )
    .await
    .map_err(RpcError::internal_error)?;

    let v =
        serde_json::to_value(snap).map_err(|e| RpcError::internal_error(format!("serialize boot snapshot: {e}")))?;
    Ok(HandlerOutcome::Reply(v))
}

async fn reload(ctx: &HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
    let mcps = ctx
        .mcps
        .as_ref()
        .ok_or_else(|| RpcError::internal_error("mcps registry unavailable"))?;

    // Per-instance skills: fan out the reload across every live
    // instance so the captain's "refresh skills" hits every active
    // session at once, not just the focused one.
    let skills_count = ctx.adapter.reload_all_skills().await;

    let profiles = ctx
        .config
        .as_ref()
        .map(|c| c.read().expect("config lock poisoned").profiles.len())
        .unwrap_or(0);
    let mcps_count = mcps.list().len();

    ctx.adapter.publish_daemon_reloaded(profiles, skills_count, mcps_count);

    Ok(HandlerOutcome::Reply(json!({
        "profiles": profiles,
        "skillsCount": skills_count,
        "mcpsCount": mcps_count,
    })))
}

async fn status(ctx: &HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
    let pid = std::process::id();
    let uptime_secs = ctx.started_at.map(|t| t.elapsed().as_secs()).unwrap_or(0);
    let socket_path = ctx.socket_path.map(|p| p.display().to_string()).unwrap_or_default();
    let instance_count = ctx.adapter.list().await.len();
    Ok(HandlerOutcome::Reply(json!({
        "pid": pid,
        "uptimeSecs": uptime_secs,
        "socketPath": socket_path,
        "version": env!("CARGO_PKG_VERSION"),
        "instanceCount": instance_count,
    })))
}

pub(crate) fn version_payload() -> Value {
    let mut out = json!({ "version": env!("CARGO_PKG_VERSION") });
    if let Some(c) = option_env!("HYPRPILOT_BUILD_COMMIT") {
        out["commit"] = Value::String(c.to_string());
    }
    if let Some(d) = option_env!("HYPRPILOT_BUILD_DATE") {
        out["buildDate"] = Value::String(d.to_string());
    }
    out
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct ShutdownParams {
    force: bool,
}

async fn shutdown(ctx: &HandlerCtx<'_>, params: Value) -> Result<HandlerOutcome, RpcError> {
    let ShutdownParams { force } = crate::rpc::handlers::util::params_or_default(params, "daemon/shutdown")?;
    let adapter = &ctx.adapter;

    if !force {
        let busy = adapter.busy_instance_ids().await;
        if !busy.is_empty() {
            return Err(RpcError {
                code: -32603,
                message: format!("turns in flight: {} busy instance(s)", busy.len()),
                data: Some(json!({
                    "error": "turns in flight",
                    "counts": {
                        "instances": adapter.list().await.len(),
                        "busyInstances": busy.len(),
                    },
                    "busyInstanceIds": busy,
                })),
            });
        }
    }

    Ok(HandlerOutcome::Reply(json!({ "exiting": true })))
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use serde_json::json;

    use super::*;
    use crate::adapters::{AcpAdapter, Adapter};
    use crate::config::Config;
    use crate::rpc::handler::HandlerCtx;

    use crate::rpc::status::StatusBroadcast;

    async fn run_handler(
        method: &str,
        params: Value,
        started_at: Option<Instant>,
        socket_path: Option<&Path>,
        acp: Arc<AcpAdapter>,
    ) -> Result<HandlerOutcome, RpcError> {
        let status = StatusBroadcast::new(true);
        let config = Arc::new(std::sync::RwLock::new(Config::default()));
        let adapter: Arc<dyn Adapter> = acp.clone();
        let ctx = HandlerCtx {
            app: None,
            status: &status,
            adapter,
            config: Some(config),
            mcps: None,
            already_subscribed: false,
            already_events_subscribed: false,
            started_at,
            socket_path,
        };
        DaemonHandler.handle(method, params, ctx).await
    }

    #[tokio::test]
    async fn status_reports_pid_and_uptime() {
        let acp = Arc::new(AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true))));
        let started_at = Instant::now() - Duration::from_secs(2);
        let socket = Path::new("/tmp/hyprpilot.sock");
        let out = run_handler("daemon/status", Value::Null, Some(started_at), Some(socket), acp)
            .await
            .unwrap();
        let v = match out {
            HandlerOutcome::Reply(v) => v,
            HandlerOutcome::StatusSubscribed(..) | HandlerOutcome::EventsSubscribed(..) => {
                panic!("expected Reply")
            }
        };
        assert_eq!(v["pid"], std::process::id());
        assert!(
            v["uptimeSecs"].as_u64().unwrap() >= 2,
            "uptime must be >=2 after sleeping 2s: {v}",
        );
        assert_eq!(v["socketPath"], "/tmp/hyprpilot.sock");
        assert_eq!(v["instanceCount"], 0);
        assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
    }

    #[tokio::test]
    async fn version_reports_pkg_version() {
        let acp = Arc::new(AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true))));
        let out = run_handler("daemon/version", Value::Null, None, None, acp)
            .await
            .unwrap();
        let v = match out {
            HandlerOutcome::Reply(v) => v,
            HandlerOutcome::StatusSubscribed(..) | HandlerOutcome::EventsSubscribed(..) => {
                panic!("expected Reply")
            }
        };
        assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
    }

    #[tokio::test]
    async fn shutdown_without_force_when_idle_returns_exiting_marker() {
        let acp = Arc::new(AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true))));
        let out = run_handler("daemon/shutdown", Value::Null, None, None, acp)
            .await
            .unwrap();
        let v = match out {
            HandlerOutcome::Reply(v) => v,
            HandlerOutcome::StatusSubscribed(..) | HandlerOutcome::EventsSubscribed(..) => {
                panic!("expected Reply")
            }
        };
        assert_eq!(v["exiting"], true);
    }

    #[tokio::test]
    async fn shutdown_when_busy_without_force_is_internal_error() {
        let acp = Arc::new(AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true))));
        acp.test_mark_busy("550e8400-e29b-41d4-a716-446655440000".into());
        let res = run_handler("daemon/shutdown", Value::Null, None, None, acp.clone()).await;
        let err = match res {
            Err(e) => e,
            Ok(_) => panic!("must reject when busy"),
        };
        assert_eq!(err.code, -32603);
        assert!(err.message.contains("turns in flight"), "{}", err.message);
    }

    #[tokio::test]
    async fn shutdown_with_force_when_busy_returns_exiting_marker() {
        let acp = Arc::new(AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true))));
        acp.test_mark_busy("550e8400-e29b-41d4-a716-446655440000".into());
        let out = run_handler("daemon/shutdown", json!({ "force": true }), None, None, acp)
            .await
            .unwrap();
        let v = match out {
            HandlerOutcome::Reply(v) => v,
            HandlerOutcome::StatusSubscribed(..) | HandlerOutcome::EventsSubscribed(..) => {
                panic!("expected Reply")
            }
        };
        assert_eq!(v["exiting"], true);
    }

    #[tokio::test]
    async fn unknown_method_in_namespace_is_method_not_found() {
        let acp = Arc::new(AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true))));
        let res = run_handler("daemon/bogus", Value::Null, None, None, acp).await;
        let err = match res {
            Err(e) => e,
            Ok(_) => panic!("must be method_not_found"),
        };
        assert_eq!(err.code, -32601);
    }
}

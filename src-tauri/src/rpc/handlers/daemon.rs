use async_trait::async_trait;
use serde_json::{json, Value};

use crate::rpc::handler::{HandlerCtx, HandlerOutcome, RpcHandler};
use crate::rpc::protocol::RpcError;

/// `daemon/*` namespace (daemon lifecycle / introspection).
///
/// `daemon/kill` returns `{"killed": true}` and `daemon/shutdown`
/// returns `{"exiting": true}`; the server inspects either marker
/// after the response flush and runs `daemon::shutdown`.
///
/// Live config reload is intentionally not exposed — MCPs, skills,
/// and config are all restart-to-reconfigure. Edit `~/.config/hyprpilot/config.toml`,
/// restart the daemon to pick up changes.
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
            other => Err(RpcError::method_not_found(other)),
        }
    }
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
            already_subscribed: false,
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
            HandlerOutcome::StatusSubscribed(..) => panic!("expected Reply"),
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
            HandlerOutcome::StatusSubscribed(..) => panic!("expected Reply"),
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
            HandlerOutcome::StatusSubscribed(..) => panic!("expected Reply"),
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
            HandlerOutcome::StatusSubscribed(..) => panic!("expected Reply"),
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

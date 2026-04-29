use async_trait::async_trait;
use serde_json::{json, Value};

use crate::adapters::Adapter;
use crate::rpc::handler::{HandlerCtx, HandlerOutcome, RpcHandler};
use crate::rpc::protocol::RpcError;

/// `daemon/*` namespace (daemon lifecycle / introspection).
///
/// `daemon/kill` returns `{"killed": true}` and `daemon/shutdown`
/// returns `{"exiting": true}`; the server inspects either marker
/// after the response flush and runs `daemon::shutdown`.
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
            "daemon/reload" => reload(&ctx).await,
            "daemon/shutdown" => shutdown(&ctx, params).await,
            other => Err(RpcError::method_not_found(other)),
        }
    }
}

async fn status(ctx: &HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
    let pid = std::process::id();
    let uptime_secs = ctx.started_at.map(|t| t.elapsed().as_secs()).unwrap_or(0);
    let socket_path = ctx.socket_path.map(|p| p.display().to_string()).unwrap_or_default();
    let instance_count = match &ctx.adapter {
        Some(a) => a.list().await.len(),
        None => 0,
    };
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

async fn reload(ctx: &HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
    let load_ctx = ctx.config_load_context.ok_or_else(|| {
        RpcError::internal_error("config load context missing — daemon/reload only works in production")
    })?;
    let config_handle = ctx
        .config
        .as_ref()
        .ok_or_else(|| RpcError::internal_error("shared config missing"))?
        .clone();
    let skills = ctx
        .skills
        .as_ref()
        .ok_or_else(|| RpcError::internal_error("skills registry missing"))?
        .clone();
    let mcps = ctx
        .mcps
        .as_ref()
        .ok_or_else(|| RpcError::internal_error("mcps registry missing"))?
        .clone();
    let acp = ctx
        .acp_adapter
        .as_ref()
        .ok_or_else(|| RpcError::internal_error("ACP adapter missing"))?
        .clone();

    // Re-run the loader with the same overlay layers the daemon
    // booted under. Validation runs after merge — failures bubble out
    // as -32603 and the daemon stays on the old config.
    let new_cfg = crate::config::load(load_ctx.cli_path.as_deref(), load_ctx.profile.as_deref())
        .map_err(|err| RpcError::internal_error(format!("config reload failed: {err:#}")))?;
    new_cfg
        .validate()
        .map_err(|err| RpcError::internal_error(format!("config validation failed after reload: {err:#}")))?;

    let profiles = new_cfg.profiles.len();
    {
        let mut cfg = config_handle.write().expect("config lock poisoned");
        *cfg = new_cfg;
    }

    if let Err(err) = skills.reload() {
        tracing::warn!(%err, "daemon/reload: skills reload failed — keeping prior set");
    }
    let skills_count = skills.list().len();

    // MCPs are restart-to-reconfigure — the catalog stays at its boot
    // snapshot. The new `[[mcps]]` from the just-loaded config file
    // only applies on next daemon start. `mcpsCount` reflects the
    // current (boot-time) registry, which is the source of truth for
    // every running instance.
    let mcps_count = mcps.list().len();

    acp.publish_daemon_reloaded(profiles, skills_count, mcps_count);

    Ok(HandlerOutcome::Reply(json!({
        "profiles": profiles,
        "skillsCount": skills_count,
        "mcpsCount": mcps_count,
    })))
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct ShutdownParams {
    force: bool,
}

async fn shutdown(ctx: &HandlerCtx<'_>, params: Value) -> Result<HandlerOutcome, RpcError> {
    let ShutdownParams { force } = crate::rpc::handlers::util::params_or_default(params, "daemon/shutdown")?;
    let acp = ctx
        .acp_adapter
        .as_ref()
        .ok_or_else(|| RpcError::internal_error("ACP adapter missing"))?
        .clone();

    if !force {
        let busy = acp.busy_instance_ids().await;
        if !busy.is_empty() {
            return Err(RpcError {
                code: -32603,
                message: format!("turns in flight: {} busy instance(s)", busy.len()),
                data: Some(json!({
                    "error": "turns in flight",
                    "counts": {
                        "instances": acp.list().await.len(),
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
    use crate::mcp::MCPsRegistry;
    use crate::rpc::handler::{ConfigLoadContext, HandlerCtx};
    use crate::rpc::protocol::RequestId;
    use crate::rpc::status::StatusBroadcast;
    use crate::skills::{SkillsBroadcast, SkillsRegistry};

    fn build_skills() -> Arc<SkillsRegistry> {
        Arc::new(SkillsRegistry::new(Vec::new(), Arc::new(SkillsBroadcast::new())))
    }

    fn build_mcps() -> Arc<MCPsRegistry> {
        Arc::new(MCPsRegistry::new(Vec::new()))
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_handler<'a>(
        method: &str,
        params: Value,
        started_at: Option<Instant>,
        socket_path: Option<&'a Path>,
        load_ctx: Option<&'a ConfigLoadContext>,
        skills: Option<Arc<SkillsRegistry>>,
        mcps: Option<Arc<MCPsRegistry>>,
        acp: Arc<AcpAdapter>,
    ) -> Result<HandlerOutcome, RpcError> {
        let status = StatusBroadcast::new(true);
        let id = RequestId::Number(1);
        let config = Arc::new(std::sync::RwLock::new(Config::default()));
        let adapter: Arc<dyn Adapter> = acp.clone();
        let ctx = HandlerCtx {
            app: None,
            status: &status,
            adapter: Some(adapter),
            acp_adapter: Some(acp),
            config: Some(config),
            id: &id,
            already_subscribed: false,
            started_at,
            socket_path,
            config_load_context: load_ctx,
            skills,
            mcps,
            existing_event_subscription_ids: &[],
            events_tx: None,
        };
        DaemonHandler.handle(method, params, ctx).await
    }

    #[tokio::test]
    async fn status_reports_pid_and_uptime() {
        let acp = Arc::new(AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true))));
        let started_at = Instant::now() - Duration::from_secs(2);
        let socket = Path::new("/tmp/hyprpilot.sock");
        let out = run_handler(
            "daemon/status",
            Value::Null,
            Some(started_at),
            Some(socket),
            None,
            None,
            None,
            acp,
        )
        .await
        .unwrap();
        let v = match out {
            HandlerOutcome::Reply(v) => v,
            HandlerOutcome::StatusSubscribed(..) | HandlerOutcome::EventsSubscribed(..) => panic!("expected Reply"),
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
        let out = run_handler("daemon/version", Value::Null, None, None, None, None, None, acp)
            .await
            .unwrap();
        let v = match out {
            HandlerOutcome::Reply(v) => v,
            HandlerOutcome::StatusSubscribed(..) | HandlerOutcome::EventsSubscribed(..) => panic!("expected Reply"),
        };
        assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
    }

    #[tokio::test]
    async fn reload_returns_post_reload_counts() {
        // No CLI path / profile → the loader walks defaults +
        // (possibly) the user XDG config. The defaults file ships zero
        // profiles, so the count is 0 unless the dev env has a real
        // config.toml — assert ≥0 only.
        let acp = Arc::new(AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true))));
        let load_ctx = ConfigLoadContext::default();
        // Use a temp config path so we don't touch the user's XDG setup.
        let tmp = tempfile::TempDir::new().unwrap();
        let cfg_path = tmp.path().join("config.toml");
        std::fs::write(&cfg_path, "").unwrap();
        let load_ctx = ConfigLoadContext {
            cli_path: Some(cfg_path),
            profile: load_ctx.profile,
        };
        let skills = build_skills();
        let mcps = build_mcps();
        let out = run_handler(
            "daemon/reload",
            Value::Null,
            None,
            None,
            Some(&load_ctx),
            Some(skills),
            Some(mcps),
            acp,
        )
        .await
        .unwrap();
        let v = match out {
            HandlerOutcome::Reply(v) => v,
            HandlerOutcome::StatusSubscribed(..) | HandlerOutcome::EventsSubscribed(..) => panic!("expected Reply"),
        };
        assert!(v.get("profiles").is_some(), "{v}");
        assert!(v.get("skillsCount").is_some(), "{v}");
        assert_eq!(v["mcpsCount"], 0);
    }

    #[tokio::test]
    async fn reload_emits_daemon_reloaded_event() {
        use crate::adapters::InstanceEvent;

        let acp = Arc::new(AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true))));
        let mut rx = acp.subscribe_events();

        let tmp = tempfile::TempDir::new().unwrap();
        let cfg_path = tmp.path().join("config.toml");
        std::fs::write(&cfg_path, "").unwrap();
        let load_ctx = ConfigLoadContext {
            cli_path: Some(cfg_path),
            profile: None,
        };
        let skills = build_skills();
        let mcps = build_mcps();
        let _out = run_handler(
            "daemon/reload",
            Value::Null,
            None,
            None,
            Some(&load_ctx),
            Some(skills),
            Some(mcps),
            acp,
        )
        .await
        .unwrap();

        let evt = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("event must fire within 1s")
            .expect("recv ok");
        assert!(
            matches!(evt, InstanceEvent::DaemonReloaded { .. }),
            "expected DaemonReloaded, got {evt:?}",
        );
    }

    #[tokio::test]
    async fn shutdown_without_force_when_idle_returns_exiting_marker() {
        let acp = Arc::new(AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true))));
        let out = run_handler("daemon/shutdown", Value::Null, None, None, None, None, None, acp)
            .await
            .unwrap();
        let v = match out {
            HandlerOutcome::Reply(v) => v,
            HandlerOutcome::StatusSubscribed(..) | HandlerOutcome::EventsSubscribed(..) => panic!("expected Reply"),
        };
        assert_eq!(v["exiting"], true);
    }

    #[tokio::test]
    async fn shutdown_when_busy_without_force_is_internal_error() {
        let acp = Arc::new(AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true))));
        // Pretend a turn is in flight by directly populating the busy
        // tracker through the test hook.
        acp.test_mark_busy("550e8400-e29b-41d4-a716-446655440000".into());
        let res = run_handler(
            "daemon/shutdown",
            Value::Null,
            None,
            None,
            None,
            None,
            None,
            acp.clone(),
        )
        .await;
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
        let out = run_handler(
            "daemon/shutdown",
            json!({ "force": true }),
            None,
            None,
            None,
            None,
            None,
            acp,
        )
        .await
        .unwrap();
        let v = match out {
            HandlerOutcome::Reply(v) => v,
            HandlerOutcome::StatusSubscribed(..) | HandlerOutcome::EventsSubscribed(..) => panic!("expected Reply"),
        };
        assert_eq!(v["exiting"], true);
    }

    #[tokio::test]
    async fn unknown_method_in_namespace_is_method_not_found() {
        let acp = Arc::new(AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true))));
        let res = run_handler("daemon/bogus", Value::Null, None, None, None, None, None, acp).await;
        let err = match res {
            Err(e) => e,
            Ok(_) => panic!("must be method_not_found"),
        };
        assert_eq!(err.code, -32601);
    }
}

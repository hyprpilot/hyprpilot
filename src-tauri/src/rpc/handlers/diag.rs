use async_trait::async_trait;
use serde_json::{json, Value};

use crate::rpc::handler::{HandlerCtx, HandlerOutcome, RpcHandler};
use crate::rpc::protocol::RpcError;

/// `diag/*` namespace — read-only operator diagnostics. Sole verb
/// today is `diag/snapshot`, a single-shot dump for "what is this
/// daemon currently doing" support tickets.
///
/// Redaction policy is load-bearing: profile env values (`agents.env`)
/// and live transcript bodies must not appear on the wire. Only
/// structural counts + ids land in the snapshot.
pub struct DiagHandler;

#[async_trait]
impl RpcHandler for DiagHandler {
    fn namespace(&self) -> &'static str {
        "diag"
    }

    async fn handle(&self, method: &str, _params: Value, ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
        match method {
            "diag/snapshot" => snapshot(&ctx).await,
            other => Err(RpcError::method_not_found(other)),
        }
    }
}

async fn snapshot(ctx: &HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
    let pid = std::process::id();
    let uptime_secs = ctx.started_at.map(|t| t.elapsed().as_secs()).unwrap_or(0);
    let socket_path = ctx.socket_path.map(|p| p.display().to_string()).unwrap_or_default();

    let instances = ctx
        .adapter
        .list()
        .await
        .into_iter()
        .map(|info| {
            json!({
                "instanceId": info.id,
                "agentId": info.agent_id,
                "profileId": info.profile_id,
                "sessionId": info.session_id,
                "mode": info.mode,
            })
        })
        .collect::<Vec<_>>();

    // Profiles: only structural fields (id, agent, has_system_prompt).
    // env values stay behind on the agent side — the snapshot must
    // never leak them.
    let (profile_summaries, profiles_count) = match &ctx.config {
        Some(handle) => {
            let cfg = handle.read().expect("config lock poisoned");
            let summaries: Vec<Value> = cfg
                .profiles
                .iter()
                .map(|p| {
                    json!({
                        "id": p.id,
                        "agent": p.agent,
                        "hasSystemPrompt": p.system_prompt.is_some(),
                    })
                })
                .collect();
            let count = cfg.profiles.len();
            (summaries, count)
        }
        None => (Vec::new(), 0),
    };

    Ok(HandlerOutcome::Reply(json!({
        "daemon": {
            "pid": pid,
            "uptimeSecs": uptime_secs,
            "version": env!("CARGO_PKG_VERSION"),
            "socketPath": socket_path,
        },
        "instances": instances,
        "profiles": {
            "count": profiles_count,
            "summaries": profile_summaries,
        },
    })))
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Instant;

    use serde_json::Value;

    use super::*;
    use crate::adapters::{AcpAdapter, Adapter};
    use crate::config::Config;
    use crate::rpc::handler::HandlerCtx;

    use crate::rpc::status::StatusBroadcast;

    /// `diag/snapshot` against an empty config + empty registry —
    /// asserts the redaction shape: `profiles.summaries` must omit
    /// `env` entirely, even when an agent / profile carries env
    /// overrides.
    #[tokio::test]
    async fn snapshot_redacts_profile_env_values() {
        let mut cfg = Config::default();
        let mut env = std::collections::BTreeMap::new();
        env.insert(
            "ANTHROPIC_API_KEY".to_string(),
            "sk-secret-token-DO-NOT-LEAK".to_string(),
        );
        cfg.agents.agents.push(crate::config::AgentConfig {
            id: "claude-code".into(),
            provider: crate::config::AgentProvider::AcpClaudeCode,
            model: None,
            command: "bunx".into(),
            args: Vec::new(),
            env,
            cwd: None,
        });
        cfg.profiles.push(crate::config::ProfileConfig {
            id: "ask".into(),
            agent: "claude-code".into(),
            model: None,
            system_prompt: Some(vec![std::path::PathBuf::from("/tmp/hyprpilot-test-prompt.md")]),
            mcps: None,
            skills: None,
            mode: None,
            cwd: None,
            env: std::collections::BTreeMap::new(),
        });

        let acp = Arc::new(AcpAdapter::new(cfg.clone(), Arc::new(StatusBroadcast::new(true))));
        let adapter: Arc<dyn Adapter> = acp.clone();
        let config = Arc::new(std::sync::RwLock::new(cfg));
        let status = StatusBroadcast::new(true);
        let socket = Path::new("/tmp/hyprpilot.sock");
        let started_at = Instant::now();
        let ctx = HandlerCtx {
            app: None,
            status: &status,
            adapter,
            config: Some(config),
            skills: None,
            mcps: None,
            already_subscribed: false,
            started_at: Some(started_at),
            socket_path: Some(socket),
        };
        let out = DiagHandler.handle("diag/snapshot", Value::Null, ctx).await.unwrap();
        let v = match out {
            HandlerOutcome::Reply(v) => v,
            HandlerOutcome::StatusSubscribed(..) => panic!("expected Reply"),
        };

        // Every secret must stay buried.
        let serialized = serde_json::to_string(&v).unwrap();
        assert!(
            !serialized.contains("ANTHROPIC_API_KEY"),
            "snapshot must not leak env keys: {serialized}",
        );
        assert!(
            !serialized.contains("sk-secret-token-DO-NOT-LEAK"),
            "snapshot must not leak env values: {serialized}",
        );

        // Daemon block carries pid / uptime / version / socket.
        assert_eq!(v["daemon"]["pid"], std::process::id());
        assert_eq!(v["daemon"]["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(v["daemon"]["socketPath"], "/tmp/hyprpilot.sock");

        // Profiles report the structural shape.
        assert_eq!(v["profiles"]["count"], 1);
        let summaries = v["profiles"]["summaries"].as_array().unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0]["id"], "ask");
        assert_eq!(summaries[0]["agent"], "claude-code");
        assert_eq!(summaries[0]["hasSystemPrompt"], true);
        assert!(summaries[0].get("env").is_none(), "env must not appear: {v}");

        assert_eq!(v["instances"], json!([]));
    }

    #[tokio::test]
    async fn snapshot_unknown_verb_is_method_not_found() {
        let acp = Arc::new(AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true))));
        let adapter: Arc<dyn Adapter> = acp.clone();
        let config = Arc::new(std::sync::RwLock::new(Config::default()));
        let status = StatusBroadcast::new(true);
        let ctx = HandlerCtx {
            app: None,
            status: &status,
            adapter,
            config: Some(config),
            skills: None,
            mcps: None,
            already_subscribed: false,
            started_at: None,
            socket_path: None,
        };
        let res = DiagHandler.handle("diag/bogus", Value::Null, ctx).await;
        let err = match res {
            Err(e) => e,
            Ok(_) => panic!("must be method_not_found"),
        };
        assert_eq!(err.code, -32601);
    }
}

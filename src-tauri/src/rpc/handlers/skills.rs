use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::rpc::handler::{HandlerCtx, HandlerOutcome, RpcHandler};
use crate::rpc::handlers::util::{params_or_default, parse_params};
use crate::rpc::protocol::RpcError;
use crate::skills::{SkillSlug, SkillSummary, SkillsRegistry};

/// `skills/*` namespace — exposes the `SkillsRegistry` to the
/// socket. Three verbs today: `skills/list`, `skills/get`,
/// `skills/reload`.
pub struct SkillsHandler {
    registry: Arc<SkillsRegistry>,
}

impl SkillsHandler {
    #[must_use]
    pub fn new(registry: Arc<SkillsRegistry>) -> Self {
        Self { registry }
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct ListParams {
    instance_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GetParams {
    slug: String,
}

#[async_trait]
impl RpcHandler for SkillsHandler {
    fn namespace(&self) -> &'static str {
        "skills"
    }

    async fn handle(&self, method: &str, params: Value, _ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
        match method {
            "skills/list" => {
                let ListParams { instance_id } = params_or_default::<ListParams>(params, method)?;
                if instance_id.is_some() {
                    // K-275 lands the `profile.skills` allowlist that this
                    // filter keys off; return a structured error rather
                    // than panicking the connection task.
                    return Err(RpcError::invalid_params(
                        "skills/list instance_id filter not yet implemented (K-275)",
                    ));
                }
                let list: Vec<SkillSummary> = self.registry.list().iter().map(SkillSummary::from).collect();
                Ok(HandlerOutcome::Reply(json!({ "skills": list })))
            }
            "skills/get" => {
                let GetParams { slug } = parse_params(params, method)?;
                let parsed = SkillSlug::parse(&slug)
                    .map_err(|e| RpcError::invalid_params(format!("skills/get slug '{slug}': {e}")))?;
                let Some(skill) = self.registry.get(&parsed) else {
                    return Err(RpcError::invalid_params(format!("unknown skill '{slug}'")));
                };
                Ok(HandlerOutcome::Reply(json!({
                    "slug": skill.slug,
                    "title": skill.title,
                    "description": skill.description,
                    "body": skill.body,
                    "path": skill.path.display().to_string(),
                    "references": skill.references,
                })))
            }
            "skills/reload" => {
                self.registry
                    .reload()
                    .map_err(|e| RpcError::internal_error(format!("skills reload failed: {e}")))?;
                let count = self.registry.list().len();
                Ok(HandlerOutcome::Reply(json!({ "reloaded": count })))
            }
            other => Err(RpcError::method_not_found(other)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use tempfile::TempDir;
    use tokio::sync::broadcast;

    use super::*;
    use crate::adapters::{AcpAdapter, Adapter};
    use crate::config::Config;
    use crate::rpc::protocol::RequestId;
    use crate::rpc::status::StatusBroadcast;
    use crate::skills::{SkillsBroadcast, SkillsChanged};

    fn seed(dir: &std::path::Path, slug: &str) {
        let d = dir.join(slug);
        fs::create_dir_all(&d).unwrap();
        fs::write(
            d.join("SKILL.md"),
            format!("---\ndescription: {slug} desc\n---\n\n# {slug}\n\n{slug} body\n"),
        )
        .unwrap();
    }

    fn build_handler(tmp: &TempDir) -> SkillsHandler {
        let (tx, _rx) = broadcast::channel::<SkillsChanged>(8);
        let broadcast = Arc::new(SkillsBroadcast::from_sender(tx));
        let reg = Arc::new(SkillsRegistry::new(vec![tmp.path().to_path_buf()], broadcast));
        reg.reload().unwrap();
        SkillsHandler::new(reg)
    }

    fn ctx<'a>(
        status: &'a StatusBroadcast,
        id: &'a RequestId,
        config: Arc<std::sync::RwLock<Config>>,
        adapter: Arc<dyn Adapter>,
        acp_adapter: Arc<AcpAdapter>,
    ) -> HandlerCtx<'a> {
        HandlerCtx {
            app: None,
            status,
            adapter: Some(adapter),
            acp_adapter: Some(acp_adapter),
            config: Some(config),
            id,
            already_subscribed: false,
            started_at: None,
            socket_path: None,
            config_load_context: None,
            skills: None,
            mcps: None,
            existing_event_subscription_ids: &[],
            events_tx: None,
        }
    }

    async fn run_handler(handler: &SkillsHandler, method: &str, params: Value) -> Result<HandlerOutcome, RpcError> {
        let status = StatusBroadcast::new(true);
        let id = RequestId::Number(1);
        let config = Arc::new(std::sync::RwLock::new(Config::default()));
        let acp = Arc::new(AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true))));
        let adapter: Arc<dyn Adapter> = acp.clone();
        let ctx = ctx(&status, &id, config, adapter, acp);
        handler.handle(method, params, ctx).await
    }

    #[tokio::test]
    async fn list_returns_summaries_without_body() {
        let tmp = TempDir::new().unwrap();
        seed(tmp.path(), "alpha");
        seed(tmp.path(), "beta");
        let handler = build_handler(&tmp);
        let out = run_handler(&handler, "skills/list", Value::Null).await.unwrap();
        let v = match out {
            HandlerOutcome::Reply(v) => v,
            _ => panic!("expected Reply"),
        };
        assert_eq!(v["skills"].as_array().unwrap().len(), 2);
        assert!(v["skills"][0].get("body").is_none());
        assert!(v["skills"][0].get("references").is_none());
        assert_eq!(v["skills"][0]["slug"], "alpha");
    }

    #[tokio::test]
    async fn get_returns_full_body_for_known_slug() {
        let tmp = TempDir::new().unwrap();
        seed(tmp.path(), "coder");
        let handler = build_handler(&tmp);
        let out = run_handler(&handler, "skills/get", json!({ "slug": "coder" }))
            .await
            .unwrap();
        let v = match out {
            HandlerOutcome::Reply(v) => v,
            _ => panic!("expected Reply"),
        };
        assert_eq!(v["slug"], "coder");
        assert!(v["body"].as_str().unwrap().contains("coder body"));
        assert!(v["references"].is_array());
        let path = v["path"].as_str().unwrap();
        assert!(
            path.contains("coder") && path.ends_with("SKILL.md"),
            "path should point at the SKILL.md file: {path}",
        );
    }

    #[tokio::test]
    async fn get_unknown_slug_returns_invalid_params() {
        let tmp = TempDir::new().unwrap();
        seed(tmp.path(), "known");
        let handler = build_handler(&tmp);
        let res = run_handler(&handler, "skills/get", json!({ "slug": "missing" })).await;
        match res {
            Err(err) => assert_eq!(err.code, -32602),
            Ok(_) => panic!("must reject unknown slug"),
        }
    }

    #[tokio::test]
    async fn get_malformed_slug_returns_invalid_params() {
        let tmp = TempDir::new().unwrap();
        seed(tmp.path(), "ok");
        let handler = build_handler(&tmp);
        let res = run_handler(&handler, "skills/get", json!({ "slug": "has space" })).await;
        match res {
            Err(err) => assert_eq!(err.code, -32602),
            Ok(_) => panic!("must reject malformed slug"),
        }
    }

    #[tokio::test]
    async fn reload_reports_count() {
        let tmp = TempDir::new().unwrap();
        seed(tmp.path(), "one");
        let handler = build_handler(&tmp);
        let out = run_handler(&handler, "skills/reload", Value::Null).await.unwrap();
        let v = match out {
            HandlerOutcome::Reply(v) => v,
            _ => panic!("expected Reply"),
        };
        assert_eq!(v["reloaded"], 1);
    }

    #[tokio::test]
    async fn unknown_verb_is_method_not_found() {
        let tmp = TempDir::new().unwrap();
        let handler = build_handler(&tmp);
        let res = run_handler(&handler, "skills/bogus", Value::Null).await;
        match res {
            Err(err) => assert_eq!(err.code, -32601),
            Ok(_) => panic!("unknown verb must return method_not_found"),
        }
    }

    #[test]
    fn skill_summary_serialization_has_no_body_keys() {
        // Regression: the listing shape must never carry `body` /
        // `frontmatter` / `references`.
        let s = SkillSummary {
            slug: SkillSlug::parse("ok").unwrap(),
            title: "T".into(),
            description: "D".into(),
        };
        let v = serde_json::to_value(&s).unwrap();
        assert!(v.get("body").is_none());
        assert!(v.get("frontmatter").is_none());
        assert!(v.get("references").is_none());
    }

    // unused import guard
    #[allow(dead_code)]
    fn _touch(_p: PathBuf) {}
}

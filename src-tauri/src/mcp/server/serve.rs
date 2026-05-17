//! `hyprpilot mcp serve` — the rmcp-backed in-tree MCP server.
//!
//! Today's surface ships a single feature (skills); the structure is
//! deliberately generic so future features (workspace introspection,
//! codebase tooling, …) plug in alongside without spawning another
//! subcommand.
//!
//! Spawned by the agent vendor (via stdio) when the daemon auto-injects
//! the `hyprpilot` server entry into `session/new`'s `mcp_servers`
//! array. The sidecar reads `SKILL.md` files from paths the daemon
//! passed via repeated `--skill <slug>=<path>` args; no daemon-socket
//! dependency.
//!
//! Current surface:
//! - Resources
//!   - `hyprpilot://skills/<slug>` — full SKILL.md body
//!   - `hyprpilot://skills/<slug>/references` — bundled references the
//!     skill declares in its frontmatter, resolved relative to its own
//!     bundle directory
//! - Tools
//!   - `list_skills` — `[{ slug, title, description, uri }]`
//!   - `read_skill { slug }` — `{ uri, body }`
//!   - `load_skill_references { slug }` — `{ uri, body }`
//!   - `reload` — rescan disk, push list-changed notifications
//!
//! References are always accessed **through** the skill that declares
//! them — never as standalone resources. A skill at
//! `<root>/git-commit/SKILL.md` with `references: [../references/scm.md]`
//! resolves to `<root>/references/scm.md` at read time. The sidecar
//! does not maintain a separate references-root concept.

use std::sync::Arc;

use anyhow::Context;
use clap::Args;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Content, ErrorCode, Implementation, ListResourceTemplatesResult,
    ListResourcesResult, ListToolsResult, PaginatedRequestParams, RawResource, RawResourceTemplate,
    ReadResourceRequestParams, ReadResourceResult, ResourceContents, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::ServerHandler;
use rmcp::ServiceExt;
use tokio::sync::RwLock;

use crate::mcp::auto_inject::SKILLS_SERVER_NAME;

use super::skills::manifest::{parse_skill_arg, ManifestEntry};
use super::skills::references::{bundle_references, parse_frontmatter_references, FrontmatterRefs};

/// Args for `hyprpilot mcp serve`. Each feature contributes its own
/// arg fields — today only skills. Adding a future feature means
/// extending this struct + plumbing the new state through into
/// `HyprpilotServer::new`.
#[derive(Debug, Args, Clone)]
pub struct ServeArgs {
    /// Per-skill manifest entry, repeated. Format: `<slug>=<path-to-SKILL.md>`.
    #[arg(long = "skill", value_parser = parse_skill_arg)]
    pub skills: Vec<ManifestEntry>,
}

/// Run the rmcp stdio server in the foreground. Returns when the
/// vendor closes the pipe (or on init error).
pub async fn run(args: ServeArgs) -> anyhow::Result<()> {
    tracing::info!(
        skills = args.skills.len(),
        "mcp::server::serve: starting hyprpilot MCP server"
    );

    let handler = HyprpilotServer::new(args);
    // Synchronously prime the skills cache before opening the
    // transport — we can't take `blocking_write` from inside the
    // tokio runtime, and we don't want the first `tools/list` to race
    // an empty cache.
    handler.reload_skills().await;

    let (stdin, stdout) = rmcp::transport::io::stdio();
    let running = handler
        .serve((stdin, stdout))
        .await
        .context("mcp::server::serve: serve failed at init")?;
    running.waiting().await.ok();
    Ok(())
}

/// In-memory cache of loaded skill bodies. Built once at construction;
/// refreshed by the `reload` tool. Behind a single `RwLock` because
/// every tool / resource handler is read-heavy and `reload` is rare.
#[derive(Debug, Default)]
struct SkillsCache {
    /// slug → (manifest entry, parsed body, frontmatter refs).
    skills: std::collections::HashMap<String, LoadedSkill>,
    /// Insertion-order slug list for stable `list_skills` output.
    order: Vec<String>,
}

#[derive(Debug, Clone)]
struct LoadedSkill {
    entry: ManifestEntry,
    title: String,
    description: String,
    body: String,
    refs: FrontmatterRefs,
}

/// The hyprpilot in-tree MCP server. Today it owns the skills feature
/// state; future features (e.g. workspace introspection) add their own
/// caches alongside `skills_cache`.
#[derive(Clone)]
struct HyprpilotServer {
    args: ServeArgs,
    skills_cache: Arc<RwLock<SkillsCache>>,
}

impl HyprpilotServer {
    fn new(args: ServeArgs) -> Self {
        Self {
            args,
            skills_cache: Arc::new(RwLock::new(SkillsCache::default())),
        }
    }

    /// Re-read every `SKILL.md` from disk into a fresh cache. The
    /// filesystem reads run on a blocking thread so the rmcp runtime
    /// stays responsive; the swap takes the async write lock briefly.
    /// Read failures show up as the slug being absent from
    /// `list_skills` rather than aborting startup.
    async fn reload_skills(&self) {
        let entries = self.args.skills.clone();
        let fresh = tokio::task::spawn_blocking(move || build_cache(&entries))
            .await
            .unwrap_or_else(|err| {
                tracing::error!(%err, "mcp::server::serve: blocking reload join failed");
                SkillsCache::default()
            });
        let mut cache = self.skills_cache.write().await;
        *cache = fresh;
    }
}

fn build_cache(entries: &[ManifestEntry]) -> SkillsCache {
    let mut cache = SkillsCache::default();
    for entry in entries {
        match std::fs::read_to_string(&entry.path) {
            Ok(body) => {
                let refs = parse_frontmatter_references(&body);
                let (title, description) = extract_title_description(&body, &entry.slug);
                cache.order.push(entry.slug.clone());
                cache.skills.insert(
                    entry.slug.clone(),
                    LoadedSkill {
                        entry: entry.clone(),
                        title,
                        description,
                        body,
                        refs,
                    },
                );
            }
            Err(err) => {
                tracing::warn!(
                    slug = %entry.slug,
                    path = %entry.path.display(),
                    %err,
                    "mcp::server::serve: failed to read SKILL.md — slug will be absent",
                );
            }
        }
    }
    cache
}

/// Best-effort extraction of `title` + `description` from frontmatter.
/// Falls back to the slug + a stock description so the lister always
/// has something to surface.
fn extract_title_description(body: &str, slug: &str) -> (String, String) {
    let Some(yaml) = strip_frontmatter(body) else {
        return (slug.to_string(), format!("Guidance for {slug}"));
    };
    let Ok(value): Result<serde_yaml::Value, _> = serde_yaml::from_str(yaml) else {
        return (slug.to_string(), format!("Guidance for {slug}"));
    };
    let title = value
        .get("title")
        .and_then(serde_yaml::Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| slug.to_string());
    let description = value
        .get("description")
        .and_then(serde_yaml::Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("Guidance for {slug}"));
    (title, description)
}

fn strip_frontmatter(body: &str) -> Option<&str> {
    let body = body.strip_prefix("---\n").or_else(|| body.strip_prefix("---\r\n"))?;
    let end = body.find("\n---\n").or_else(|| body.find("\r\n---\r\n"))?;
    Some(&body[..end])
}

fn skill_uri(slug: &str) -> String {
    format!("hyprpilot://skills/{slug}")
}

fn skill_references_uri(slug: &str) -> String {
    format!("hyprpilot://skills/{slug}/references")
}

/// Parse a `hyprpilot://...` URI into its addressed slice.
enum ParsedUri<'a> {
    Skill(&'a str),
    SkillReferences(&'a str),
}

fn parse_uri(uri: &str) -> Option<ParsedUri<'_>> {
    let rest = uri.strip_prefix("hyprpilot://")?;
    let after = rest.strip_prefix("skills/")?;
    if let Some(slug) = after.strip_suffix("/references") {
        Some(ParsedUri::SkillReferences(slug))
    } else {
        Some(ParsedUri::Skill(after))
    }
}

fn empty_object_schema() -> Arc<serde_json::Map<String, serde_json::Value>> {
    let mut map = serde_json::Map::new();
    map.insert("type".into(), serde_json::Value::String("object".into()));
    map.insert("properties".into(), serde_json::Value::Object(serde_json::Map::new()));
    map.insert("additionalProperties".into(), serde_json::Value::Bool(false));
    Arc::new(map)
}

fn slug_object_schema() -> Arc<serde_json::Map<String, serde_json::Value>> {
    let mut map = serde_json::Map::new();
    map.insert("type".into(), serde_json::Value::String("object".into()));
    let mut props = serde_json::Map::new();
    let mut slug_prop = serde_json::Map::new();
    slug_prop.insert("type".into(), serde_json::Value::String("string".into()));
    slug_prop.insert(
        "description".into(),
        serde_json::Value::String("The skill slug.".into()),
    );
    props.insert("slug".into(), serde_json::Value::Object(slug_prop));
    map.insert("properties".into(), serde_json::Value::Object(props));
    map.insert(
        "required".into(),
        serde_json::Value::Array(vec![serde_json::Value::String("slug".into())]),
    );
    map.insert("additionalProperties".into(), serde_json::Value::Bool(false));
    Arc::new(map)
}

impl ServerHandler for HyprpilotServer {
    fn get_info(&self) -> ServerInfo {
        let mut caps = ServerCapabilities::default();
        caps.tools = Some(rmcp::model::ToolsCapability {
            list_changed: Some(true),
        });
        caps.resources = Some(rmcp::model::ResourcesCapability {
            subscribe: Some(false),
            list_changed: Some(true),
        });
        ServerInfo::new(caps)
            .with_server_info(Implementation::new(
                SKILLS_SERVER_NAME.to_string(),
                env!("CARGO_PKG_VERSION").to_string(),
            ))
            .with_instructions(
                "Hyprpilot in-tree MCP server. Skills are exposed as \
                 `hyprpilot://skills/<slug>` resources. Call `list_skills` to \
                 enumerate; `read_skill` to fetch a body; `load_skill_references` \
                 to bundle the references a skill declares in its frontmatter \
                 (resolved relative to the skill's bundle dir).",
            )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, rmcp::ErrorData> {
        let tools = vec![
            Tool::new_with_raw(
                "list_skills",
                Some("List every skill the daemon resolved for this session.".into()),
                empty_object_schema(),
            ),
            Tool::new_with_raw(
                "read_skill",
                Some(
                    "Read a skill's full SKILL.md body. Equivalent to reading the \
                     `hyprpilot://skills/<slug>` resource."
                        .into(),
                ),
                slug_object_schema(),
            ),
            Tool::new_with_raw(
                "load_skill_references",
                Some(
                    "Bundle every reference declared in a skill's frontmatter, \
                     resolved relative to the skill's bundle dir. Equivalent to \
                     reading `hyprpilot://skills/<slug>/references`."
                        .into(),
                ),
                slug_object_schema(),
            ),
            Tool::new_with_raw(
                "reload",
                Some(
                    "Rescan every SKILL.md from disk. Use after editing a skill on disk \
                     to refresh the cache without restarting the session."
                        .into(),
                ),
                empty_object_schema(),
            ),
        ];
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let args = request.arguments.unwrap_or_default();
        match request.name.as_ref() {
            "list_skills" => {
                let cache = self.skills_cache.read().await;
                let entries: Vec<serde_json::Value> = cache
                    .order
                    .iter()
                    .filter_map(|slug| cache.skills.get(slug))
                    .map(|s| {
                        serde_json::json!({
                            "slug": s.entry.slug,
                            "title": s.title,
                            "description": s.description,
                            "uri": skill_uri(&s.entry.slug),
                        })
                    })
                    .collect();
                Ok(CallToolResult::structured(serde_json::Value::Array(entries)))
            }
            "read_skill" => {
                let slug = require_string(&args, "slug")?;
                let cache = self.skills_cache.read().await;
                let Some(skill) = cache.skills.get(slug) else {
                    return Ok(CallToolResult::error(vec![Content::text(format!(
                        "unknown skill: {slug}"
                    ))]));
                };
                Ok(CallToolResult::structured(serde_json::json!({
                    "uri": skill_uri(slug),
                    "body": skill.body,
                })))
            }
            "load_skill_references" => {
                let slug = require_string(&args, "slug")?;
                let cache = self.skills_cache.read().await;
                let Some(skill) = cache.skills.get(slug) else {
                    return Ok(CallToolResult::error(vec![Content::text(format!(
                        "unknown skill: {slug}"
                    ))]));
                };
                let Some(bundle_dir) = skill.entry.bundle_dir() else {
                    return Ok(CallToolResult::error(vec![Content::text(
                        "skill manifest path has no parent directory",
                    )]));
                };
                let body = bundle_references(bundle_dir, &skill.refs);
                Ok(CallToolResult::structured(serde_json::json!({
                    "uri": skill_references_uri(slug),
                    "body": body,
                })))
            }
            "reload" => {
                // Filesystem reads run on a blocking thread so the
                // rmcp runtime stays responsive; the cache swap takes
                // the async write lock briefly.
                self.reload_skills().await;
                Ok(CallToolResult::structured(serde_json::json!({
                    "reloaded": self.skills_cache.read().await.skills.len(),
                })))
            }
            other => Err(rmcp::ErrorData::new(
                ErrorCode::METHOD_NOT_FOUND,
                format!("unknown tool: {other}"),
                None,
            )),
        }
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, rmcp::ErrorData> {
        let cache = self.skills_cache.read().await;
        let mut resources = Vec::with_capacity(cache.skills.len());
        for slug in &cache.order {
            let Some(skill) = cache.skills.get(slug) else { continue };
            resources.push(rmcp::model::Resource::new(
                RawResource::new(skill_uri(slug), slug.clone())
                    .with_title(skill.title.clone())
                    .with_description(skill.description.clone())
                    .with_mime_type("text/markdown"),
                None,
            ));
        }
        Ok(ListResourcesResult::with_all_items(resources))
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, rmcp::ErrorData> {
        let templates = vec![rmcp::model::ResourceTemplate::new(
            RawResourceTemplate::new("hyprpilot://skills/{slug}/references", "skill-references")
                .with_description(
                    "Bundle of every reference declared in the skill's frontmatter, \
                 concatenated with `--- <basename> ---` delimiters.",
                )
                .with_mime_type("text/markdown"),
            None,
        )];
        Ok(ListResourceTemplatesResult::with_all_items(templates))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, rmcp::ErrorData> {
        let uri = &request.uri;
        match parse_uri(uri) {
            Some(ParsedUri::Skill(slug)) => {
                let cache = self.skills_cache.read().await;
                let Some(skill) = cache.skills.get(slug) else {
                    return Err(rmcp::ErrorData::invalid_params(format!("unknown skill: {slug}"), None));
                };
                Ok(ReadResourceResult::new(vec![ResourceContents::TextResourceContents {
                    uri: uri.clone(),
                    mime_type: Some("text/markdown".into()),
                    text: skill.body.clone(),
                    meta: None,
                }]))
            }
            Some(ParsedUri::SkillReferences(slug)) => {
                let cache = self.skills_cache.read().await;
                let Some(skill) = cache.skills.get(slug) else {
                    return Err(rmcp::ErrorData::invalid_params(format!("unknown skill: {slug}"), None));
                };
                let Some(bundle_dir) = skill.entry.bundle_dir() else {
                    return Err(rmcp::ErrorData::internal_error(
                        "skill manifest path has no parent directory",
                        None,
                    ));
                };
                let body = bundle_references(bundle_dir, &skill.refs);
                Ok(ReadResourceResult::new(vec![ResourceContents::TextResourceContents {
                    uri: uri.clone(),
                    mime_type: Some("text/markdown".into()),
                    text: body,
                    meta: None,
                }]))
            }
            None => Err(rmcp::ErrorData::invalid_params(
                format!("unrecognised uri: {uri}"),
                None,
            )),
        }
    }
}

/// Resolve a required string arg from an MCP tool call payload.
fn require_string<'a>(
    args: &'a serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<&'a str, rmcp::ErrorData> {
    args.get(key)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| rmcp::ErrorData::invalid_params(format!("missing string argument `{key}`"), None))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_uris() {
        assert!(matches!(
            parse_uri("hyprpilot://skills/foo"),
            Some(ParsedUri::Skill("foo"))
        ));
        assert!(matches!(
            parse_uri("hyprpilot://skills/foo/references"),
            Some(ParsedUri::SkillReferences("foo"))
        ));
        assert!(parse_uri("hyprpilot://unknown/x").is_none());
        assert!(parse_uri("not-our-scheme://x").is_none());
    }

    #[test]
    fn extract_title_description_with_frontmatter() {
        let body = "---\ntitle: My Skill\ndescription: Does the thing\n---\nbody";
        let (t, d) = extract_title_description(body, "x");
        assert_eq!(t, "My Skill");
        assert_eq!(d, "Does the thing");
    }

    #[test]
    fn extract_title_description_fallbacks() {
        let (t, d) = extract_title_description("no frontmatter", "myslug");
        assert_eq!(t, "myslug");
        assert_eq!(d, "Guidance for myslug");
    }
}

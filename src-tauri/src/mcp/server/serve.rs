//! `hyprpilot mcp serve` — the rmcp-backed in-tree MCP server.
//!
//! Spawned by the agent vendor (via stdio) when the daemon auto-injects
//! the `hyprpilot` server entry into `session/new`'s `mcp_servers`
//! array. The sidecar reads skills by SCANNING DIRECTORIES directly —
//! the same discovery logic the daemon's `SkillsRegistry` uses — so
//! adding a new skill to a configured directory is immediately visible
//! after `reload`, and the daemon doesn't have to enumerate individual
//! files when building the spawn command.
//!
//! Current surface:
//! - Resources
//!   - `hyprpilot://skills/<slug>` — full SKILL.md body
//!   - `hyprpilot://skills/<slug>/references` — bundled references
//! - Tools
//!   - `list_skills` — `{ skills: [{ slug, title, description, uri, metadata }] }`
//!   - `read_skill { slug }` — `{ uri, body, metadata }`
//!   - `load_skill_references { slug }` — `{ uri, body, metadata }`
//!   - `reload` — rescan dirs, push list-changed notifications
//!   - `open { path }` — open a URL, file, or directory in the
//!     OS-default handler (`xdg-open` / `open` / `start`). The MCP
//!     server is a stdio sidecar (not inside the Tauri webview), so
//!     `tauri-plugin-shell`'s `open()` isn't reachable from here —
//!     the cross-platform `open` crate provides the same semantics.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use clap::Args;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Content, ErrorCode, Implementation, ListResourceTemplatesResult,
    ListResourcesResult, ListToolsResult, Meta, PaginatedRequestParams, RawResource, RawResourceTemplate,
    ReadResourceRequestParams, ReadResourceResult, ResourceContents, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::ServerHandler;
use rmcp::ServiceExt;
use serde::Serialize;
use tokio::sync::RwLock;

use crate::config::ResolvedSkillEntry;
use crate::mcp::auto_inject::SKILLS_SERVER_NAME;
use crate::skills::SkillsRegistry;

use super::skills::references::{bundle_references, frontmatter_references, FrontmatterRefs};

/// Args for `hyprpilot mcp serve`. Skills are discovered by directory
/// scan — the daemon passes `--skill-dir <path>` once per configured
/// root and `--skill-ignore <glob>` for slug patterns to suppress.
/// This mirrors how the daemon's own `SkillsRegistry` works at boot
/// and preserves each directory's own ignore list — a skill slug
/// suppressed in one root is still visible when it appears in
/// another root with no ignore for that pattern.
#[derive(Debug, Args, Clone)]
pub struct ServeArgs {
    /// JSON-encoded skill root entry. Repeatable — directories are
    /// searched in declaration order; first-slug-wins on collision.
    ///
    /// Shape: `{ "dir": "<abs-path>", "ignore": ["glob1", "glob2"] }`
    ///
    /// Encoding per-dir ignore globs inside the same arg keeps each
    /// entry self-contained: the daemon serializes its resolved
    /// `ResolvedSkillEntry` set as JSON objects, the sidecar
    /// deserializes and builds a matching `SkillsRegistry` that
    /// applies each root's ignore list independently.
    #[arg(long = "skill-dir", value_parser = parse_skill_dir_arg)]
    pub skill_dirs: Vec<SkillDirEntry>,
}

/// One decoded `--skill-dir` entry. The daemon serializes
/// `ResolvedSkillEntry` as JSON; the sidecar deserializes back.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct SkillDirEntry {
    pub dir: PathBuf,
    #[serde(default)]
    pub ignore: Vec<String>,
}

fn parse_skill_dir_arg(raw: &str) -> Result<SkillDirEntry, String> {
    serde_json::from_str::<SkillDirEntry>(raw)
        .map_err(|e| format!("--skill-dir must be a JSON object `{{\"dir\":\"...\",\"ignore\":[...]}}`: {e}"))
}

/// Run the rmcp stdio server in the foreground. Returns when the
/// vendor closes the pipe (or on init error).
pub async fn run(args: ServeArgs) -> anyhow::Result<()> {
    tracing::info!(
        dirs = args.skill_dirs.len(),
        "mcp::server::serve: starting hyprpilot MCP server"
    );

    let handler = HyprpilotServer::new(args)?;
    handler.reload_skills().await;

    let (stdin, stdout) = rmcp::transport::io::stdio();
    let running = handler
        .serve((stdin, stdout))
        .await
        .context("mcp::server::serve: serve failed at init")?;
    running.waiting().await.ok();
    Ok(())
}

// ── In-memory cache ───────────────────────────────────────────────────

#[derive(Debug, Default)]
struct SkillsCache {
    skills: std::collections::HashMap<String, LoadedSkill>,
    order: Vec<String>,
}

#[derive(Debug, Clone)]
struct LoadedSkill {
    slug: String,
    /// Absolute path to the `SKILL.md` file. Used to derive
    /// `bundle_dir` for reference resolution.
    path: PathBuf,
    title: String,
    description: String,
    metadata: SkillMetadata,
    body: String,
    refs: FrontmatterRefs,
}

impl LoadedSkill {
    fn bundle_dir(&self) -> Option<&std::path::Path> {
        self.path.parent()
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct SkillMetadata {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    interaction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    argument_hint: Option<String>,
    disable_model_invocation: bool,
    references: Vec<String>,
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    bundle_dir: Option<String>,
}

impl SkillMetadata {
    fn from_skill(skill: &crate::skills::Skill, refs: &FrontmatterRefs) -> Self {
        Self {
            name: frontmatter_string(&skill.frontmatter, "name").unwrap_or_else(|| skill.slug.to_string()),
            interaction: frontmatter_string(&skill.frontmatter, "interaction"),
            argument_hint: frontmatter_string(&skill.frontmatter, "argument-hint"),
            disable_model_invocation: frontmatter_bool(&skill.frontmatter, "disable-model-invocation").unwrap_or(false),
            references: refs.references.clone(),
            path: skill.path.display().to_string(),
            bundle_dir: skill.path.parent().map(|p| p.display().to_string()),
        }
    }
}

// ── Server ────────────────────────────────────────────────────────────

#[derive(Clone)]
struct HyprpilotServer {
    registry: Arc<SkillsRegistry>,
    skills_cache: Arc<RwLock<SkillsCache>>,
}

impl HyprpilotServer {
    fn new(args: ServeArgs) -> anyhow::Result<Self> {
        // Build one `ResolvedSkillEntry` per decoded `--skill-dir`
        // JSON entry. Each entry carries its OWN ignore list so the
        // sidecar replicates the daemon's per-dir suppression exactly —
        // a slug ignored in one root is still visible from another
        // root that doesn't suppress it. A bad glob is logged + skipped
        // rather than aborting startup (graceful degradation).
        let entries: Vec<ResolvedSkillEntry> = args
            .skill_dirs
            .into_iter()
            .map(|entry| {
                let ignore = if entry.ignore.is_empty() {
                    None
                } else {
                    let mut builder = globset::GlobSetBuilder::new();
                    for pat in &entry.ignore {
                        match globset::Glob::new(pat) {
                            Ok(g) => {
                                builder.add(g);
                            }
                            Err(err) => {
                                tracing::warn!(
                                    %err,
                                    pattern = %pat,
                                    dir = %entry.dir.display(),
                                    "mcp::server: bad skill-ignore glob — skipping"
                                );
                            }
                        }
                    }
                    builder.build().ok()
                };
                ResolvedSkillEntry {
                    dir: entry.dir,
                    ignore_patterns: entry.ignore,
                    ignore,
                }
            })
            .collect();

        Ok(Self {
            registry: Arc::new(SkillsRegistry::new(entries)),
            skills_cache: Arc::new(RwLock::new(SkillsCache::default())),
        })
    }

    async fn reload_skills(&self) {
        let registry = self.registry.clone();
        let result = tokio::task::spawn_blocking(move || {
            registry.reload().map_err(|e| e.to_string())?;
            Ok::<Vec<crate::skills::Skill>, String>(registry.list())
        })
        .await;

        let skills = match result {
            Ok(Ok(s)) => s,
            Ok(Err(err)) => {
                tracing::error!(%err, "mcp::server: skills reload failed");
                return;
            }
            Err(err) => {
                tracing::error!(%err, "mcp::server: blocking reload join failed");
                return;
            }
        };

        let mut cache = self.skills_cache.write().await;
        *cache = build_cache(skills);
    }
}

fn build_cache(skills: Vec<crate::skills::Skill>) -> SkillsCache {
    let mut cache = SkillsCache::default();
    for skill in skills {
        let slug = skill.slug.to_string();
        let refs = frontmatter_references(&skill.frontmatter);
        let metadata = SkillMetadata::from_skill(&skill, &refs);
        let title = if skill.title.trim().is_empty() {
            metadata.name.clone()
        } else {
            skill.title.clone()
        };
        let description = skill.description.clone();
        cache.order.push(slug.clone());
        cache.skills.insert(
            slug.clone(),
            LoadedSkill {
                slug,
                path: skill.path,
                title,
                description,
                metadata,
                body: skill.body,
                refs,
            },
        );
    }
    cache
}

// ── Helpers ───────────────────────────────────────────────────────────

fn frontmatter_string(value: &serde_yaml::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(serde_yaml::Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
}

fn frontmatter_bool(value: &serde_yaml::Value, key: &str) -> Option<bool> {
    value.get(key).and_then(serde_yaml::Value::as_bool)
}

fn skill_uri(slug: &str) -> String {
    format!("hyprpilot://skills/{slug}")
}

fn skill_references_uri(slug: &str) -> String {
    format!("hyprpilot://skills/{slug}/references")
}

fn list_skills_payload(cache: &SkillsCache) -> serde_json::Value {
    let entries: Vec<serde_json::Value> = cache
        .order
        .iter()
        .filter_map(|slug| cache.skills.get(slug))
        .map(|s| {
            serde_json::json!({
                "slug": s.slug,
                "title": s.title,
                "description": s.description,
                "uri": skill_uri(&s.slug),
                "metadata": s.metadata,
            })
        })
        .collect();

    serde_json::json!({ "skills": entries })
}

fn skill_meta(skill: &LoadedSkill) -> Meta {
    let mut meta = serde_json::Map::new();
    meta.insert(
        "skill".into(),
        serde_json::to_value(&skill.metadata).expect("skill metadata serializes"),
    );
    Meta(meta)
}

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

fn open_object_schema() -> Arc<serde_json::Map<String, serde_json::Value>> {
    let mut map = serde_json::Map::new();
    map.insert("type".into(), serde_json::Value::String("object".into()));
    let mut props = serde_json::Map::new();
    let mut path_prop = serde_json::Map::new();
    path_prop.insert("type".into(), serde_json::Value::String("string".into()));
    path_prop.insert(
        "description".into(),
        serde_json::Value::String(
            "URL, file path, or directory path to open in the OS default handler. \
             Accepts `https://`, `file://`, absolute paths, and relative paths — \
             the same shapes `xdg-open` / `open` / `start` accept natively."
                .into(),
        ),
    );
    props.insert("path".into(), serde_json::Value::Object(path_prop));
    map.insert("properties".into(), serde_json::Value::Object(props));
    map.insert(
        "required".into(),
        serde_json::Value::Array(vec![serde_json::Value::String("path".into())]),
    );
    map.insert("additionalProperties".into(), serde_json::Value::Bool(false));
    Arc::new(map)
}

/// Return a `CallToolResult` with `is_error: true` and a human-readable
/// message. Uses `CallToolResult::error` so the struct's `#[non_exhaustive]`
/// guard is respected — direct construction is rejected by the compiler.
fn tool_error(msg: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![Content::text(msg)])
}

fn require_string<'a>(
    args: &'a serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<&'a str, rmcp::ErrorData> {
    args.get(key)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| rmcp::ErrorData::invalid_params(format!("missing string argument `{key}`"), None))
}

// ── MCP protocol impl ─────────────────────────────────────────────────

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
                 (resolved relative to the skill's bundle dir). Use `open` to \
                 open a URL, file, or directory in the OS default handler.",
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
                Some("List every skill the daemon resolved for this session, including frontmatter metadata.".into()),
                empty_object_schema(),
            ),
            Tool::new_with_raw(
                "read_skill",
                Some(
                    "Read a skill's full SKILL.md body and frontmatter metadata. \
                     Equivalent to reading the `hyprpilot://skills/<slug>` resource."
                        .into(),
                ),
                slug_object_schema(),
            ),
            Tool::new_with_raw(
                "load_skill_references",
                Some(
                    "Bundle every reference declared in a skill's frontmatter, \
                     resolved relative to the skill's bundle dir, and include the \
                     skill metadata. Equivalent to reading \
                     `hyprpilot://skills/<slug>/references`."
                        .into(),
                ),
                slug_object_schema(),
            ),
            Tool::new_with_raw(
                "reload",
                Some(
                    "Rescan every skill directory from disk. Use after editing a \
                     skill file to refresh the cache without restarting the session."
                        .into(),
                ),
                empty_object_schema(),
            ),
            Tool::new_with_raw(
                "open",
                Some(
                    "Open a URL, file path, or directory in the OS default handler. \
                     Uses `xdg-open` on Linux, `open` on macOS, `start` on Windows. \
                     The MCP sidecar is a plain stdio process — this is a native OS \
                     call, not Tauri."
                        .into(),
                ),
                open_object_schema(),
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
                Ok(CallToolResult::structured(list_skills_payload(&cache)))
            }
            "read_skill" => {
                let slug = require_string(&args, "slug")?;
                let cache = self.skills_cache.read().await;
                let Some(skill) = cache.skills.get(slug) else {
                    return Ok(tool_error(format!("unknown skill: {slug}")));
                };
                Ok(CallToolResult::structured(serde_json::json!({
                    "uri": skill_uri(slug),
                    "body": skill.body,
                    "metadata": skill.metadata,
                })))
            }
            "load_skill_references" => {
                let slug = require_string(&args, "slug")?;
                let cache = self.skills_cache.read().await;
                let Some(skill) = cache.skills.get(slug) else {
                    return Ok(tool_error(format!("unknown skill: {slug}")));
                };
                let Some(bundle_dir) = skill.bundle_dir() else {
                    return Ok(tool_error("skill path has no parent directory"));
                };
                let body = bundle_references(bundle_dir, &skill.refs);
                Ok(CallToolResult::structured(serde_json::json!({
                    "uri": skill_references_uri(slug),
                    "body": body,
                    "metadata": skill.metadata,
                })))
            }
            "reload" => {
                self.reload_skills().await;
                let count = self.skills_cache.read().await.skills.len();
                Ok(CallToolResult::structured(serde_json::json!({
                    "reloaded": count,
                })))
            }
            "open" => {
                let path = require_string(&args, "path")?;
                match open::that_detached(path) {
                    Ok(()) => Ok(CallToolResult::structured(serde_json::json!({
                        "opened": path,
                    }))),
                    Err(err) => Ok(tool_error(format!("open failed: {err}"))),
                }
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
                    .with_mime_type("text/markdown")
                    .with_size(skill.body.len().try_into().unwrap_or(u32::MAX))
                    .with_meta(skill_meta(skill)),
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
        let templates = vec![
            rmcp::model::ResourceTemplate::new(
                RawResourceTemplate::new("hyprpilot://skills/{slug}", "skill")
                    .with_description("Full SKILL.md body for the addressed skill slug.")
                    .with_mime_type("text/markdown"),
                None,
            ),
            rmcp::model::ResourceTemplate::new(
                RawResourceTemplate::new("hyprpilot://skills/{slug}/references", "skill-references")
                    .with_description(
                        "Bundle of every reference declared in the skill's frontmatter, \
                     concatenated with `--- <basename> ---` delimiters.",
                    )
                    .with_mime_type("text/markdown"),
                None,
            ),
        ];
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
                    meta: Some(skill_meta(skill)),
                }]))
            }
            Some(ParsedUri::SkillReferences(slug)) => {
                let cache = self.skills_cache.read().await;
                let Some(skill) = cache.skills.get(slug) else {
                    return Err(rmcp::ErrorData::invalid_params(format!("unknown skill: {slug}"), None));
                };
                let Some(bundle_dir) = skill.bundle_dir() else {
                    return Err(rmcp::ErrorData::internal_error(
                        "skill path has no parent directory",
                        None,
                    ));
                };
                let body = bundle_references(bundle_dir, &skill.refs);
                Ok(ReadResourceResult::new(vec![ResourceContents::TextResourceContents {
                    uri: uri.clone(),
                    mime_type: Some("text/markdown".into()),
                    text: body,
                    meta: Some(skill_meta(skill)),
                }]))
            }
            None => Err(rmcp::ErrorData::invalid_params(
                format!("unrecognised uri: {uri}"),
                None,
            )),
        }
    }
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
    fn skill_metadata_reads_frontmatter() {
        let frontmatter: serde_yaml::Value = serde_yaml::from_str(
            r#"
name: plan-hard
interaction: chat
argument-hint: "[goal]"
disable-model-invocation: true
references:
  - ../references/plan-mode.md
"#,
        )
        .unwrap();
        let refs = frontmatter_references(&frontmatter);
        let skill = crate::skills::Skill {
            slug: crate::skills::SkillSlug::parse("plan-hard").unwrap(),
            title: String::new(),
            description: "Deep planning".to_string(),
            body: "body".to_string(),
            path: PathBuf::from("/tmp/plan-hard/SKILL.md"),
            frontmatter,
            references: Vec::new(),
        };

        let metadata = SkillMetadata::from_skill(&skill, &refs);

        assert_eq!(
            metadata,
            SkillMetadata {
                name: "plan-hard".to_string(),
                interaction: Some("chat".to_string()),
                argument_hint: Some("[goal]".to_string()),
                disable_model_invocation: true,
                references: vec!["../references/plan-mode.md".to_string()],
                path: "/tmp/plan-hard/SKILL.md".to_string(),
                bundle_dir: Some("/tmp/plan-hard".to_string()),
            }
        );
    }

    #[test]
    fn build_cache_falls_back_to_frontmatter_name_for_title() {
        let frontmatter: serde_yaml::Value = serde_yaml::from_str("name: myskill\n").unwrap();
        let skill = crate::skills::Skill {
            slug: crate::skills::SkillSlug::parse("myskill").unwrap(),
            title: String::new(),
            description: "desc".to_string(),
            body: "body".to_string(),
            path: PathBuf::from("/tmp/myskill/SKILL.md"),
            frontmatter,
            references: Vec::new(),
        };

        let cache = build_cache(vec![skill]);
        let loaded = cache.skills.get("myskill").unwrap();

        assert_eq!(loaded.title, "myskill");
        assert_eq!(loaded.description, "desc");
    }

    #[test]
    fn list_skills_payload_is_record_rooted() {
        let mut cache = SkillsCache::default();

        cache.order.push("plan-hard".to_string());
        cache.skills.insert(
            "plan-hard".to_string(),
            LoadedSkill {
                slug: "plan-hard".to_string(),
                path: PathBuf::from("/tmp/plan-hard/SKILL.md"),
                title: "Plan hard".to_string(),
                description: "Deep planning".to_string(),
                metadata: SkillMetadata {
                    name: "plan-hard".to_string(),
                    interaction: Some("chat".to_string()),
                    argument_hint: Some("[goal]".to_string()),
                    disable_model_invocation: true,
                    references: vec!["../references/plan-mode.md".to_string()],
                    path: "/tmp/plan-hard/SKILL.md".to_string(),
                    bundle_dir: Some("/tmp/plan-hard".to_string()),
                },
                body: String::new(),
                refs: FrontmatterRefs {
                    references: vec!["../references/plan-mode.md".to_string()],
                },
            },
        );

        let payload = list_skills_payload(&cache);

        assert!(payload.is_object());
        assert_eq!(
            payload,
            serde_json::json!({
                "skills": [{
                    "slug": "plan-hard",
                    "title": "Plan hard",
                    "description": "Deep planning",
                    "uri": "hyprpilot://skills/plan-hard",
                    "metadata": {
                        "name": "plan-hard",
                        "interaction": "chat",
                        "argumentHint": "[goal]",
                        "disableModelInvocation": true,
                        "references": ["../references/plan-mode.md"],
                        "path": "/tmp/plan-hard/SKILL.md",
                        "bundleDir": "/tmp/plan-hard"
                    }
                }]
            })
        );
    }

    #[test]
    fn skill_meta_is_nested_under_skill_key() {
        let skill = LoadedSkill {
            slug: "plan-hard".to_string(),
            path: PathBuf::from("/tmp/plan-hard/SKILL.md"),
            title: "Plan hard".to_string(),
            description: "Deep planning".to_string(),
            metadata: SkillMetadata {
                name: "plan-hard".to_string(),
                interaction: Some("chat".to_string()),
                argument_hint: None,
                disable_model_invocation: false,
                references: Vec::new(),
                path: "/tmp/plan-hard/SKILL.md".to_string(),
                bundle_dir: Some("/tmp/plan-hard".to_string()),
            },
            body: String::new(),
            refs: FrontmatterRefs::default(),
        };

        let meta = skill_meta(&skill);

        assert_eq!(
            meta.get("skill")
                .and_then(|v| v.get("name"))
                .and_then(serde_json::Value::as_str),
            Some("plan-hard")
        );
    }
}

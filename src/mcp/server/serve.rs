//! `hyprpilot mcp serve` — the rmcp-backed in-tree MCP server.
//!
//! Spawned by the agent vendor (via stdio) when the launcher
//! auto-injects the `hyprpilot` server entry into the vendor's MCP
//! catalog. The sidecar reads skills by SCANNING DIRECTORIES directly
//! — the same discovery logic the launcher's `SkillsRegistry` uses —
//! so adding a new skill to a configured directory is immediately
//! visible after `reload`, and the launcher doesn't have to enumerate
//! individual files when building the spawn command.
//!
//! Current surface:
//! - Resources
//!   - `hyprpilot://skills/<slug>` — full SKILL.md body
//!   - `hyprpilot://skills/<slug>/references` — bundled references
//!   - Both carry a namespaced `_meta`: `io.hyprpilot/frontmatter`
//!     (the ENTIRE parsed frontmatter, verbatim keys) and
//!     `io.hyprpilot/skill` (the curated `SkillMetadata` view). See
//!     `skills/metadata.rs`.
//! - Tools
//!   - `list_skills` — `{ skills: [{ slug, title, description, uri, metadata, frontmatter }] }`
//!   - `read_skill { slug }` — `{ uri, body, metadata, frontmatter }`
//!   - `load_skill_references { slug }` — `{ uri, body, metadata, frontmatter }`
//!   - `reload` — rescan dirs, push a resource list-changed
//!     notification (skills back the resource list; the tool list is
//!     static, so no tool-list-changed fires)
//!   - `open { path }` — open a URL, file, or directory in the
//!     OS-default handler (`xdg-open` / `open` / `start`) via the
//!     cross-platform `open` crate.
//!
//! Frontmatter passthrough is generic: `metadata` stays the narrow,
//! curated view (name / interaction / argument-hint /
//! disable-model-invocation / references / path / bundleDir — unchanged
//! shape, for backcompat); `frontmatter` is the WHOLE parsed YAML
//! frontmatter projected losslessly to JSON, so an author can add any
//! new frontmatter key and it reaches the agent with zero server
//! changes. `skills/metadata.rs` owns the conversion + the `_meta`
//! namespacing; this module just wires it into the cache + the wire
//! shapes.

use std::path::PathBuf;
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
use serde::Serialize;
use tokio::sync::RwLock;

use crate::config::ResolvedSkillEntry;
use crate::mcp::auto_inject::SKILLS_SERVER_NAME;
use crate::skills::SkillsRegistry;

use super::skills::metadata::{frontmatter_json, skill_meta};
use super::skills::references::{bundle_references, frontmatter_references, FrontmatterRefs};

/// Args for `hyprpilot mcp serve`. Skills are discovered by directory
/// scan — the launcher passes `--skill-dir <json>` once per configured
/// root, each carrying that root's ignore globs. This mirrors how the
/// launcher's own `SkillsRegistry` works and preserves each
/// directory's own ignore list — a skill slug suppressed in one root
/// is still visible when it appears in another root with no ignore for
/// that pattern.
#[derive(Debug, Args, Clone)]
pub struct ServeArgs {
    /// JSON-encoded skill root entry. Repeatable — directories are
    /// searched in declaration order; first-slug-wins on collision.
    ///
    /// Shape: `{ "dir": "<abs-path>", "ignore": ["glob1", "glob2"] }`
    ///
    /// Encoding per-dir ignore globs inside the same arg keeps each
    /// entry self-contained: the launcher serializes its resolved
    /// `ResolvedSkillEntry` set as JSON objects, the sidecar
    /// deserializes and builds a matching `SkillsRegistry` that
    /// applies each root's ignore list independently.
    #[arg(long = "skill-dir", value_parser = parse_skill_dir_arg)]
    pub skill_dirs: Vec<SkillDirEntry>,
}

/// One decoded `--skill-dir` entry. The launcher serializes
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
pub(crate) struct LoadedSkill {
    slug: String,
    /// Absolute path to the `SKILL.md` file. Used to derive
    /// `bundle_dir` for reference resolution.
    path: PathBuf,
    title: String,
    description: String,
    /// Curated/derived view — unchanged shape, kept for backcompat.
    pub(crate) metadata: SkillMetadata,
    /// The ENTIRE parsed frontmatter, losslessly projected to JSON
    /// once here (not per request) via
    /// `skills::metadata::frontmatter_json`.
    pub(crate) frontmatter_json: serde_json::Map<String, serde_json::Value>,
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
pub(crate) struct SkillMetadata {
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
        // sidecar replicates the launcher's per-dir suppression exactly —
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
        // Converted ONCE per skill here — not per request — so every
        // `list_resources` / `read_resource` / tool call reuses the
        // same lossless YAML→JSON projection.
        let frontmatter_json_value = frontmatter_json(&skill.frontmatter);
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
                frontmatter_json: frontmatter_json_value,
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
                "frontmatter": s.frontmatter_json,
            })
        })
        .collect();

    serde_json::json!({ "skills": entries })
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

/// A successful tool result carrying BOTH a human-readable `content`
/// text block AND the structured JSON payload. Clients that render only
/// `structuredContent` (Claude Code) read the JSON; clients that render
/// only `content` (opencode) read the text — a structured-only result
/// shows there as "Unknown". `CallToolResult::structured` sets
/// `structured_content` and a raw-JSON text block whose exact shape is
/// an rmcp-version detail, so we overwrite `content` with an explicit,
/// legible summary to guarantee the text block regardless of client or
/// rmcp version. `#[non_exhaustive]` forbids the struct literal but not
/// mutating the owned instance's public fields.
fn structured_with_text(summary: impl Into<String>, value: serde_json::Value) -> CallToolResult {
    let mut result = CallToolResult::structured(value);
    result.content = vec![Content::text(summary)];
    result
}

/// A one-line-per-skill catalogue for the `list_skills` text block.
fn list_skills_summary(cache: &SkillsCache) -> String {
    if cache.order.is_empty() {
        return "No skills available.".into();
    }
    let mut out = format!("{} skill(s) available:\n", cache.order.len());
    for slug in &cache.order {
        let Some(skill) = cache.skills.get(slug) else {
            continue;
        };
        if skill.description.is_empty() {
            out.push_str(&format!("- {}\n", skill.slug));
        } else {
            out.push_str(&format!("- {}: {}\n", skill.slug, skill.description));
        }
    }
    out.push_str("Call `read_skill` with a slug to fetch the full SKILL.md body.");
    out
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
        // The tool set is static (`list_tools` always returns the same
        // five) — do NOT advertise tool-list-changed. Skills back the
        // resource list, which `reload` can change, so resources DO
        // advertise list-changed (and `reload` fires it).
        caps.tools = Some(rmcp::model::ToolsCapability {
            list_changed: Some(false),
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
                 open a URL, file, or directory in the OS default handler. Every \
                 resource and tool result carries the skill's ENTIRE frontmatter \
                 verbatim (as `frontmatter` in tool output, and as \
                 `io.hyprpilot/frontmatter` in resource `_meta`) alongside the \
                 curated `metadata` view — read whichever shape fits.",
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
                Some("List every skill resolved for this session, including frontmatter metadata.".into()),
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
                     call."
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
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let args = request.arguments.unwrap_or_default();
        match request.name.as_ref() {
            "list_skills" => {
                let cache = self.skills_cache.read().await;
                Ok(structured_with_text(
                    list_skills_summary(&cache),
                    list_skills_payload(&cache),
                ))
            }
            "read_skill" => {
                let slug = require_string(&args, "slug")?;
                let cache = self.skills_cache.read().await;
                let Some(skill) = cache.skills.get(slug) else {
                    return Ok(tool_error(format!("unknown skill: {slug}")));
                };
                Ok(structured_with_text(
                    skill.body.clone(),
                    serde_json::json!({
                        "uri": skill_uri(slug),
                        "body": skill.body,
                        "metadata": skill.metadata,
                        "frontmatter": skill.frontmatter_json,
                    }),
                ))
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
                Ok(structured_with_text(
                    body.clone(),
                    serde_json::json!({
                        "uri": skill_references_uri(slug),
                        "body": body,
                        "metadata": skill.metadata,
                        "frontmatter": skill.frontmatter_json,
                    }),
                ))
            }
            "reload" => {
                self.reload_skills().await;
                let count = self.skills_cache.read().await.skills.len();
                // The resource list (one per skill) may have changed —
                // fire the list-changed notification `get_info`
                // advertises so a connected client re-fetches instead of
                // trusting a stale `list_resources`. Best-effort: a
                // failed notify logs but doesn't fail the reload.
                if let Err(err) = context.peer.notify_resource_list_changed().await {
                    tracing::debug!(%err, "mcp::server: reload resource list-changed notification failed");
                }
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
            // Body resource. `name` is the always-present slug; `title`
            // is the human title; `description` / `mimeType` / `size` /
            // `_meta` fill in the standard MCP Resource fields.
            resources.push(rmcp::model::Resource::new(
                RawResource::new(skill_uri(slug), skill.slug.clone())
                    .with_title(skill.title.clone())
                    .with_description(skill.description.clone())
                    .with_mime_type("text/markdown")
                    .with_size(skill.body.len().try_into().unwrap_or(u32::MAX))
                    .with_meta(skill_meta(skill)),
                None,
            ));
            // References resource — only listed when the skill actually
            // declares references, so the list never advertises an empty
            // bundle. Same standard-field population as the body.
            if !skill.refs.references.is_empty() {
                resources.push(rmcp::model::Resource::new(
                    RawResource::new(skill_references_uri(slug), format!("{}/references", skill.slug))
                        .with_title(format!("{} — references", skill.title))
                        .with_description(format!(
                            "Bundled reference files for the `{slug}` skill ({} declared).",
                            skill.refs.references.len()
                        ))
                        .with_mime_type("text/markdown")
                        .with_meta(skill_meta(skill)),
                    None,
                ));
            }
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
        };

        let cache = build_cache(vec![skill]);
        let loaded = cache.skills.get("myskill").unwrap();

        assert_eq!(loaded.title, "myskill");
        assert_eq!(loaded.description, "desc");
    }

    #[test]
    fn build_cache_populates_frontmatter_json_once() {
        let frontmatter: serde_yaml::Value = serde_yaml::from_str(
            r#"
name: myskill
license: MIT
"#,
        )
        .unwrap();
        let skill = crate::skills::Skill {
            slug: crate::skills::SkillSlug::parse("myskill").unwrap(),
            title: String::new(),
            description: "desc".to_string(),
            body: "body".to_string(),
            path: PathBuf::from("/tmp/myskill/SKILL.md"),
            frontmatter: frontmatter.clone(),
        };

        let cache = build_cache(vec![skill]);
        let loaded = cache.skills.get("myskill").unwrap();

        assert_eq!(loaded.frontmatter_json, frontmatter_json(&frontmatter));
        assert_eq!(
            loaded
                .frontmatter_json
                .get("license")
                .and_then(serde_json::Value::as_str),
            Some("MIT")
        );
    }

    /// Backcompat pin: today's `metadata` shape (and every other
    /// existing field) must stay byte-for-byte identical after adding
    /// the generic `frontmatter` passthrough field.
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
                frontmatter_json: serde_json::Map::new(),
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
                    },
                    "frontmatter": {}
                }]
            })
        );
    }

    /// Forward-compat: an arbitrary/unknown frontmatter key (nested
    /// map + array) rides through `list_skills`'s `frontmatter` field
    /// verbatim — proving zero-server-change forward compat for any
    /// new frontmatter field an author adds.
    #[test]
    fn list_skills_payload_frontmatter_carries_arbitrary_nested_key_verbatim() {
        let frontmatter: serde_yaml::Value = serde_yaml::from_str(
            r#"
name: plan-hard
x-vendor-extension:
  nested:
    - one
    - two
  flag: false
"#,
        )
        .unwrap();
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
                    interaction: None,
                    argument_hint: None,
                    disable_model_invocation: false,
                    references: Vec::new(),
                    path: "/tmp/plan-hard/SKILL.md".to_string(),
                    bundle_dir: Some("/tmp/plan-hard".to_string()),
                },
                frontmatter_json: frontmatter_json(&frontmatter),
                body: String::new(),
                refs: FrontmatterRefs::default(),
            },
        );

        let payload = list_skills_payload(&cache);

        let entry_frontmatter = &payload["skills"][0]["frontmatter"];
        assert_eq!(
            entry_frontmatter["x-vendor-extension"],
            serde_json::json!({ "nested": ["one", "two"], "flag": false })
        );
    }

    #[test]
    fn skill_meta_namespaces_curated_view_under_io_hyprpilot_skill() {
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
            frontmatter_json: serde_json::Map::new(),
            body: String::new(),
            refs: FrontmatterRefs::default(),
        };

        let meta = skill_meta(&skill);

        // Legacy bare `skill` key is gone — namespaced only.
        assert!(meta.get("skill").is_none());
        assert_eq!(
            meta.get("io.hyprpilot/skill")
                .and_then(|v| v.get("name"))
                .and_then(serde_json::Value::as_str),
            Some("plan-hard")
        );
    }

    #[test]
    fn skill_meta_frontmatter_carries_arbitrary_nested_key_verbatim() {
        let frontmatter: serde_yaml::Value = serde_yaml::from_str(
            r#"
name: plan-hard
license: MIT
metadata:
  owner: captain
  tags:
    - alpha
    - beta
"#,
        )
        .unwrap();
        let refs = frontmatter_references(&frontmatter);
        let metadata = SkillMetadata::from_skill(
            &crate::skills::Skill {
                slug: crate::skills::SkillSlug::parse("plan-hard").unwrap(),
                title: String::new(),
                description: "Deep planning".to_string(),
                body: "body".to_string(),
                path: PathBuf::from("/tmp/plan-hard/SKILL.md"),
                frontmatter: frontmatter.clone(),
            },
            &refs,
        );
        let skill = LoadedSkill {
            slug: "plan-hard".to_string(),
            path: PathBuf::from("/tmp/plan-hard/SKILL.md"),
            title: "Plan hard".to_string(),
            description: "Deep planning".to_string(),
            metadata,
            frontmatter_json: frontmatter_json(&frontmatter),
            body: String::new(),
            refs,
        };

        let meta = skill_meta(&skill);

        // `license` and the arbitrary nested `metadata` key are NOT
        // part of the curated `SkillMetadata` shape — proving they
        // only ride through via the raw frontmatter passthrough.
        let raw = meta.get("io.hyprpilot/frontmatter").expect("frontmatter present");
        assert_eq!(raw.get("license").and_then(serde_json::Value::as_str), Some("MIT"));
        let nested = raw.get("metadata").and_then(serde_json::Value::as_object).unwrap();
        assert_eq!(nested.get("owner").and_then(serde_json::Value::as_str), Some("captain"));
        assert_eq!(
            nested.get("tags").and_then(serde_json::Value::as_array).map(Vec::len),
            Some(2)
        );
    }
}

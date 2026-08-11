//! `hyprpilot mcp skills` — the rmcp-backed skills MCP server.
//!
//! Spawned by the agent vendor (via stdio) when the launcher
//! auto-injects the `hyprpilot_skills` server entry into the vendor's
//! MCP catalog. The sidecar reads skills by SCANNING DIRECTORIES directly
//! — the same discovery logic the launcher's `SkillsRegistry` uses —
//! so adding a new skill to a configured directory is immediately
//! visible after `reload`, and the launcher doesn't have to enumerate
//! individual files when building the spawn command.
//!
//! Current surface:
//! - Resources
//!   - `hyprpilot://skills` — the catalogue index (markdown), which
//!     also documents the two schemes below
//!   - `hyprpilot://skills/<slug>` — full SKILL.md body
//!   - `hyprpilot://references/<slug>` — every declared reference,
//!     bundled (parallel top-level scheme, NOT a `/references` segment
//!     nested under the slug — the nested form broke client URI
//!     autocomplete)
//!   - `hyprpilot://references/<slug>/<name>` — ONE reference, named by
//!     its file stem (or its own frontmatter `name`)
//!   - All carry ONE namespaced `_meta` key, `io.hyprpilot/skill`:
//!     the verbatim frontmatter MINUS `title`/`description` (already
//!     carried by the spec `Resource` fields) and MINUS `references`
//!     (superseded by the resolved manifest, which addresses each one
//!     by name instead of publishing its path) PLUS the runtime-derived
//!     `path`, `bundleDir`, `size`, `modified`, `created`. Nothing in
//!     that block repeats a spec-compliant `Resource` field. See
//!     `skills/wire_metadata.rs`.
//! - Tools
//!   - `list_skills` — `{ skills: [{ slug, title, description, uri,
//!     references: [name], metadata }] }`
//!   - `read_skill { slug, bundle? }` — `{ uri, body, references: [manifest],
//!     bundle, metadata }`. Reference BODIES are opt-in (`bundle: true`);
//!     the manifest naming and addressing each one always rides along,
//!     so declining a body is never a silent gap.
//!   - `load_skill_references { slug, references? }` — one or more
//!     reference bodies by name; omitted fetches all, `[]` fetches none
//!   - `reload` — rescan dirs, push a resource list-changed
//!     notification (skills back the resource list; the tool list is
//!     fixed for a given process, so no tool-list-changed fires)
//!
//! The harness tools (`spawn` / `session_*`) live on a SEPARATE server
//! — `hyprpilot mcp harness`, see `super::harness_server`. They were
//! once gated onto this one behind a `--with-harness` flag; splitting
//! them into their own process makes the gate structural (this server
//! cannot serve a harness tool because it does not implement one)
//! rather than a name check that had to be remembered in both
//! `list_tools` and `call_tool`.
//!
//! The tool list is fixed for the process, which is why
//! `tools.list_changed` stays `false` rather than being advertised as
//! dynamic.
//!
//! Metadata is de-duplicated to a SINGLE block (`metadata` in tool
//! output, `io.hyprpilot/skill` in resource `_meta`): the WHOLE parsed
//! YAML frontmatter projected losslessly to JSON, minus the keys another
//! field already carries — `title`/`description` (the spec fields, byte
//! for byte) and `references` (the resolved manifest) — plus the
//! runtime-derived `path`, `bundleDir`, `size`, `modified` and
//! `created`. An author can add any new frontmatter key and it reaches
//! the agent verbatim with zero server changes.
//! `skills/wire_metadata.rs` owns the conversion + the merge + the
//! `_meta` namespacing; this module wires it into the cache + the wire
//! shapes.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use clap::Args;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, ErrorCode, Implementation, ListResourceTemplatesResult,
    ListResourcesResult, ListToolsResult, PaginatedRequestParams, ReadResourceRequestParams, ReadResourceResponse,
    ReadResourceResult, ResourceContents, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::ServerHandler;
use rmcp::ServiceExt;
use tokio::sync::RwLock;

use crate::config::mcp::DEFAULT_SKILLS_SERVER_NAME;
use crate::config::ResolvedSkillEntry;
use crate::mcp::skills::SkillsRegistry;

use super::rpc::{empty_object_schema, require_string, structured_with_text, tool_error, wait_for_shutdown};
use crate::mcp::skills::wire_metadata::{frontmatter_json, skill_block, skill_meta};
use crate::mcp::skills::wire_references::{
    self, append_references, frontmatter_references, FrontmatterRefs, ReferenceEntry,
};

/// Args for `hyprpilot mcp skills`. Skills are discovered by directory
/// scan — the launcher passes `--skill-dir <json>` once per configured
/// root, each carrying that root's ignore globs. This mirrors how the
/// launcher's own `SkillsRegistry` works and preserves each
/// directory's own ignore list — a skill slug suppressed in one root
/// is still visible when it appears in another root with no ignore for
/// that pattern.
#[derive(Debug, Args, Clone)]
pub struct SkillsArgs {
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
pub async fn run_skills(args: SkillsArgs, config: super::ConfigSource) -> anyhow::Result<()> {
    tracing::info!(dirs = args.skill_dirs.len(), "mcp: starting the skills server");
    run(SkillsServer::new(args, config)?).await
}

async fn run(handler: SkillsServer) -> anyhow::Result<()> {
    handler.reload_skills().await;

    let (stdin, stdout) = rmcp::transport::io::stdio();
    let running = handler
        .serve((stdin, stdout))
        .await
        .context("mcp::server::skills_server: serve failed at init")?;

    // Race the transport against SIGTERM/SIGHUP. Without this a
    // supervisor stopping the sidecar would skip every destructor and
    // strand live sessions — `PR_SET_PDEATHSIG` still covers the
    // SIGKILL case, but only after the kernel notices, and it cannot
    // remove the session directories.
    wait_for_shutdown(running).await;

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
    /// The single de-duplicated metadata block, built once here (not
    /// per request): verbatim frontmatter MINUS `title`/`description`
    /// (carried by the spec `Resource` fields) PLUS runtime `path` +
    /// `bundleDir`. Serves both the tool `metadata` field and the
    /// resource `_meta` (`io.hyprpilot/skill`) — see
    /// `skills::wire_metadata::{skill_block, skill_meta}`.
    pub(crate) meta_block: serde_json::Map<String, serde_json::Value>,
    body: String,
    refs: FrontmatterRefs,
}

impl LoadedSkill {
    fn bundle_dir(&self) -> Option<&std::path::Path> {
        self.path.parent()
    }

    /// Resolve every declared reference: name, URI, timestamps, and its
    /// own frontmatter.
    ///
    /// Resolved from disk per call rather than cached alongside the
    /// body: `reload` is the body's invalidation point, but a reference
    /// is edited far more often than the skill that declares it, and
    /// caching here would serve a stale convention — and a stale mtime,
    /// which is the one thing these fields exist to report — until an
    /// unrelated reload happened to clear it.
    fn references(&self) -> Vec<ReferenceEntry> {
        if self.refs.references.is_empty() {
            return Vec::new();
        }
        self.bundle_dir()
            .map(|dir| wire_references::resolve(dir, &self.slug, &self.refs))
            .unwrap_or_default()
    }
}

// ── Server ────────────────────────────────────────────────────────────

#[derive(Clone)]
struct SkillsServer {
    registry: Arc<SkillsRegistry>,
    skills_cache: Arc<RwLock<SkillsCache>>,
}

impl SkillsServer {
    fn new(args: SkillsArgs, _config: super::ConfigSource) -> anyhow::Result<Self> {
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
                                    "mcp::server: bad skill ignore glob — skipping"
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

    /// Server instructions — the one place a client learns the whole
    /// workflow before it reads any individual tool schema.
    fn instructions(&self) -> String {
        String::from(
            "Hyprpilot skills MCP server. Skills are exposed as \
             `hyprpilot://skills/<slug>` resources. Call `list_skills` to \
             enumerate; `read_skill` to fetch a body. \
             A skill may declare REFERENCES — shared convention files it \
             cites by name. Their bodies are NOT included by default: \
             `read_skill` lists what the skill declares (name, address, size, \
             when it last changed), and you fetch only what the body directs \
             you to, with `load_skill_references { slug, references: [...] }` \
             or by reading `hyprpilot://references/<slug>/<name>`. Omit \
             `references` to get them all, or pass `bundle: true` to \
             `read_skill` for body-plus-everything in one call. A reference \
             is named by its file name without the extension. Bundled \
             references are delimited by a `reference:` YAML block naming \
             each one. Every resource and tool result carries the skill's \
             frontmatter verbatim in ONE block (as `metadata` in tool output, \
             and as the `io.hyprpilot/skill` key in resource `_meta`) — minus \
             `title` / `description` (already in the spec Resource fields) \
             and `references` (superseded by the manifest), plus the runtime \
             `path`, `bundleDir`, `size`, `modified` and `created`.",
        )
    }

    async fn reload_skills(&self) {
        let registry = self.registry.clone();
        let result = tokio::task::spawn_blocking(move || {
            registry.reload().map_err(|e| e.to_string())?;
            Ok::<Vec<crate::mcp::skills::Skill>, String>(registry.list())
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

fn build_cache(skills: Vec<crate::mcp::skills::Skill>) -> SkillsCache {
    let mut cache = SkillsCache::default();
    for skill in skills {
        let slug = skill.slug.to_string();
        let refs = frontmatter_references(&skill.frontmatter);
        // The single merged block, built ONCE per skill here — not per
        // request — so every `list_resources` / `read_resource` / tool
        // call reuses the same lossless YAML→JSON projection.
        let meta_block = skill_block(&frontmatter_json(&skill.frontmatter), &skill.path);
        // Title falls back to the frontmatter `name`, then the slug,
        // when no frontmatter `title` was set.
        let title = if skill.title.trim().is_empty() {
            frontmatter_string(&skill.frontmatter, "name").unwrap_or_else(|| slug.clone())
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
                meta_block,
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

fn skill_uri(slug: &str) -> String {
    format!("hyprpilot://skills/{slug}")
}

fn skill_references_uri(slug: &str) -> String {
    format!("hyprpilot://references/{slug}")
}

/// Comma-separated addressable names, for an error message that tells
/// the caller what it *could* have asked for.
fn available_names(entries: &[ReferenceEntry]) -> String {
    let names: Vec<String> = entries
        .iter()
        .filter(|e| !e.shadowed)
        .map(|e| format!("`{}`", e.name))
        .collect();
    if names.is_empty() {
        "(none - this skill declares no references)".to_string()
    } else {
        names.join(", ")
    }
}

fn list_skills_payload(cache: &SkillsCache) -> serde_json::Value {
    let entries: Vec<serde_json::Value> = cache
        .order
        .iter()
        .filter_map(|slug| cache.skills.get(slug))
        .map(|s| {
            // Names only, not the full manifest: `list_skills` is the
            // routing view ("which skill?"), and full manifests across a
            // large catalogue would add tens of kilobytes to every call.
            // `read_skill` carries the addressable detail.
            serde_json::json!({
                "slug": s.slug,
                "title": s.title,
                "description": s.description,
                "uri": skill_uri(&s.slug),
                "references": wire_references::names(&s.references()),
                "metadata": s.meta_block,
            })
        })
        .collect();

    serde_json::json!({ "skills": entries })
}

enum ParsedUri<'a> {
    /// The bare `hyprpilot://skills` index. Cannot collide with a slug:
    /// every skill URI carries a `skills/` prefix, and `strip_prefix`
    /// requires the separator.
    Catalogue,
    Skill(&'a str),
    /// Every reference a skill declares, bundled.
    SkillReferences(&'a str),
    /// One reference, addressed by name.
    SkillReference(&'a str, &'a str),
}

fn parse_uri(uri: &str) -> Option<ParsedUri<'_>> {
    let rest = uri.strip_prefix("hyprpilot://")?;
    // Two parallel top-level forms — the references scheme is NOT a
    // `/references` segment nested under the slug (that nesting broke
    // client URI autocomplete).
    if rest == "skills" {
        return Some(ParsedUri::Catalogue);
    }
    if let Some(slug) = rest.strip_prefix("skills/") {
        return Some(ParsedUri::Skill(slug));
    }
    let rest = rest.strip_prefix("references/")?;
    // Split at the FIRST separator: a slug can never contain one
    // (`SkillSlug::parse` rejects `/` and `\`) and a resolved reference
    // name is rejected back to its stem if it does, so one segment is
    // unambiguously the bundle and two are unambiguously one reference.
    // A trailing slash with no name (`references/<slug>/`) is neither —
    // it addresses nothing and must not silently become the bundle.
    match rest.split_once('/') {
        None => Some(ParsedUri::SkillReferences(rest)),
        Some((_, "")) => None,
        Some((slug, name)) => Some(ParsedUri::SkillReference(slug, name)),
    }
}

fn slug_prop() -> serde_json::Value {
    serde_json::json!({ "type": "string", "description": "The skill slug." })
}

fn object_schema(props: serde_json::Value) -> Arc<serde_json::Map<String, serde_json::Value>> {
    let serde_json::Value::Object(map) = serde_json::json!({
        "type": "object",
        "properties": props,
        "required": ["slug"],
        "additionalProperties": false,
    }) else {
        unreachable!("json! object literal")
    };
    Arc::new(map)
}

/// `load_skill_references`'s schema — `slug`, plus an optional ARRAY of
/// reference names.
///
/// An array rather than a single name because a skill body routinely
/// cites two or three references for one step, and a singular-only tool
/// turns that into N round trips. Omitted means every reference; an
/// explicitly EMPTY array means none, because an empty list must never
/// decay into its opposite.
fn load_references_object_schema() -> Arc<serde_json::Map<String, serde_json::Value>> {
    object_schema(serde_json::json!({
        "slug": slug_prop(),
        "references": {
            "type": "array",
            "items": { "type": "string" },
            "description":
                "Names of the references to fetch, as listed in the skill's `references` \
                 manifest (the file name without its extension, e.g. `output-diff`). \
                 Omit to fetch every reference the skill declares. An empty array fetches \
                 none. An unknown name is an error rather than a partial result.",
        },
    }))
}

/// `read_skill`'s schema — `slug`, plus an opt-IN for the full bundle.
///
/// Bundling defaults OFF. The body always carries a manifest of what the
/// skill declares — name, URI, size, mtime — so the agent can see what
/// it has not loaded and fetch precisely what the body tells it to.
/// `bundle: true` is the one-call shortcut for when everything is wanted
/// anyway.
fn read_skill_object_schema() -> Arc<serde_json::Map<String, serde_json::Value>> {
    object_schema(serde_json::json!({
        "slug": slug_prop(),
        "bundle": {
            "type": "boolean",
            "description":
                "Append the full body of every declared reference. Defaults to false - the \
                 result always lists what the skill declares and how to address each one, \
                 so fetch only what the skill body actually directs you to. Pass true only \
                 when you want every reference in one call.",
        },
    }))
}

/// The `hyprpilot://skills` index — the whole catalogue as one
/// markdown document.
///
/// Exists for the ATTACHMENT path: a client injecting this costs no
/// tool call at all. A model reading it still spends one (a generic
/// resource read), so `list_skills` stays the better route for the
/// model — same cost, but named and described.
///
/// It leads with how to chain the other two schemes, because an index
/// whose entries the reader cannot then load is only half an answer.
fn catalogue_markdown(cache: &SkillsCache) -> String {
    let mut out = String::from(
        "# hyprpilot skills\n\n\
         Each entry below is loadable by URI — no tool call required:\n\n\
         - `hyprpilot://skills/<slug>` — the skill's full `SKILL.md` body. Read this first; it is the \
         instruction set. It ends with a list of the references that skill declares, but not their \
         bodies.\n\
         - `hyprpilot://references/<slug>/<name>` — ONE of those references. `<name>` is its file name \
         without the extension, exactly as the skill body cites it.\n\
         - `hyprpilot://references/<slug>` — all of them at once, when the body genuinely needs them all.\n\n\
         So the chain is: pick a slug here → read `skills/<slug>` → follow the reference directives in \
         its body into `references/<slug>/<name>`, one at a time. Shared references repeat heavily \
         across skills, so fetching only what a step names keeps what you already hold from being \
         re-sent. The `reload` tool rescans the roots if this index looks stale.\n\n",
    );
    if cache.order.is_empty() {
        out.push_str("_No skills available._\n");
        return out;
    }
    out.push_str(&format!("## {} available\n\n", cache.order.len()));
    for slug in &cache.order {
        let Some(skill) = cache.skills.get(slug) else {
            continue;
        };
        out.push_str(&format!("### `{}`\n\n", skill.slug));
        if !skill.title.is_empty() && skill.title != skill.slug.as_str() {
            out.push_str(&format!("**{}**\n\n", skill.title));
        }
        if !skill.description.is_empty() {
            out.push_str(&format!("{}\n\n", skill.description));
        }
        out.push_str(&format!("`{}`", skill_uri(slug)));
        if !skill.refs.references.is_empty() {
            out.push_str(&format!(
                " · {} reference(s) at `{}`",
                skill.refs.references.len(),
                skill_references_uri(slug)
            ));
        }
        out.push_str("\n\n");
    }

    out
}

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

// ── MCP protocol impl ─────────────────────────────────────────────────

impl ServerHandler for SkillsServer {
    fn supported_protocol_versions(&self) -> std::borrow::Cow<'static, [rmcp::model::ProtocolVersion]> {
        super::rpc::supported_protocol_versions()
    }

    fn get_info(&self) -> ServerInfo {
        let mut caps = ServerCapabilities::default();
        // The tool set is fixed for the life of THIS process — the
        // four skills tools. It never changes, so do NOT advertise
        // tool-list-changed. Skills back the resource list, which
        // `reload` can change, so resources DO advertise list-changed
        // (and `reload` fires it).
        // rmcp 2 marks these `#[non_exhaustive]` — no struct literal
        // outside the crate — so mutate the owned `default()` instances'
        // public fields instead.
        let mut tools = rmcp::model::ToolsCapability::default();
        tools.list_changed = Some(false);
        caps.tools = Some(tools);
        let mut resources = rmcp::model::ResourcesCapability::default();
        resources.subscribe = Some(false);
        resources.list_changed = Some(true);
        caps.resources = Some(resources);
        ServerInfo::new(caps)
            .with_server_info(Implementation::new(
                DEFAULT_SKILLS_SERVER_NAME.to_string(),
                env!("CARGO_PKG_VERSION").to_string(),
            ))
            .with_instructions(self.instructions())
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
                    "Read a skill's full SKILL.md body and frontmatter metadata. The result \
                     also lists every reference the skill declares - name, address, size and \
                     when it last changed - but NOT their bodies. Fetch those with \
                     `load_skill_references`, or pass `bundle: true` to get them all in one \
                     call. Equivalent to reading the `hyprpilot://skills/<slug>` resource."
                        .into(),
                ),
                read_skill_object_schema(),
            ),
            Tool::new_with_raw(
                "load_skill_references",
                Some(
                    "Fetch the body of one or more references a skill declares, by name. \
                     Pass `references: [\"output-diff\", \"scm-detect\"]` for specific ones, \
                     or omit it for all of them. Equivalent to reading \
                     `hyprpilot://references/<slug>/<name>` (one) or \
                     `hyprpilot://references/<slug>` (all)."
                        .into(),
                ),
                load_references_object_schema(),
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
        ];
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, rmcp::ErrorData> {
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
                let want_bundle = args.get("bundle").and_then(serde_json::Value::as_bool).unwrap_or(false);
                let cache = self.skills_cache.read().await;
                let Some(skill) = cache.skills.get(slug) else {
                    return Ok(tool_error(format!("unknown skill: {slug}")));
                };
                let entries = skill.references();
                // Opting into the bundle replaces the footer with the
                // real thing; otherwise the footer is what tells the
                // reader those bodies exist and how to reach them.
                let text = if want_bundle {
                    let bundle = wire_references::bundle(&entries, None).unwrap_or_default();
                    append_references(&skill.body, slug, entries.len(), &bundle)
                } else {
                    format!("{}{}", skill.body, wire_references::manifest_footer(&entries, slug))
                };
                // `body` stays the body — appending into it would change
                // the field's meaning for anything reading the structured
                // result. The concatenation is the text projection only.
                Ok(structured_with_text(
                    text,
                    serde_json::json!({
                        "uri": skill_uri(slug),
                        "body": skill.body,
                        "references": wire_references::manifest(&entries),
                        "bundle": want_bundle
                            .then(|| wire_references::bundle(&entries, None).unwrap_or_default()),
                        "metadata": skill.meta_block,
                    }),
                ))
            }
            "load_skill_references" => {
                let slug = require_string(&args, "slug")?;
                // Absent means every reference; an explicitly empty
                // array means none. The two must not collapse — an empty
                // list decaying into "everything" is exactly the footgun
                // `--no-delegates` exists to avoid on the harness side.
                let select = match args.get("references") {
                    None | Some(serde_json::Value::Null) => None,
                    Some(serde_json::Value::Array(items)) => {
                        let mut names = Vec::with_capacity(items.len());
                        for item in items {
                            let Some(name) = item.as_str() else {
                                return Ok(tool_error("`references` must be an array of strings"));
                            };
                            names.push(name.to_string());
                        }
                        Some(names)
                    }
                    Some(_) => return Ok(tool_error("`references` must be an array of strings")),
                };
                let cache = self.skills_cache.read().await;
                let Some(skill) = cache.skills.get(slug) else {
                    return Ok(tool_error(format!("unknown skill: {slug}")));
                };
                let entries = skill.references();
                let body = match wire_references::bundle(&entries, select.as_deref()) {
                    Ok(body) => body,
                    Err(unknown) => {
                        return Ok(tool_error(format!(
                            "skill `{slug}` declares no reference named {}. Available: {}",
                            unknown.iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", "),
                            available_names(&entries)
                        )))
                    }
                };
                Ok(structured_with_text(
                    body.clone(),
                    serde_json::json!({
                        "uri": skill_references_uri(slug),
                        "body": body,
                        "references": wire_references::manifest(&entries),
                        "metadata": skill.meta_block,
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
                Ok(structured_with_text(
                    format!("Reloaded {count} skill(s)."),
                    serde_json::json!({ "reloaded": count }),
                ))
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
        let mut resources = Vec::with_capacity(cache.skills.len() + 1);
        // The index goes FIRST — it is the entry point, and it explains
        // how to load everything under it.
        let catalogue = catalogue_markdown(&cache);
        resources.push(
            rmcp::model::Resource::new("hyprpilot://skills", "skills")
                .with_title("hyprpilot skills — catalogue")
                .with_description(format!(
                    "Every available skill with its description, and how to load one: read \
                     `hyprpilot://skills/<slug>` for the body, then `hyprpilot://references/<slug>` for \
                     the files it declares. {} skill(s).",
                    cache.order.len()
                ))
                .with_mime_type("text/markdown")
                .with_size(catalogue.len() as u64),
        );
        for slug in &cache.order {
            let Some(skill) = cache.skills.get(slug) else { continue };
            // Body resource. `name` is the always-present slug; `title`
            // is the human title; `description` / `mimeType` / `size` /
            // `_meta` fill in the standard MCP Resource fields.
            resources.push(
                rmcp::model::Resource::new(skill_uri(slug), skill.slug.clone())
                    .with_title(skill.title.clone())
                    .with_description(skill.description.clone())
                    .with_mime_type("text/markdown")
                    .with_size(skill.body.len() as u64)
                    .with_meta(skill_meta(&skill.meta_block)),
            );
            // References resource — only listed when the skill actually
            // declares references, so the list never advertises an empty
            // bundle. Same standard-field population as the body.
            if !skill.refs.references.is_empty() {
                resources.push(
                    rmcp::model::Resource::new(skill_references_uri(slug), format!("{} references", skill.slug))
                        .with_title(format!("{} — references", skill.title))
                        .with_description(format!(
                            "Bundled reference files for the `{slug}` skill ({} declared).",
                            skill.refs.references.len()
                        ))
                        .with_mime_type("text/markdown")
                        .with_meta(skill_meta(&skill.meta_block)),
                );
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
            rmcp::model::ResourceTemplate::new("hyprpilot://skills/{slug}", "skill")
                .with_description("Full SKILL.md body for the addressed skill slug.")
                .with_mime_type("text/markdown"),
            rmcp::model::ResourceTemplate::new("hyprpilot://references/{slug}", "skill-references")
                .with_description(
                    "Every reference the skill declares, bundled into one document, each \
                     delimited by a `reference:` block naming it.",
                )
                .with_mime_type("text/markdown"),
            rmcp::model::ResourceTemplate::new("hyprpilot://references/{slug}/{reference}", "skill-reference")
                .with_description(
                    "One reference, addressed by name - the file name without its \
                     extension, as listed in the skill's `references` manifest.",
                )
                .with_mime_type("text/markdown"),
        ];
        Ok(ListResourceTemplatesResult::with_all_items(templates))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, rmcp::ErrorData> {
        let uri = &request.uri;
        match parse_uri(uri) {
            Some(ParsedUri::Catalogue) => {
                let cache = self.skills_cache.read().await;
                Ok(ReadResourceResult::new(vec![ResourceContents::TextResourceContents {
                    uri: uri.clone(),
                    mime_type: Some("text/markdown".into()),
                    text: catalogue_markdown(&cache),
                    meta: None,
                }])
                .into())
            }
            Some(ParsedUri::Skill(slug)) => {
                let cache = self.skills_cache.read().await;
                let Some(skill) = cache.skills.get(slug) else {
                    return Err(rmcp::ErrorData::invalid_params(format!("unknown skill: {slug}"), None));
                };
                // The attachment path — palette picks and `#{...}` land
                // here — and the one place a manifest FOOTER is
                // load-bearing rather than a nicety: a resource read
                // returns text plus `_meta`, and many clients never
                // surface `_meta` to the model. Without the footer an
                // attached skill would lose its references with no
                // in-context signal at all, which is the silent gap
                // bundling-by-default used to prevent.
                let entries = skill.references();
                Ok(ReadResourceResult::new(vec![ResourceContents::TextResourceContents {
                    uri: uri.clone(),
                    mime_type: Some("text/markdown".into()),
                    text: format!("{}{}", skill.body, wire_references::manifest_footer(&entries, slug)),
                    meta: Some(skill_meta(&skill.meta_block)),
                }])
                .into())
            }
            Some(ParsedUri::SkillReferences(slug)) => {
                let cache = self.skills_cache.read().await;
                let Some(skill) = cache.skills.get(slug) else {
                    return Err(rmcp::ErrorData::invalid_params(format!("unknown skill: {slug}"), None));
                };
                let body = wire_references::bundle(&skill.references(), None).unwrap_or_default();
                Ok(ReadResourceResult::new(vec![ResourceContents::TextResourceContents {
                    uri: uri.clone(),
                    mime_type: Some("text/markdown".into()),
                    text: body,
                    meta: Some(skill_meta(&skill.meta_block)),
                }])
                .into())
            }
            Some(ParsedUri::SkillReference(slug, name)) => {
                let cache = self.skills_cache.read().await;
                let Some(skill) = cache.skills.get(slug) else {
                    return Err(rmcp::ErrorData::invalid_params(format!("unknown skill: {slug}"), None));
                };
                let entries = skill.references();
                // A resource read has no in-band marker convention — a
                // client asking for one reference gets content or an
                // error, so an unknown name errors here rather than
                // returning the soft `status: not-found` block the
                // bundle uses for a declared-but-unreadable file.
                let body = wire_references::bundle(&entries, Some(&[name.to_string()])).map_err(|_| {
                    rmcp::ErrorData::invalid_params(
                        format!(
                            "skill `{slug}` declares no reference named `{name}`. Available: {}",
                            available_names(&entries)
                        ),
                        None,
                    )
                })?;
                Ok(ReadResourceResult::new(vec![ResourceContents::TextResourceContents {
                    uri: uri.clone(),
                    mime_type: Some("text/markdown".into()),
                    text: body,
                    meta: Some(skill_meta(&skill.meta_block)),
                }])
                .into())
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

    /// The bare index URI must not shadow a skill. Every skill URI
    /// carries a `skills/` prefix, so the equality check has to come
    /// first and `skillsfoo` must still be nothing.
    #[test]
    fn the_catalogue_uri_cannot_shadow_a_slug() {
        assert!(matches!(parse_uri("hyprpilot://skills"), Some(ParsedUri::Catalogue)));
        assert!(matches!(
            parse_uri("hyprpilot://skills/git-commit"),
            Some(ParsedUri::Skill("git-commit"))
        ));
        assert!(matches!(
            parse_uri("hyprpilot://references/git-commit"),
            Some(ParsedUri::SkillReferences("git-commit"))
        ));
        assert!(parse_uri("hyprpilot://skillsfoo").is_none());
        assert!(parse_uri("hyprpilot://nope").is_none());
    }

    /// The index leads with how to chain the other two schemes — an
    /// index whose entries the reader cannot then load is half an answer.
    #[test]
    fn the_catalogue_explains_how_to_load_what_it_lists() {
        let empty = SkillsCache::default();
        let out = catalogue_markdown(&empty);
        assert!(out.contains("hyprpilot://skills/<slug>"), "must name the body scheme");
        assert!(
            out.contains("hyprpilot://references/<slug>"),
            "must name the references scheme"
        );
        assert!(out.contains("No skills available"), "an empty catalogue still renders");
    }

    /// The single-reference form splits at the FIRST separator, so a
    /// slug and a name are never confused. A trailing slash addresses
    /// nothing and must NOT silently degrade into the bundle.
    #[test]
    fn a_single_reference_uri_splits_slug_from_name() {
        assert!(matches!(
            parse_uri("hyprpilot://references/git-commit/output-diff"),
            Some(ParsedUri::SkillReference("git-commit", "output-diff"))
        ));
        // `file_stem` strips only the LAST extension, so a dotted name
        // is legal and must survive intact.
        assert!(matches!(
            parse_uri("hyprpilot://references/git-commit/plan.v2"),
            Some(ParsedUri::SkillReference("git-commit", "plan.v2"))
        ));
        // One segment is still the whole bundle.
        assert!(matches!(
            parse_uri("hyprpilot://references/git-commit"),
            Some(ParsedUri::SkillReferences("git-commit"))
        ));
        // A trailing slash is neither.
        assert!(parse_uri("hyprpilot://references/git-commit/").is_none());
    }

    #[test]
    fn parses_known_uris() {
        // Body: the `skills/<slug>` top-level form.
        assert!(matches!(
            parse_uri("hyprpilot://skills/foo"),
            Some(ParsedUri::Skill("foo"))
        ));
        // References: the parallel `references/<slug>` top-level form
        // (NOT `skills/<slug>/references` — that nesting broke client
        // autocomplete).
        assert!(matches!(
            parse_uri("hyprpilot://references/foo"),
            Some(ParsedUri::SkillReferences("foo"))
        ));
        // The old nested form is no longer a references URI — it parses
        // as a body slug that literally contains `foo/references`, which
        // resolves to no known skill rather than the references bundle.
        assert!(matches!(
            parse_uri("hyprpilot://skills/foo/references"),
            Some(ParsedUri::Skill("foo/references"))
        ));
        assert!(parse_uri("hyprpilot://unknown/x").is_none());
        assert!(parse_uri("not-our-scheme://x").is_none());
    }

    fn loaded_skill(slug: &str, title: &str, description: &str, frontmatter_yaml: &str, path: &str) -> LoadedSkill {
        let frontmatter: serde_yaml::Value = serde_yaml::from_str(frontmatter_yaml).unwrap();
        let path = PathBuf::from(path);
        LoadedSkill {
            slug: slug.to_string(),
            meta_block: skill_block(&frontmatter_json(&frontmatter), &path),
            path,
            title: title.to_string(),
            description: description.to_string(),
            body: String::new(),
            refs: frontmatter_references(&frontmatter),
        }
    }

    #[test]
    fn build_cache_falls_back_to_frontmatter_name_for_title() {
        let frontmatter: serde_yaml::Value = serde_yaml::from_str("name: myskill\n").unwrap();
        let skill = crate::mcp::skills::Skill {
            slug: crate::mcp::skills::SkillSlug::parse("myskill").unwrap(),
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

    /// References are resolved but their BODIES are not served by
    /// default — the skill body carries a manifest footer naming each
    /// one and how to address it, so the reader can see what it has not
    /// loaded. Opting in swaps the footer for the real bundle.
    #[test]
    fn references_resolve_to_addresses_and_the_bodies_are_opt_in() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("myskill");
        std::fs::create_dir_all(skill_dir.join("references")).unwrap();
        std::fs::write(skill_dir.join("references/local.md"), "local body").unwrap();

        let frontmatter: serde_yaml::Value =
            serde_yaml::from_str("name: myskill\nreferences:\n  - ./references/local.md\n").unwrap();
        let skill = crate::mcp::skills::Skill {
            slug: crate::mcp::skills::SkillSlug::parse("myskill").unwrap(),
            title: String::new(),
            description: "desc".to_string(),
            body: "body".to_string(),
            path: skill_dir.join("SKILL.md"),
            frontmatter,
        };

        let cache = build_cache(vec![skill]);
        let loaded = cache.skills.get("myskill").unwrap();
        let entries = loaded.references();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "local");
        assert_eq!(entries[0].uri.as_deref(), Some("hyprpilot://references/myskill/local"));

        // Default: the address, not the body.
        let footer = wire_references::manifest_footer(&entries, "myskill");
        assert!(footer.contains("uri: hyprpilot://references/myskill/local"));
        assert!(
            !footer.contains("local body"),
            "the default must not carry reference bodies"
        );

        // Opt-in: the body.
        let bundled = wire_references::bundle(&entries, None).unwrap();
        assert!(bundled.contains("name: local"));
        assert!(bundled.contains("local body"));
        let text = append_references(&loaded.body, "myskill", entries.len(), &bundled);
        assert!(text.starts_with("body\n"));
        assert!(text.contains("skill_references:\n  skill: myskill\n  count: 1"));
    }

    /// frontmatter keys survive, and the runtime `path` + `bundleDir`
    /// are injected.
    #[test]
    fn build_cache_builds_single_meta_block() {
        let frontmatter: serde_yaml::Value = serde_yaml::from_str(
            r#"
name: myskill
license: MIT
"#,
        )
        .unwrap();
        let skill = crate::mcp::skills::Skill {
            slug: crate::mcp::skills::SkillSlug::parse("myskill").unwrap(),
            title: String::new(),
            description: "desc".to_string(),
            body: "body".to_string(),
            path: PathBuf::from("/tmp/myskill/SKILL.md"),
            frontmatter,
        };

        let cache = build_cache(vec![skill]);
        let loaded = cache.skills.get("myskill").unwrap();

        assert_eq!(
            loaded.meta_block.get("license").and_then(serde_json::Value::as_str),
            Some("MIT")
        );
        assert_eq!(
            loaded.meta_block.get("path").and_then(serde_json::Value::as_str),
            Some("/tmp/myskill/SKILL.md")
        );
        assert_eq!(
            loaded.meta_block.get("bundleDir").and_then(serde_json::Value::as_str),
            Some("/tmp/myskill")
        );
    }

    /// The `list_skills` entry keeps the headline `slug`/`title`/
    /// `description`/`uri` scan view and a SINGLE `metadata` block —
    /// no separate `frontmatter` field, and no `title`/`description`
    /// repeated inside the block (they are the headline fields already).
    #[test]
    fn list_skills_payload_single_block_no_spec_dupes() {
        let mut cache = SkillsCache::default();
        cache.order.push("plan-hard".to_string());
        cache.skills.insert(
            "plan-hard".to_string(),
            loaded_skill(
                "plan-hard",
                "Plan hard",
                "Deep planning",
                r#"
name: plan-hard
title: Plan hard
description: Deep planning
argument-hint: "[goal]"
references:
  - ../references/plan-mode.md
"#,
                "/tmp/plan-hard/SKILL.md",
            ),
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
                    // Names only — the routing view. `read_skill`
                    // carries the addressable manifest.
                    "references": ["plan-mode"],
                    "metadata": {
                        "name": "plan-hard",
                        "argument-hint": "[goal]",
                        "path": "/tmp/plan-hard/SKILL.md",
                        "bundleDir": "/tmp/plan-hard"
                    }
                }]
            })
        );
        // No separate `frontmatter` field anymore.
        assert!(payload["skills"][0].get("frontmatter").is_none());
        // Spec-duplicated keys are NOT inside the block.
        assert!(payload["skills"][0]["metadata"].get("title").is_none());
        assert!(payload["skills"][0]["metadata"].get("description").is_none());
        // The raw declared paths are superseded by the manifest and must
        // not ride along — publishing them invites reading the files
        // directly instead of going through the server.
        assert!(payload["skills"][0]["metadata"].get("references").is_none());
        assert!(!payload.to_string().contains("../references/"));
    }

    /// Forward-compat: an arbitrary/unknown frontmatter key (nested
    /// map + array) rides through the single `metadata` block verbatim
    /// — proving zero-server-change forward compat for any new
    /// frontmatter field an author adds.
    #[test]
    fn list_skills_payload_carries_arbitrary_nested_key_verbatim() {
        let mut cache = SkillsCache::default();
        cache.order.push("plan-hard".to_string());
        cache.skills.insert(
            "plan-hard".to_string(),
            loaded_skill(
                "plan-hard",
                "Plan hard",
                "Deep planning",
                r#"
name: plan-hard
x-vendor-extension:
  nested:
    - one
    - two
  flag: false
"#,
                "/tmp/plan-hard/SKILL.md",
            ),
        );

        let payload = list_skills_payload(&cache);

        let block = &payload["skills"][0]["metadata"];
        assert_eq!(
            block["x-vendor-extension"],
            serde_json::json!({ "nested": ["one", "two"], "flag": false })
        );
    }

    /// The resource `_meta` carries ONE namespaced key
    /// (`io.hyprpilot/skill`) — no `io.hyprpilot/frontmatter`, no bare
    /// `skill` — and the block drops the spec-duplicated
    /// `title`/`description` while keeping a custom frontmatter key +
    /// the runtime `path`/`bundleDir`.
    #[test]
    fn skill_meta_single_key_drops_spec_dupes_keeps_custom_and_runtime() {
        let skill = loaded_skill(
            "plan-hard",
            "Plan hard",
            "Deep planning",
            r#"
name: plan-hard
title: Plan hard
description: Deep planning
license: MIT
metadata:
  owner: captain
  tags:
    - alpha
    - beta
"#,
            "/tmp/plan-hard/SKILL.md",
        );

        let meta = skill_meta(&skill.meta_block);

        // Exactly one namespaced key; the legacy keys are gone.
        assert_eq!(meta.len(), 1);
        assert!(meta.get("io.hyprpilot/frontmatter").is_none());
        assert!(meta.get("skill").is_none());
        let block = meta.get("io.hyprpilot/skill").expect("skill block present");

        // Spec-duplicated keys dropped.
        assert!(block.get("title").is_none());
        assert!(block.get("description").is_none());
        // Frontmatter `name` (NOT the same as Resource.name = slug) kept.
        assert_eq!(block.get("name").and_then(serde_json::Value::as_str), Some("plan-hard"));
        // Custom keys ride through verbatim.
        assert_eq!(block.get("license").and_then(serde_json::Value::as_str), Some("MIT"));
        let nested = block.get("metadata").and_then(serde_json::Value::as_object).unwrap();
        assert_eq!(nested.get("owner").and_then(serde_json::Value::as_str), Some("captain"));
        assert_eq!(
            nested.get("tags").and_then(serde_json::Value::as_array).map(Vec::len),
            Some(2)
        );
        // Runtime-derived keys present.
        assert_eq!(
            block.get("path").and_then(serde_json::Value::as_str),
            Some("/tmp/plan-hard/SKILL.md")
        );
        assert_eq!(
            block.get("bundleDir").and_then(serde_json::Value::as_str),
            Some("/tmp/plan-hard")
        );
    }
}

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
//! - Resources — the catalogue and skill bodies, and NOTHING else
//!   - `hyprpilot://skills` — the catalogue index (markdown)
//!   - `hyprpilot://skills/<slug>` — full SKILL.md body, followed by a
//!     manifest footer naming every reference it declares and the PATH
//!     that fetches each
//!   - Both carry ONE namespaced `_meta` key, `io.hyprpilot/skill`:
//!     the verbatim frontmatter MINUS `title`/`description` (already
//!     carried by the spec `Resource` fields) and MINUS `references`
//!     (superseded by the resolved manifest) PLUS the runtime-derived
//!     `path`, `bundleDir`, `size`, `modified`, `created`. Nothing in
//!     that block repeats a spec-compliant `Resource` field. See
//!     `skills/wire_metadata.rs`.
//!   - There is deliberately NO reference URI. A reference's identity is
//!     its path, not a slug-and-name, so a resource scheme would be a
//!     second address for something the manifest already addresses
//!     better — and enumerating one per reference would have cost more
//!     context than the skills themselves.
//! - Tools
//!   - `list_skills` — `{ skills: [{ slug, title, description, uri,
//!     referenceCount, metadata }] }`. Served purely from cache; it
//!     does NOT resolve references, which would mean reading every
//!     declared file of every skill on every call.
//!   - `read_skill { slug, bundle? }` — `{ uri, body, references: [manifest],
//!     bundle, metadata }`. Reference BODIES are opt-in (`bundle: true`);
//!     the manifest addressing each one always rides along, so declining
//!     a body is never a silent gap.
//!   - `list_skill_references { slug }` — one skill's reference metadata,
//!     no bodies. Each row carries the canonical `path`: the address to
//!     load it, and the identity that says whether you already hold it.
//!   - `read_skill_references { references: [path] }` — bodies by PATH,
//!     validated against the set some skill actually declares. Paths
//!     address files rather than skills, so one call spans skills, a
//!     shared file is fetched once, and repeats collapse.
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
    /// Every canonical path some skill declares — the allow-list a load
    /// request is checked against.
    ///
    /// Built here rather than per request because it is STRUCTURAL: it
    /// changes only when a skill's frontmatter does, which is exactly
    /// what `reload` invalidates. Resolving it per call would mean
    /// canonicalizing every declared path of every skill just to answer
    /// one fetch.
    declared: std::collections::HashSet<String>,
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

    /// Resolve every declared reference: its canonical path, display
    /// name, timestamps, and its own frontmatter.
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
            .map(|dir| wire_references::resolve(dir, &self.refs))
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
             cites. Their bodies are NOT included by default: `read_skill` \
             lists what the skill declares (path, name, size, when it last \
             changed), and you fetch only what the body directs you to with \
             `read_skill_references { references: [\"<path>\", ...] }`, \
             passing the `path` values from that list. Or pass \
             `bundle: true` to `read_skill` for body-plus-everything in one \
             call. References have no URI of their own — a path addresses a \
             FILE, so one call spans skills, and a path you already loaded \
             needs no second fetch even when another skill cites it under a \
             different name. `list_skill_references { slug }` shows a \
             skill's paths without reading any bodies. Bundled references \
             are delimited by a `reference:` YAML block naming each one. \
             Every resource and tool result carries the skill's \
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
        if let Some(dir) = skill.path.parent() {
            cache.declared.extend(wire_references::declared_paths(dir, &refs));
        }
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

fn frontmatter_string(value: &yaml_serde::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(yaml_serde::Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
}

fn skill_uri(slug: &str) -> String {
    format!("hyprpilot://skills/{slug}")
}

fn list_skills_payload(cache: &SkillsCache) -> serde_json::Value {
    let entries: Vec<serde_json::Value> = cache
        .order
        .iter()
        .filter_map(|slug| cache.skills.get(slug))
        .map(|s| {
            // Reference DETAIL is deliberately absent: `list_skills` is
            // the routing view ("which skill?"), it is served purely
            // from cache, and resolving references here would mean
            // reading every declared file of every skill on every call.
            // `list_skill_references` owns that question.
            serde_json::json!({
                "slug": s.slug,
                "title": s.title,
                "description": s.description,
                "uri": skill_uri(&s.slug),
                "referenceCount": s.refs.references.len(),
                "metadata": s.meta_block,
            })
        })
        .collect();

    serde_json::json!({ "skills": entries })
}

/// The whole resource surface: a catalogue index and one body per
/// skill.
///
/// There is deliberately NO reference URI. Reference bodies are reached
/// only through `read_skill_references`, addressed by path — a resource
/// scheme would need a slug-and-name address for something whose real
/// identity is its path, and would duplicate a tool that already does
/// the job with de-duplication built in.
enum ParsedUri<'a> {
    /// The bare `hyprpilot://skills` index. Cannot collide with a slug:
    /// every skill URI carries a `skills/` prefix, and `strip_prefix`
    /// requires the separator.
    Catalogue,
    Skill(&'a str),
}

fn parse_uri(uri: &str) -> Option<ParsedUri<'_>> {
    let rest = uri.strip_prefix("hyprpilot://")?;
    if rest == "skills" {
        return Some(ParsedUri::Catalogue);
    }
    rest.strip_prefix("skills/")
        .filter(|slug| !slug.is_empty())
        .map(ParsedUri::Skill)
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

/// `list_skill_references`'s schema — a REQUIRED `slug`.
///
/// Required because the alternative was a whole-catalogue scan, and on
/// a real root that is a six-figure payload — the single largest thing
/// this server could hand a client. Per-skill listing answers the same
/// question incrementally: each row carries the canonical `path`, so a
/// caller compares against what it already loaded rather than needing
/// the corpus up front.
fn list_references_object_schema() -> Arc<serde_json::Map<String, serde_json::Value>> {
    object_schema(serde_json::json!({ "slug": slug_prop() }))
}

/// `read_skill_references`'s schema — an ARRAY of canonical paths.
///
/// Paths rather than slug-plus-name because a path is what a reference
/// IS, while a slug and a name are one of several addresses for it.
/// Addressing by path means a file cited by many skills is one entry
/// rather than many, a caller can fetch across skills in one call, and
/// there is no collision or shadowing rule to explain.
fn read_references_object_schema() -> Arc<serde_json::Map<String, serde_json::Value>> {
    let serde_json::Value::Object(map) = serde_json::json!({
        "type": "object",
        "properties": {
            "references": {
                "type": "array",
                "items": { "type": "string" },
                "description":
                    "Canonical paths to fetch, exactly as they appear as `path` in a \
                     skill's reference manifest. A path no skill declares is an error \
                     rather than a partial result.",
            },
        },
        "required": ["references"],
        "additionalProperties": false,
    }) else {
        unreachable!("json! object literal")
    };
    Arc::new(map)
}

/// `read_skill`'s schema — `slug`, plus an opt-IN for the full bundle.
///
/// Bundling defaults OFF. The body always carries a manifest of what the
/// skill declares — path, name, size, mtime — so the agent can see what
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
         instruction set. It ends with a list of the references that skill declares: each one's \
         PATH and name, but not its body.\n\n\
         Reference bodies have no URI of their own. Pass the paths from that list to \
         `read_skill_references` — one call takes as many as you need, and a path is a file, so \
         references from several skills come back together. A path is also an IDENTITY: the same \
         shared file is cited by many skills under different names, so a path you already loaded \
         needs no second fetch. `list_skill_references { slug }` shows a skill's paths without \
         reading any bodies.\n\n\
         So the chain is: pick a slug here → read `skills/<slug>` → follow the reference directives \
         in its body, loading only the paths those steps actually name. The `reload` tool rescans \
         the roots if this index looks stale.\n\n",
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
                " · {} reference(s) — `list_skill_references {{ slug: \"{slug}\" }}`",
                skill.refs.references.len()
            ));
        }
        out.push_str("\n\n");
    }

    out
}

/// Text projection for `list_skill_references`.
///
/// Leads with the files cited by MORE THAN ONE skill, because that is
/// the question the tool exists to answer: a caller that already holds
/// `output-diff` from one skill should be able to see, in one glance,
/// that another skill's citation is the same file rather than a second
/// one to fetch.
fn list_references_summary(slug: &str, entries: &[ReferenceEntry]) -> String {
    if entries.is_empty() {
        return format!("`{slug}` declares no references.");
    }
    let mut out = format!("{slug} declares {} reference(s):\n", entries.len());
    for entry in entries {
        let size = entry.stat.size.unwrap_or_default();
        let modified = entry.stat.modified.as_deref().unwrap_or("unknown");
        match &entry.path {
            Some(path) => out.push_str(&format!(
                "  {path}\n    name: {} ({size} bytes, modified {modified})\n",
                entry.name
            )),
            None => out.push_str(&format!("  [NOT FOUND] name: {}\n", entry.name)),
        }
    }
    out.push_str(
        "\nBodies are NOT included. Pass the paths above to `read_skill_references`. \
         The path is also the identity: the same shared file is cited by many skills \
         under different names, so a path you already loaded needs no second fetch.",
    );
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
                     `read_skill_references`, or pass `bundle: true` to get them all in one \
                     call. Equivalent to reading the `hyprpilot://skills/<slug>` resource."
                        .into(),
                ),
                read_skill_object_schema(),
            ),
            Tool::new_with_raw(
                "list_skill_references",
                Some(
                    "List one skill's reference METADATA without any bodies - canonical \
                     path, name, size and when each last changed. Use it to see what a \
                     skill cites before spending tokens on it. The `path` is both the \
                     identity and the address: pass it to `read_skill_references` to get \
                     the body, and compare it against paths you already loaded, since the \
                     same shared file is cited by many skills under different names."
                        .into(),
                ),
                list_references_object_schema(),
            ),
            Tool::new_with_raw(
                "read_skill_references",
                Some(
                    "Fetch reference bodies by PATH. Pass the `path` values from a skill's \
                     reference manifest - `read_skill` and `list_skill_references` both \
                     return them. Paths address files, not skills, so one call fetches \
                     references from as many skills as you like, a file cited by several \
                     skills is fetched once, and a repeated path is served once. Only paths \
                     some skill actually declares are served."
                        .into(),
                ),
                read_references_object_schema(),
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
                    let bundle = wire_references::bundle(&entries);
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
                            .then(|| wire_references::bundle(&entries)),
                        "metadata": skill.meta_block,
                    }),
                ))
            }
            "list_skill_references" => {
                let slug = require_string(&args, "slug")?;
                let cache = self.skills_cache.read().await;
                let Some(skill) = cache.skills.get(slug) else {
                    return Ok(tool_error(format!("unknown skill: {slug}")));
                };
                let entries = skill.references();
                Ok(structured_with_text(
                    list_references_summary(slug, &entries),
                    serde_json::json!({
                        "slug": slug,
                        "references": wire_references::manifest(&entries),
                    }),
                ))
            }
            "read_skill_references" => {
                let Some(serde_json::Value::Array(items)) = args.get("references") else {
                    return Ok(tool_error(
                        "`references` is required and must be an array of canonical paths, \
                         as listed in a skill's reference manifest",
                    ));
                };
                let cache = self.skills_cache.read().await;
                // Validate against the set of paths some skill actually
                // declares, built once per reload. A caller-supplied
                // path is CHECKED, never joined — so this reaches
                // exactly the files the skills already reference and no
                // others. Canonicalizing first means a caller may pass
                // any spelling of a declared file.
                let mut paths = Vec::with_capacity(items.len());
                let mut unknown = Vec::new();
                for item in items {
                    let Some(raw) = item.as_str() else {
                        return Ok(tool_error("`references` must be an array of strings"));
                    };
                    match wire_references::canonical(raw).filter(|p| cache.declared.contains(p)) {
                        // Repeats are collapsed: a caller assembling a
                        // selection across several steps of a skill, or
                        // across skills that share a file, must not
                        // amplify its own response.
                        Some(path) if !paths.contains(&path) => paths.push(path),
                        Some(_) => {}
                        None => unknown.push(raw.to_string()),
                    }
                }
                if !unknown.is_empty() {
                    return Ok(tool_error(format!(
                        "no skill declares {}. Pass the `path` values from a skill's \
                         reference manifest (`read_skill` or `list_skill_references`).",
                        unknown.iter().map(|p| format!("`{p}`")).collect::<Vec<_>>().join(", ")
                    )));
                }
                let entries = wire_references::resolve_paths(&paths);
                let body = wire_references::bundle(&entries);
                Ok(structured_with_text(
                    body.clone(),
                    serde_json::json!({
                        "body": body,
                        "references": wire_references::manifest(&entries),
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
                     `hyprpilot://skills/<slug>` for the body, then pass the paths it lists to \
                     `read_skill_references` for the files it declares. {} skill(s).",
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
            // References are deliberately absent from this listing —
            // and from the resource surface entirely. There is no
            // reference URI to enumerate: a reference is addressed by
            // its path through `read_skill_references`.
            //
            // This listing is the single most expensive thing this
            // server can hand a client: measured against a 127-skill
            // catalogue it was 231 resources / ~170 KB, of which 48% was
            // `_meta` — and the bundle resource's `_meta` was its own
            // skill's block repeated verbatim, paying twice for one
            // skill's metadata. Enumerating every individual reference
            // on top would have reached ~710 entries and ~520 KB, which
            // is most of a context window spent before a single skill is
            // read. A template costs one entry regardless of catalogue
            // size, and `list_skill_references` answers "what does this
            // skill cite" far more cheaply than a listing can.
        }
        Ok(ListResourcesResult::with_all_items(resources))
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, rmcp::ErrorData> {
        let templates = vec![rmcp::model::ResourceTemplate::new("hyprpilot://skills/{slug}", "skill")
            .with_description("Full SKILL.md body for the addressed skill slug.")
            .with_mime_type("text/markdown")];
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
        assert!(parse_uri("hyprpilot://skillsfoo").is_none());
        assert!(parse_uri("hyprpilot://nope").is_none());
    }

    /// The index must point at the tool that loads references, because
    /// there is no reference URI to chain into any more.
    #[test]
    fn the_catalogue_explains_how_to_load_what_it_lists() {
        let empty = SkillsCache::default();
        let out = catalogue_markdown(&empty);
        assert!(out.contains("hyprpilot://skills/<slug>"), "must name the body scheme");
        assert!(
            out.contains("read_skill_references"),
            "must name how reference bodies are reached"
        );
        assert!(
            !out.contains("hyprpilot://references"),
            "the references scheme is gone and must not be advertised"
        );
        assert!(out.contains("No skills available"), "an empty catalogue still renders");
    }

    /// The resource surface is exactly the catalogue and skill bodies.
    /// Every former reference URI now addresses nothing — a stale client
    /// must get a clean "unrecognised uri" rather than a body.
    #[test]
    fn the_reference_uri_scheme_no_longer_resolves() {
        for gone in [
            "hyprpilot://references/git-commit",
            "hyprpilot://references/git-commit/output-diff",
            "hyprpilot://references/git-commit/",
            "hyprpilot://references",
        ] {
            assert!(parse_uri(gone).is_none(), "{gone} must not parse");
        }
    }

    #[test]
    fn parses_known_uris() {
        assert!(matches!(
            parse_uri("hyprpilot://skills/foo"),
            Some(ParsedUri::Skill("foo"))
        ));
        // A slug is a single segment, but parsing does not enforce that
        // — an unknown slug simply resolves to no skill.
        assert!(matches!(
            parse_uri("hyprpilot://skills/foo/references"),
            Some(ParsedUri::Skill("foo/references"))
        ));
        // A bare trailing slash addresses nothing.
        assert!(parse_uri("hyprpilot://skills/").is_none());
        assert!(parse_uri("hyprpilot://unknown/x").is_none());
        assert!(parse_uri("not-our-scheme://x").is_none());
    }

    fn loaded_skill(slug: &str, title: &str, description: &str, frontmatter_yaml: &str, path: &str) -> LoadedSkill {
        let frontmatter: yaml_serde::Value = yaml_serde::from_str(frontmatter_yaml).unwrap();
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
        let frontmatter: yaml_serde::Value = yaml_serde::from_str("name: myskill\n").unwrap();
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

        let frontmatter: yaml_serde::Value =
            yaml_serde::from_str("name: myskill\nreferences:\n  - ./references/local.md\n").unwrap();
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
        let address = entries[0].path.clone().expect("a readable reference has an address");

        // The declared path must land in the cache's allow-list, or the
        // address the manifest publishes would be refused by the very
        // call it is meant to feed.
        assert!(cache.declared.contains(&address));

        // Default: the address, not the body.
        let footer = wire_references::manifest_footer(&entries, "myskill");
        assert!(footer.contains(&format!("path: {address}")));
        assert!(
            !footer.contains("local body"),
            "the default must not carry reference bodies"
        );

        // Opt-in: the body.
        let bundled = wire_references::bundle(&entries);
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
        let frontmatter: yaml_serde::Value = yaml_serde::from_str(
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
                    // A count, not the names: this view is served
                    // purely from cache, and resolving names would mean
                    // reading every reference of every skill per call.
                    "referenceCount": 1,
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

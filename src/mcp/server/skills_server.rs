//! `hyprpilot mcp skills` — the rmcp-backed skills MCP server.
//!
//! Spawned by the agent vendor (via stdio) when the launcher
//! auto-injects the `hyprpilot-skills` server entry into the vendor's
//! MCP catalog. The sidecar reads skills by SCANNING DIRECTORIES directly
//! — the same discovery logic the launcher's `SkillsRegistry` uses —
//! so adding a new skill to a configured directory is picked up without
//! restarting the session, and the launcher doesn't have to enumerate
//! individual files when building the spawn command.
//!
//! Every root is WATCHED (`crate::watch`, debounced). A change rescans
//! and announces itself; `reload` forces the same rescan for the cases
//! a watch cannot cover — a root the watcher reports degraded or off,
//! or a reference file outside every root.
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
//!   - `reload` — force a rescan. The watcher does this on its own for
//!     any change under a root, and BOTH callers announce through one
//!     `announce()`, so they cannot disagree about a delta. Skills back
//!     the resource list; the tool list is fixed for a given process,
//!     so no tool-list-changed fires
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

use clap::Args;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, ErrorCode, Implementation, ListResourceTemplatesResult,
    ListResourcesResult, ListToolsResult, PaginatedRequestParams, ReadResourceRequestParams, ReadResourceResponse,
    ReadResourceResult, ResourceContents, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::ServerHandler;
use tokio::sync::RwLock;

use crate::config::mcp::DEFAULT_SKILLS_SERVER_NAME;
use crate::config::ResolvedSkillEntry;
use crate::mcp::skills::SkillsRegistry;

/// The receiver half `arm_watch` hands the relay.
type WatchSignals = tokio::sync::mpsc::UnboundedReceiver<crate::watch::WatchSignal>;

use super::rpc::{
    empty_object_schema, require_string, structured_with_text, tool_error, wait_for_shutdown, RESULT_CACHE_SCOPE,
    RESULT_TTL_MS,
};
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
    /// Defaults ON, so a hand-written MCP catalogue entry carrying the
    /// pre-watcher JSON shape still gets a watched root.
    #[serde(default = "watch_default")]
    pub watch: bool,
}

fn watch_default() -> bool {
    true
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
    // Armed BEFORE the startup scan, so an edit landing between the scan
    // and the first drain is queued rather than lost.
    let (watcher, signals) = handler.arm_watch(crate::watch::DEBOUNCE).await;

    // Startup scan. Nothing is connected yet, so the delta has no
    // one to notify — discard it deliberately rather than by accident.
    let _ = handler.reload_skills().await;

    // Cloned before serving consumes the handler — Arcs only, the same
    // shape the harness uses to keep a handle on its session table.
    let relay_server = handler.clone();

    let (stdin, stdout) = rmcp::transport::io::stdio();
    let running = super::rpc::serve_from_first_byte(handler, (stdin, stdout));

    // The peer exists only once the service is running, which is also
    // the earliest a notification could reach anyone — so this ordering
    // is correct, not merely convenient.
    let relay = tokio::spawn(relay_server.relay_watch(signals, running.peer().clone()));

    // Race the transport against SIGTERM/SIGHUP. Without this a
    // supervisor stopping the sidecar would skip every destructor and
    // strand live sessions — `PR_SET_PDEATHSIG` still covers the
    // SIGKILL case, but only after the kernel notices, and it cannot
    // remove the session directories.
    wait_for_shutdown(running).await;

    // Stop notifying before the transport is gone, then release the
    // watcher. `Watcher`'s drop only sets the debouncer's stop flag, so
    // teardown never blocks on that thread.
    relay.abort();
    drop(watcher);

    Ok(())
}

// ── In-memory cache ───────────────────────────────────────────────────

#[derive(Debug, Default)]
struct SkillsCache {
    skills: std::collections::HashMap<String, LoadedSkill>,
    order: Vec<String>,
    /// Every canonical path some skill declares, with the skills citing
    /// it and the fingerprint the manifest serves for it.
    ///
    /// The allow-list half is STRUCTURAL: it changes only when a
    /// skill's frontmatter does, and resolving it per call would mean
    /// canonicalizing every declared path of every skill just to answer
    /// one fetch.
    ///
    /// The fingerprint half is what lets a rescan tell a reference edit
    /// from silence. Bodies stay uncached deliberately — they resolve
    /// per call so `modified` is always live — but `modified` is a
    /// SERVED manifest field, so a fingerprint change IS a change in
    /// served content. That makes the diff exact rather than a
    /// heuristic, at one `metadata()` per unique declared file per
    /// rescan.
    declared: std::collections::HashMap<String, DeclaredReference>,
}

/// One declared reference path, as the cache remembers it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DeclaredReference {
    /// Slugs citing this path, in catalogue order. A file 60 skills
    /// share is ONE entry with 60 citers, which is what makes a shared
    /// convention's edit cost one stat rather than 60.
    citers: Vec<String>,
    stat: crate::mcp::skills::wire_time::FileStat,
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
    /// The client's `subscriptions/listen` stream, when it opened one.
    subscriptions: super::rpc::Subscriptions,
    /// Orders rescans against each other.
    ///
    /// rmcp runs every request in its own task, so two `reload` calls
    /// could already interleave as scan A, scan B, swap B, swap A —
    /// regressing the cache and diffing A against B's state. The
    /// watcher makes that ordinary rather than rare. The cache's own
    /// `RwLock` still serves readers; this only orders writers.
    reload_gate: Arc<tokio::sync::Mutex<()>>,
    /// Per-root watch coverage, so a caller can tell whether it needs
    /// `reload` at all.
    watch_status: Arc<RwLock<crate::watch::WatchStatus>>,
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
                    watch: entry.watch,
                }
            })
            .collect();

        Ok(Self {
            registry: Arc::new(SkillsRegistry::new(entries)),
            skills_cache: Arc::new(RwLock::new(SkillsCache::default())),
            subscriptions: super::rpc::Subscriptions::default(),
            reload_gate: Arc::new(tokio::sync::Mutex::new(())),
            watch_status: Arc::new(RwLock::new(crate::watch::WatchStatus::default())),
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
             `path`, `bundleDir`, `size`, `modified` and `created`. \
             Skill roots are WATCHED: an edit is rescanned and announced as \
             `resources/updated` (per affected skill) plus \
             `resources/list_changed`, so you never need `reload` unless \
             `list_skills` reports a root degraded or off.",
        )
    }

    /// Arm the watcher over every configured root and record what it
    /// covers.
    ///
    /// Called BEFORE the startup scan: an edit landing between the scan
    /// and the first drain is then queued rather than lost. The channel
    /// is unbounded and nothing reads it yet.
    async fn arm_watch(&self, debounce: std::time::Duration) -> (Option<crate::watch::Watcher>, WatchSignals) {
        let roots: Vec<crate::watch::WatchRoot> = self
            .registry
            .dirs()
            .iter()
            .map(|entry| crate::watch::WatchRoot {
                dir: entry.dir.clone(),
                ignore: entry.ignore.clone(),
                watch: entry.watch,
            })
            .collect();
        let armed = crate::watch::arm(&roots, debounce);
        *self.watch_status.write().await = armed.status;
        (armed.watcher, armed.signals)
    }

    /// Fire what one rescan invalidated.
    ///
    /// The single notification path. Both callers — the `reload` tool
    /// and the watcher relay — reach the wire only through here, so the
    /// two cannot drift into announcing different things for the same
    /// delta.
    async fn announce(&self, peer: &rmcp::service::Peer<RoleServer>, delta: &CatalogueDelta) {
        let plan = delta.plan();
        if plan.list_changed {
            self.subscriptions.resource_list_changed(peer).await;
        }
        self.subscriptions.resources_updated(peer, plan.updated).await;
    }

    /// Turn watch signals into rescans for as long as the transport
    /// lives.
    ///
    /// Never an opener and never on a request's path, so it cannot
    /// reintroduce the pre-loop deadlock `serve_from_first_byte` exists
    /// to avoid — the serve loop is already spawned when this starts.
    async fn relay_watch(self, mut signals: WatchSignals, peer: rmcp::service::Peer<RoleServer>) {
        while let Some(first) = signals.recv().await {
            // Drain the burst before doing any work: a `git checkout`
            // that outlasts the debounce window still costs one rescan
            // per quiet window, and no signal is skipped.
            let mut degraded = first.degraded().map(str::to_string);
            while let Ok(more) = signals.try_recv() {
                degraded = degraded.or_else(|| more.degraded().map(str::to_string));
            }
            if let Some(reason) = degraded {
                self.watch_status.write().await.degrade_all(&reason);
            }
            let delta = self.reload_skills().await;
            if delta.is_empty() {
                // Editor temp files and `git` internals reach here and
                // diff to nothing. Free on the wire, which is why the
                // filter does not try to guess them by name.
                tracing::debug!("mcp::server: watched change rescanned — no catalogue change");
                continue;
            }
            tracing::info!(
                membership_changed = delta.membership_changed,
                updated = delta.updated.len(),
                references_changed = delta.references_changed.len(),
                reference_citers = delta.reference_citers.len(),
                "mcp::server: skills rescanned from a watched change"
            );
            self.announce(&peer, &delta).await;
        }
        // The sender dropped, so the watcher thread is gone. Say so once
        // rather than reporting coverage that no longer exists.
        self.watch_status.write().await.degrade_all("watcher thread exited");
        tracing::warn!("mcp::server: skills watcher stopped — `reload` is the only refresh now");
    }

    /// Rescan disk and report what changed, so the caller can fire the
    /// notification that matches. Returns an empty delta when the reload
    /// failed — a failed rescan leaves the cache untouched, so claiming
    /// anything changed would invalidate a client's cache for nothing.
    async fn reload_skills(&self) -> CatalogueDelta {
        let _ordered = self.reload_gate.lock().await;
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
                return CatalogueDelta::default();
            }
            Err(err) => {
                tracing::error!(%err, "mcp::server: blocking reload join failed");
                return CatalogueDelta::default();
            }
        };

        let mut cache = self.skills_cache.write().await;
        let next = build_cache(skills);
        let delta = CatalogueDelta::between(&cache, &next);
        *cache = next;
        delta
    }
}

/// What a `reload` actually changed, so the right notification fires for
/// the right URI.
///
/// With `ttlMs` effectively indefinite (`rpc::RESULT_TTL_MS`), a client
/// re-fetches only when told to. `resources/list_changed` covers a skill
/// appearing or disappearing; it says nothing about a body that changed
/// under an unchanged slug, which is the common edit and the one
/// `hyprpilot-reload` exists to catch. That needs a per-URI
/// `resources/updated`.
#[derive(Debug, Default, PartialEq)]
struct CatalogueDelta {
    /// Slugs added or removed — membership, so the LIST changed.
    membership_changed: bool,
    /// Slugs whose body or metadata differs from the previous scan.
    updated: Vec<String>,
    /// Canonical paths whose fingerprint changed, appeared, or vanished.
    /// Reported so a caller knows which files to re-fetch, by the
    /// address it already uses.
    references_changed: Vec<String>,
    /// Slugs citing any changed path, minus any already in `updated`.
    /// Their BODIES are unchanged; their manifests and footers are not,
    /// and both are served text.
    reference_citers: Vec<String>,
}

impl CatalogueDelta {
    fn between(before: &SkillsCache, after: &SkillsCache) -> Self {
        let updated: Vec<String> = after
            .order
            .iter()
            .filter(|slug| {
                // Compare the body AND the metadata block: an edit that
                // only touches frontmatter leaves the body identical but
                // still changes what every surface reports.
                match (before.skills.get(*slug), after.skills.get(*slug)) {
                    (Some(old), Some(new)) => old.body != new.body || old.meta_block != new.meta_block,
                    _ => false,
                }
            })
            .cloned()
            .collect();

        let (references_changed, reference_citers) = Self::references_between(before, after, &updated);

        Self {
            membership_changed: before.order != after.order,
            updated,
            references_changed,
            reference_citers,
        }
    }

    /// Which declared files moved, and which surviving skills cite them.
    ///
    /// A path on ONE side only counts: a declared file that could not be
    /// canonicalized is absent from `declared` entirely, so a reference
    /// appearing flips its citer's manifest row from `status: not-found`
    /// to a real path — a served change with no body edit behind it.
    fn references_between(before: &SkillsCache, after: &SkillsCache, updated: &[String]) -> (Vec<String>, Vec<String>) {
        let mut changed = Vec::new();
        let mut citers = Vec::new();

        for (path, entry) in &after.declared {
            let moved = match before.declared.get(path) {
                Some(old) => old.stat != entry.stat,
                None => true,
            };
            if moved {
                changed.push(path.clone());
            }
        }
        // Vanished paths: a citer that survives now serves a
        // `status: not-found` row where it served a real one.
        for path in before.declared.keys() {
            if !after.declared.contains_key(path) {
                changed.push(path.clone());
            }
        }
        changed.sort_unstable();

        for path in &changed {
            let entry = after.declared.get(path).or_else(|| before.declared.get(path));
            for slug in entry.map(|e| e.citers.as_slice()).unwrap_or_default() {
                // Only skills that still exist, and only once. A slug
                // already in `updated` gets its notification from there.
                if after.skills.contains_key(slug) && !updated.contains(slug) && !citers.contains(slug) {
                    citers.push(slug.clone());
                }
            }
        }
        // Catalogue order, so the announcement is stable across rescans.
        citers.sort_by_key(|slug| after.order.iter().position(|s| s == slug));

        (changed, citers)
    }

    fn is_empty(&self) -> bool {
        !self.membership_changed && self.updated.is_empty() && self.references_changed.is_empty()
    }

    /// The URIs a rescan invalidates, decided in one pure place so the
    /// watcher and the `reload` tool cannot drift apart.
    fn plan(&self) -> Announcement {
        let mut updated: Vec<String> = self.updated.iter().map(|slug| skill_uri(slug)).collect();
        // A citer's served text (body plus manifest footer) changed even
        // though its body did not. A subscriber holding that one skill
        // has no other way to learn it.
        updated.extend(self.reference_citers.iter().map(|slug| skill_uri(slug)));
        // The index renders slug, title, description and reference
        // COUNT — never a reference's own content. Firing it for a
        // reference edit would be exactly the spurious invalidation the
        // diff exists to prevent.
        if self.membership_changed || !self.updated.is_empty() {
            updated.push(catalogue_uri());
        }
        Announcement {
            // Anything at all. A pre-`2026-07-28` client cannot
            // subscribe, so `resources/updated` is not a signal it can
            // act on; `list_changed` is the only one it has, and a
            // reference edit must reach it as something rather than
            // silence.
            list_changed: !self.is_empty(),
            updated,
        }
    }
}

/// What one rescan tells connected clients.
#[derive(Debug, Default, PartialEq)]
struct Announcement {
    list_changed: bool,
    updated: Vec<String>,
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
            for path in wire_references::declared_paths(dir, &refs) {
                cache
                    .declared
                    .entry(path)
                    .or_insert_with_key(|path| DeclaredReference {
                        citers: Vec::new(),
                        stat: crate::mcp::skills::wire_time::FileStat::read(std::path::Path::new(path)),
                    })
                    .citers
                    .push(slug.clone());
            }
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

/// Root of every URI this server serves — the catalogue index itself
/// and, with a `/<slug>` suffix, each skill body.
const SKILLS_URI_ROOT: &str = "hyprpilot://skills";

/// The catalogue index resource. It renders every skill's slug, title
/// and description, so ANY skill change makes it stale.
fn catalogue_uri() -> String {
    SKILLS_URI_ROOT.to_string()
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
         in its body, loading only the paths those steps actually name. The roots are watched, so \
         this index is kept current; `reload` forces a rescan if a root is reported \
         unwatched.\n\n",
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

/// Watch coverage as the tools report it. `active` is true only when
/// every root is covered, so a client reading it can stop checking.
fn watch_payload(status: &crate::watch::WatchStatus) -> serde_json::Value {
    serde_json::json!({
        "active": status.active(),
        "roots": status.roots,
    })
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
        // Per-resource subscriptions are how a client learns that ONE
        // skill body changed rather than re-reading the catalogue. It is
        // what makes the indefinite `ttlMs` safe — see
        // `rpc::RESULT_TTL_MS` — so it is advertised, and
        // `accepted_subscription_filter` below is what actually accepts
        // the opt-in at `2026-07-28`.
        resources.subscribe = Some(true);
        resources.list_changed = Some(true);
        caps.resources = Some(resources);
        ServerInfo::new(caps)
            .with_server_info(Implementation::new(
                DEFAULT_SKILLS_SERVER_NAME.to_string(),
                env!("CARGO_PKG_VERSION").to_string(),
            ))
            .with_instructions(self.instructions())
    }

    /// Record the negotiated protocol version as the peer's, per
    /// `rpc::initialize_negotiated`.
    async fn initialize(
        &self,
        request: rmcp::model::InitializeRequestParams,
        context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> Result<rmcp::model::InitializeResult, rmcp::ErrorData> {
        Ok(super::rpc::initialize_negotiated(self, request, &context))
    }

    /// Accept the `subscriptions/listen` opt-in at `2026-07-28`.
    ///
    /// rmcp leaves this `None` — subscriptions unimplemented — so
    /// without it a client on the current revision has NO channel for
    /// the notifications this server already emits, and the indefinite
    /// `ttlMs` would have nothing to invalidate it.
    ///
    /// The SDK intersects what we return with both the request and the
    /// capabilities advertised above, so echoing the two categories we
    /// actually fire is enough; a client asking for
    /// `toolsListChanged` gets it dropped, correctly, because the tool
    /// set cannot change.
    fn accepted_subscription_filter(
        &self,
        requested: &rmcp::model::SubscriptionFilter,
    ) -> Option<rmcp::model::SubscriptionFilter> {
        // `hyprpilot://skills` (the catalogue index) and
        // `hyprpilot://skills/<slug>` are the only URIs this server ever
        // fires for.
        // Delegates to the same parser `read_resource` uses, so an
        // acknowledged URI is by construction one this server can serve
        // and fire for. Re-implementing the match here is how
        // `hyprpilot://skillsfoo` — and an empty slug — got acknowledged
        // and then never fired, which is the "waiting forever" contract
        // this filter exists to close.
        super::rpc::accept_resource_subscriptions(requested, |uri| parse_uri(uri).is_some())
    }

    /// Hold the subscription stream open so notifications can ride it.
    async fn listen(&self, context: rmcp::service::SubscriptionContext) -> Result<(), rmcp::ErrorData> {
        self.subscriptions.run(context).await;
        Ok(())
    }

    /// Legacy `resources/subscribe`, honoured so `resources.subscribe:
    /// true` is truthful at every revision we negotiate. Records
    /// nothing: a peer with no `subscriptions/listen` stream already
    /// receives these notifications as broadcasts. rmcp's default
    /// answers `-32601`, which would make the capability a lie.
    #[allow(deprecated)]
    async fn subscribe(
        &self,
        _request: rmcp::model::SubscribeRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<(), rmcp::ErrorData> {
        Ok(())
    }

    #[allow(deprecated)]
    async fn unsubscribe(
        &self,
        _request: rmcp::model::UnsubscribeRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<(), rmcp::ErrorData> {
        Ok(())
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
                    "Force a rescan of every skill directory. The roots are WATCHED, so \
                     an edit is rescanned and announced on its own - call this only when \
                     `list_skills` reports a root degraded or off, or after editing a \
                     reference file that lives outside every configured root."
                        .into(),
                ),
                empty_object_schema(),
            ),
        ];
        Ok(ListToolsResult::with_all_items(tools)
            .with_ttl_ms(RESULT_TTL_MS)
            .with_cache_scope(RESULT_CACHE_SCOPE))
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
                let watch = self.watch_status.read().await;
                let mut payload = list_skills_payload(&cache);
                if let Some(map) = payload.as_object_mut() {
                    map.insert("watch".into(), watch_payload(&watch));
                }
                let mut summary = list_skills_summary(&cache);
                // Appended ONLY when coverage is partial: a text-only
                // client (opencode renders `content`, never
                // `structured_content`) would otherwise never learn it
                // needs `reload`, and an untroubled session pays
                // nothing for the check.
                if let Some(line) = watch.summary_line() {
                    summary.push_str(&format!("\n{line} Call `reload` after editing files under it."));
                }
                Ok(structured_with_text(summary, payload))
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
                    match wire_references::canonical(raw).filter(|p| cache.declared.contains_key(p)) {
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
                let delta = self.reload_skills().await;
                let count = self.skills_cache.read().await.skills.len();
                if delta.is_empty() {
                    // Nothing moved, so nothing is invalidated. Firing
                    // anyway would cost every subscriber a full re-fetch
                    // for a no-op reload.
                    tracing::debug!(count, "mcp::server: skills reloaded — no change");
                }
                // The SAME path the watcher relay takes. `ttlMs` is
                // effectively indefinite, so a client re-reads only when
                // told to — which makes this the whole invalidation
                // story rather than a nicety, and makes one shared
                // notification path the only way the two callers cannot
                // disagree about a delta.
                self.announce(&context.peer, &delta).await;
                tracing::info!(
                    count,
                    membership_changed = delta.membership_changed,
                    updated = delta.updated.len(),
                    references_changed = delta.references_changed.len(),
                    "mcp::server: skills reloaded"
                );
                let watch = self.watch_status.read().await;
                Ok(structured_with_text(
                    format!("Reloaded {count} skill(s)."),
                    serde_json::json!({
                        "reloaded": count,
                        "membershipChanged": delta.membership_changed,
                        "updated": delta.updated,
                        // Paths, by the address `read_skill_references`
                        // already takes — so a caller holding a stale
                        // reference body knows exactly what to re-fetch
                        // without re-deriving it from a manifest.
                        "referencesChanged": delta.references_changed,
                        "watch": watch_payload(&watch),
                    }),
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
        Ok(ListResourcesResult::with_all_items(resources)
            .with_ttl_ms(RESULT_TTL_MS)
            .with_cache_scope(RESULT_CACHE_SCOPE))
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, rmcp::ErrorData> {
        let templates = vec![rmcp::model::ResourceTemplate::new("hyprpilot://skills/{slug}", "skill")
            .with_description("Full SKILL.md body for the addressed skill slug.")
            .with_mime_type("text/markdown")];
        Ok(ListResourceTemplatesResult::with_all_items(templates)
            .with_ttl_ms(RESULT_TTL_MS)
            .with_cache_scope(RESULT_CACHE_SCOPE))
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
                .with_ttl_ms(RESULT_TTL_MS)
                .with_cache_scope(RESULT_CACHE_SCOPE)
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
                .with_ttl_ms(RESULT_TTL_MS)
                .with_cache_scope(RESULT_CACHE_SCOPE)
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

    /// The acknowledgment is a promise to notify, so it must accept
    /// exactly the URIs this server can fire for. A prefix match with no
    /// separator accepted `hyprpilot://skillsfoo`; an empty slug
    /// addresses nothing.
    #[test]
    fn only_addressable_skill_uris_are_acknowledged() {
        let ok = |uri: &str| parse_uri(uri).is_some();

        assert!(ok("hyprpilot://skills"), "the catalogue index is fireable");
        assert!(ok("hyprpilot://skills/git-commit"));

        assert!(!ok("hyprpilot://skillsfoo"), "a missing separator is not a slug");
        assert!(!ok("hyprpilot://skills/"), "an empty slug addresses nothing");
        assert!(!ok("hyprpilot://sessions/abc"), "another server's scheme");
        assert!(!ok("file:///etc/passwd"));
    }

    /// Build a cache from `(slug, body)` pairs — the delta is about
    /// bodies and membership, so that is all these fixtures need.
    fn cache_of(entries: &[(&str, &str)]) -> SkillsCache {
        let mut cache = SkillsCache::default();
        for (slug, body) in entries {
            let mut skill = loaded_skill(slug, slug, "d", "name: x\n", &format!("/tmp/{slug}/SKILL.md"));
            skill.body = (*body).to_string();
            cache.order.push((*slug).to_string());
            cache.skills.insert((*slug).to_string(), skill);
        }
        cache
    }

    /// A hand-written MCP catalogue entry predates the flag and must
    /// still get a watched root - the sidecar is reachable without a
    /// config to consult, so the JSON shape has to tolerate its own
    /// history.
    #[test]
    fn a_skill_dir_arg_without_watch_defaults_on() {
        let entry: SkillDirEntry = serde_json::from_str(r#"{"dir":"/skills","ignore":[]}"#).expect("parses");
        assert!(entry.watch);
        let off: SkillDirEntry =
            serde_json::from_str(r#"{"dir":"/skills","ignore":[],"watch":false}"#).expect("parses");
        assert!(!off.watch);
    }

    /// Give `slug` a declared reference at `path` with fingerprint
    /// `stat`. The delta compares fingerprints, so a test moves a
    /// reference by moving this and nothing else.
    fn cite(cache: &mut SkillsCache, path: &str, stat: &str, citers: &[&str]) {
        cache.declared.insert(
            path.to_string(),
            DeclaredReference {
                citers: citers.iter().map(|s| (*s).to_string()).collect(),
                stat: crate::mcp::skills::wire_time::FileStat {
                    size: Some(1),
                    modified: Some(stat.to_string()),
                    created: None,
                },
            },
        );
    }

    /// THE reference-gap pin. A shared convention file changes; no
    /// skill body moved, so the old delta reported nothing and every
    /// citing skill went stale in a client's cache for the full ttl.
    /// `modified` is a served manifest field, so a fingerprint change IS
    /// a change in served content.
    #[test]
    fn a_reference_edit_updates_every_skill_that_cites_it() {
        let mut before = cache_of(&[("alpha", "same"), ("beta", "same")]);
        cite(&mut before, "/refs/output-diff.md", "t1", &["alpha", "beta"]);
        let mut after = cache_of(&[("alpha", "same"), ("beta", "same")]);
        cite(&mut after, "/refs/output-diff.md", "t2", &["alpha", "beta"]);

        let delta = CatalogueDelta::between(&before, &after);
        assert!(!delta.is_empty());
        assert!(delta.updated.is_empty(), "no body moved");
        assert!(!delta.membership_changed);
        assert_eq!(delta.references_changed, vec!["/refs/output-diff.md".to_string()]);
        assert_eq!(delta.reference_citers, vec!["alpha".to_string(), "beta".to_string()]);
    }

    /// The index renders slug, title, description and reference COUNT —
    /// never a reference's content. Firing it here would be exactly the
    /// spurious invalidation the diff exists to prevent.
    #[test]
    fn a_reference_edit_does_not_stale_the_catalogue_index() {
        let mut before = cache_of(&[("alpha", "same")]);
        cite(&mut before, "/refs/x.md", "t1", &["alpha"]);
        let mut after = cache_of(&[("alpha", "same")]);
        cite(&mut after, "/refs/x.md", "t2", &["alpha"]);

        let plan = CatalogueDelta::between(&before, &after).plan();
        assert!(plan.list_changed, "an older client has no other signal");
        assert_eq!(plan.updated, vec![skill_uri("alpha")]);
        assert!(!plan.updated.contains(&catalogue_uri()));
    }

    /// A body edit DOES stale the index — it renders the description,
    /// which frontmatter can move without touching membership.
    #[test]
    fn a_body_edit_stales_the_catalogue_index() {
        let plan = CatalogueDelta::between(&cache_of(&[("alpha", "v1")]), &cache_of(&[("alpha", "v2")])).plan();
        assert_eq!(plan.updated, vec![skill_uri("alpha"), catalogue_uri()]);
    }

    /// A declared file that could not be canonicalized is absent from
    /// `declared` entirely, so its citer serves a `status: not-found`
    /// row. The file appearing flips that row to a real path — served
    /// content, with no body edit behind it.
    #[test]
    fn a_reference_appearing_updates_its_citer() {
        let before = cache_of(&[("alpha", "same")]);
        let mut after = cache_of(&[("alpha", "same")]);
        cite(&mut after, "/refs/new.md", "t1", &["alpha"]);

        let delta = CatalogueDelta::between(&before, &after);
        assert_eq!(delta.references_changed, vec!["/refs/new.md".to_string()]);
        assert_eq!(delta.reference_citers, vec!["alpha".to_string()]);
    }

    #[test]
    fn a_reference_vanishing_updates_its_surviving_citer() {
        let mut before = cache_of(&[("alpha", "same")]);
        cite(&mut before, "/refs/gone.md", "t1", &["alpha"]);
        let after = cache_of(&[("alpha", "same")]);

        let delta = CatalogueDelta::between(&before, &after);
        assert_eq!(delta.references_changed, vec!["/refs/gone.md".to_string()]);
        assert_eq!(delta.reference_citers, vec!["alpha".to_string()]);
    }

    /// A slug that no longer exists must never be announced — a client
    /// would fetch a URI that now errors.
    #[test]
    fn a_removed_skill_is_never_a_reference_citer() {
        let mut before = cache_of(&[("alpha", "same"), ("beta", "same")]);
        cite(&mut before, "/refs/x.md", "t1", &["alpha", "beta"]);
        let mut after = cache_of(&[("alpha", "same")]);
        cite(&mut after, "/refs/x.md", "t2", &["alpha"]);

        let delta = CatalogueDelta::between(&before, &after);
        assert!(delta.membership_changed);
        assert_eq!(delta.reference_citers, vec!["alpha".to_string()]);
    }

    /// A skill whose body ALSO changed is announced once. Two
    /// `resources/updated` for one URI is a client re-fetching twice.
    #[test]
    fn a_citer_whose_body_also_moved_is_announced_once() {
        let mut before = cache_of(&[("alpha", "v1")]);
        cite(&mut before, "/refs/x.md", "t1", &["alpha"]);
        let mut after = cache_of(&[("alpha", "v2")]);
        cite(&mut after, "/refs/x.md", "t2", &["alpha"]);

        let delta = CatalogueDelta::between(&before, &after);
        assert_eq!(delta.updated, vec!["alpha".to_string()]);
        assert!(delta.reference_citers.is_empty());
        assert_eq!(delta.plan().updated, vec![skill_uri("alpha"), catalogue_uri()]);
    }

    /// The extension of `a_reload_that_changed_nothing_notifies_nothing`
    /// to references: an untouched reference must stay silent, or the
    /// watcher would announce on every editor temp file.
    #[test]
    fn an_unchanged_reference_fingerprint_announces_nothing() {
        let mut before = cache_of(&[("alpha", "same")]);
        cite(&mut before, "/refs/x.md", "t1", &["alpha"]);
        let mut after = cache_of(&[("alpha", "same")]);
        cite(&mut after, "/refs/x.md", "t1", &["alpha"]);

        let delta = CatalogueDelta::between(&before, &after);
        assert!(delta.is_empty());
        assert_eq!(delta.plan(), Announcement::default());
    }

    /// One `metadata()` per unique file, not per citation. 479
    /// citations across the captain's roots resolve to 60 files; the
    /// per-citation shape would stat each one eight times a rescan.
    #[test]
    fn build_cache_stats_each_declared_file_once() {
        let dir = tempfile::tempdir().unwrap();
        let shared = dir.path().join("shared.md");
        std::fs::write(&shared, "convention").unwrap();
        let canonical = std::fs::canonicalize(&shared).unwrap().display().to_string();

        let mut skills = Vec::new();
        for slug in ["alpha", "beta", "gamma"] {
            let bundle = dir.path().join(slug);
            std::fs::create_dir_all(&bundle).unwrap();
            let path = bundle.join("SKILL.md");
            std::fs::write(&path, "body").unwrap();
            skills.push(crate::mcp::skills::Skill {
                slug: crate::mcp::skills::SkillSlug::parse(slug).unwrap(),
                path,
                title: slug.to_string(),
                description: "d".into(),
                frontmatter: yaml_serde::from_str(
                    "references:
  - ../shared.md
",
                )
                .unwrap(),
                body: "body".into(),
            });
        }

        let cache = build_cache(skills);
        assert_eq!(cache.declared.len(), 1, "one entry for the shared file");
        assert_eq!(
            cache.declared[&canonical].citers,
            vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()]
        );
    }

    /// The common edit: a body changes under an unchanged slug. The list
    /// has not moved, so `resources/list_changed` cannot express it —
    /// only a per-URI `resources/updated` can, and with an indefinite
    /// `ttlMs` that notification is the ONLY thing that invalidates the
    /// client's copy.
    #[test]
    fn an_edited_body_updates_only_that_skill() {
        let delta = CatalogueDelta::between(
            &cache_of(&[("alpha", "v1"), ("beta", "same")]),
            &cache_of(&[("alpha", "v2"), ("beta", "same")]),
        );

        assert!(!delta.membership_changed, "no skill was added or removed");
        assert_eq!(delta.updated, vec!["alpha".to_string()], "beta did not change");
    }

    /// Frontmatter-only edits leave the body byte-identical while
    /// changing what every surface reports, so the metadata block is
    /// compared too.
    #[test]
    fn a_frontmatter_only_edit_still_counts_as_updated() {
        let mut before = cache_of(&[("alpha", "same")]);
        let mut after = cache_of(&[("alpha", "same")]);
        before.skills.get_mut("alpha").unwrap().meta_block = skill_block(
            &frontmatter_json(&yaml_serde::from_str("name: old\n").unwrap()),
            std::path::Path::new("/tmp/a"),
        );
        after.skills.get_mut("alpha").unwrap().meta_block = skill_block(
            &frontmatter_json(&yaml_serde::from_str("name: new\n").unwrap()),
            std::path::Path::new("/tmp/a"),
        );

        assert_eq!(
            CatalogueDelta::between(&before, &after).updated,
            vec!["alpha".to_string()]
        );
    }

    #[test]
    fn adding_or_removing_a_skill_is_a_membership_change() {
        let added = CatalogueDelta::between(
            &cache_of(&[("alpha", "b")]),
            &cache_of(&[("alpha", "b"), ("beta", "b")]),
        );
        assert!(added.membership_changed);
        assert!(added.updated.is_empty(), "an untouched skill must not be re-sent");

        let removed = CatalogueDelta::between(
            &cache_of(&[("alpha", "b"), ("beta", "b")]),
            &cache_of(&[("alpha", "b")]),
        );
        assert!(removed.membership_changed);
    }

    /// The one that makes an indefinite ttl viable: a reload that
    /// changed nothing must invalidate nothing. Firing spuriously would
    /// make every `reload` cost a full re-fetch and teach clients to
    /// ignore us.
    #[test]
    fn a_reload_that_changed_nothing_notifies_nothing() {
        let delta = CatalogueDelta::between(&cache_of(&[("alpha", "b")]), &cache_of(&[("alpha", "b")]));

        assert!(delta.is_empty());
        assert!(!delta.membership_changed);
        assert!(delta.updated.is_empty());
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
        assert!(cache.declared.contains_key(&address));

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

#[cfg(test)]
mod watch_tests {
    use super::{SkillDirEntry, SkillsArgs, SkillsServer};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    const META: &str = r#""_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientInfo":{"name":"t","version":"1"},"io.modelcontextprotocol/clientCapabilities":{}}"#;

    fn write_skill(root: &std::path::Path, slug: &str, body: &str, refs: &str) {
        let bundle = root.join(slug);
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(
            bundle.join("SKILL.md"),
            format!("---\ndescription: d\n{refs}---\n\n# {slug}\n\n{body}\n"),
        )
        .unwrap();
    }

    /// Serve a real skills server over a duplex with the watcher armed
    /// and the relay running, exactly as `run()` wires it: arm, scan,
    /// serve, then spawn the relay with the peer.
    async fn serve_watched(
        root: &std::path::Path,
    ) -> (
        tokio::io::DuplexStream,
        tokio::io::Lines<BufReader<tokio::io::DuplexStream>>,
        Option<crate::watch::Watcher>,
        tokio::task::JoinHandle<()>,
        rmcp::service::RunningService<rmcp::service::RoleServer, SkillsServer>,
    ) {
        let handler = SkillsServer::new(
            SkillsArgs {
                skill_dirs: vec![SkillDirEntry {
                    dir: root.to_path_buf(),
                    ignore: Vec::new(),
                    watch: true,
                }],
            },
            crate::mcp::server::ConfigSource::default(),
        )
        .expect("build skills server");

        let (watcher, signals) = handler.arm_watch(std::time::Duration::from_millis(50)).await;
        let _ = handler.reload_skills().await;
        let relay_server = handler.clone();

        let (client_tx, server_rx) = tokio::io::duplex(1 << 16);
        let (server_tx, client_rx) = tokio::io::duplex(1 << 16);
        let running = crate::mcp::server::rpc::serve_from_first_byte(handler, (server_rx, server_tx));
        let relay = tokio::spawn(relay_server.relay_watch(signals, running.peer().clone()));

        (client_tx, BufReader::new(client_rx).lines(), watcher, relay, running)
    }

    /// Open a `subscriptions/listen` stream for `uris` and wait for the
    /// acknowledgment, so the edit that follows cannot race the
    /// subscription.
    async fn listen(
        client_tx: &mut tokio::io::DuplexStream,
        lines: &mut tokio::io::Lines<BufReader<tokio::io::DuplexStream>>,
        uris: &[&str],
    ) {
        let subs = serde_json::to_string(uris).unwrap();
        client_tx
            .write_all(
                format!(
                    "{{\"jsonrpc\":\"2.0\",\"id\":\"l\",\"method\":\"subscriptions/listen\",\"params\":{{{META},\"notifications\":{{\"resourcesListChanged\":true,\"resourceSubscriptions\":{subs}}}}}}}\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        client_tx.flush().await.unwrap();
        collect_until(lines, |seen| {
            seen.iter().any(|l| l.contains("subscriptions/acknowledged"))
        })
        .await;
    }

    /// Read lines until `done` or a 5 s bound. Bounded per line, because
    /// the failure this guards produces NOTHING and a fixed line count
    /// would hang on the healthy path too.
    async fn collect_until(
        lines: &mut tokio::io::Lines<BufReader<tokio::io::DuplexStream>>,
        done: impl Fn(&[String]) -> bool,
    ) -> Vec<String> {
        let mut seen = Vec::new();
        while let Ok(Ok(Some(line))) = tokio::time::timeout(std::time::Duration::from_secs(5), lines.next_line()).await
        {
            seen.push(line);
            if done(&seen) {
                break;
            }
        }
        seen
    }

    fn updated_for(lines: &[String], uri: &str) -> bool {
        lines
            .iter()
            .any(|l| l.contains("notifications/resources/updated") && l.contains(&format!("\"uri\":\"{uri}\"")))
    }

    /// THE end-to-end pin, and the whole point of the feature: an edit
    /// on disk reaches a subscribed client with nobody calling `reload`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_edit_on_disk_reaches_a_subscribed_client_without_reload() {
        let root = tempfile::tempdir().unwrap();
        write_skill(root.path(), "alpha", "v1", "");
        let (mut client_tx, mut lines, watcher, relay, running) = serve_watched(root.path()).await;
        listen(&mut client_tx, &mut lines, &["hyprpilot://skills/alpha"]).await;

        write_skill(root.path(), "alpha", "v2 edited", "");

        let seen = collect_until(&mut lines, |seen| {
            updated_for(seen, "hyprpilot://skills/alpha")
                && seen.iter().any(|l| l.contains("notifications/resources/list_changed"))
        })
        .await;
        assert!(
            updated_for(&seen, "hyprpilot://skills/alpha"),
            "no per-skill update reached the client: {seen:?}"
        );
        assert!(
            seen.iter().any(|l| l.contains("notifications/resources/list_changed")),
            "no list_changed reached the client: {seen:?}"
        );

        relay.abort();
        drop(watcher);
        drop(client_tx);
        running.cancel().await.ok();
    }

    /// The reference-gap pin over the real wire. Editing a shared
    /// convention file moves no skill body, and before the fingerprint
    /// diff this reached a client as silence for the full 24h ttl.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_reference_edit_on_disk_reaches_the_citing_skill() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("references")).unwrap();
        std::fs::write(root.path().join("references/shared.md"), "v1").unwrap();
        write_skill(
            root.path(),
            "alpha",
            "body",
            "references:\n  - ../references/shared.md\n",
        );

        let (mut client_tx, mut lines, watcher, relay, running) = serve_watched(root.path()).await;
        listen(&mut client_tx, &mut lines, &["hyprpilot://skills/alpha"]).await;

        // Only the reference moves. The skill body is untouched.
        std::fs::write(root.path().join("references/shared.md"), "v2 edited").unwrap();

        let seen = collect_until(&mut lines, |seen| updated_for(seen, "hyprpilot://skills/alpha")).await;
        assert!(
            updated_for(&seen, "hyprpilot://skills/alpha"),
            "a reference edit reached the citing skill as silence: {seen:?}"
        );
        // The index renders a reference COUNT, not its content.
        assert!(
            !updated_for(&seen, "hyprpilot://skills"),
            "the catalogue index was invalidated for a reference edit: {seen:?}"
        );

        relay.abort();
        drop(watcher);
        drop(client_tx);
        running.cancel().await.ok();
    }

    /// A client that opened no stream still has to hear it: everything
    /// before `2026-07-28` cannot subscribe, and a broadcast is the only
    /// channel it has.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_client_with_no_stream_gets_the_broadcast() {
        let root = tempfile::tempdir().unwrap();
        write_skill(root.path(), "alpha", "v1", "");
        let (mut client_tx, mut lines, watcher, relay, running) = serve_watched(root.path()).await;

        client_tx
            .write_all(
                format!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\",\"params\":{{{META}}}}}\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        client_tx.flush().await.unwrap();
        collect_until(&mut lines, |seen| seen.iter().any(|l| l.contains("\"result\""))).await;

        write_skill(root.path(), "alpha", "v2 edited", "");

        let seen = collect_until(&mut lines, |seen| {
            seen.iter().any(|l| l.contains("notifications/resources/list_changed"))
        })
        .await;
        assert!(
            seen.iter().any(|l| l.contains("notifications/resources/list_changed")),
            "no broadcast reached a stream-less client: {seen:?}"
        );

        relay.abort();
        drop(watcher);
        drop(client_tx);
        running.cancel().await.ok();
    }

    /// Editor temp files rescan and diff to nothing. Free on the wire is
    /// what lets the filter skip guessing them by name.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn editor_noise_announces_nothing() {
        let root = tempfile::tempdir().unwrap();
        write_skill(root.path(), "alpha", "v1", "");
        let (mut client_tx, mut lines, watcher, relay, running) = serve_watched(root.path()).await;
        listen(&mut client_tx, &mut lines, &["hyprpilot://skills/alpha"]).await;

        std::fs::write(root.path().join("alpha/.SKILL.md.swp"), "editor scratch").unwrap();

        let quiet = tokio::time::timeout(std::time::Duration::from_millis(1500), lines.next_line()).await;
        assert!(quiet.is_err(), "editor noise produced a notification: {quiet:?}");

        relay.abort();
        drop(watcher);
        drop(client_tx);
        running.cancel().await.ok();
    }
}

#[cfg(test)]
mod opener_tests {
    use super::{SkillsArgs, SkillsServer};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    const META: &str = r#""_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientInfo":{"name":"t","version":"1"},"io.modelcontextprotocol/clientCapabilities":{"roots":{"listChanged":true}}}"#;

    /// Drive one opener sequence against a real skills server over an
    /// in-memory duplex and return every line it wrote.
    ///
    /// The opener is a PARAMETER because rmcp gives the connection's
    /// first request its own code path: `initialize` negotiates,
    /// anything else with valid 2026 `_meta` takes the stateless
    /// branch. A smoke test that only ever opens with `initialize`
    /// exercises one of them and reports the other as covered.
    async fn opener_run(opener: &str, expect_ack: bool) -> Vec<String> {
        let handler = SkillsServer::new(
            SkillsArgs { skill_dirs: Vec::new() },
            crate::mcp::server::ConfigSource::default(),
        )
        .expect("build skills server");

        let (mut client_tx, server_rx) = tokio::io::duplex(1 << 16);
        let (server_tx, client_rx) = tokio::io::duplex(1 << 16);
        let running = crate::mcp::server::rpc::serve_from_first_byte(handler, (server_rx, server_tx));

        client_tx.write_all(opener.as_bytes()).await.unwrap();
        client_tx
            .write_all(
                format!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\",\"params\":{{{META}}}}}\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        client_tx.flush().await.unwrap();

        // Read per line with its own bound, and stop on the first quiet
        // gap rather than on a line count. The failure this guards
        // produces NOTHING, so a reader that waits for a fixed number of
        // lines hangs on the healthy path too and reports both as empty.
        let mut lines = Vec::new();
        let mut buf = BufReader::new(client_rx).lines();
        while let Ok(Ok(Some(line))) = tokio::time::timeout(std::time::Duration::from_secs(5), buf.next_line()).await {
            lines.push(line);
            // Stop as soon as the post-opener request is answered —
            // that is the whole question. Waiting for a quiet gap would
            // add its full timeout to every healthy run.
            // Stop once everything expected has arrived. The ack is a
            // NOTIFICATION, so it can land either side of the response —
            // breaking on the response alone drops it on a fast run.
            if answered(&lines, "1") && (!expect_ack || acknowledged(&lines)) {
                break;
            }
        }
        drop(client_tx);
        running.cancel().await.ok();
        lines
    }

    fn acknowledged(lines: &[String]) -> bool {
        lines.iter().any(|l| l.contains("subscriptions/acknowledged"))
    }

    /// A RESULT for `id`, not merely a message carrying it. Matching an
    /// error too would let a `tools/list` that started failing satisfy a
    /// test whose whole question is whether the server still answers.
    fn answered(lines: &[String], id: &str) -> bool {
        lines.iter().any(|l| {
            serde_json::from_str::<serde_json::Value>(l).ok().is_some_and(|v| {
                v.get("result").is_some()
                    && v.get("id").map(|i| i.to_string().trim_matches('"').to_string()) == Some(id.to_string())
            })
        })
    }

    /// The negotiated version must be what the peer is RECORDED as, not
    /// what it asked for. rmcp's in-loop `initialize` records the
    /// request verbatim, so without `initialize_negotiated` a client
    /// told `2025-11-25` still receives `2026-07-28` result shapes —
    /// and one validating the revision it agreed rejects the listing,
    /// which is the same failure the `ttlMs` stamp exists for.
    ///
    /// Sequenced deliberately: requests now run concurrently, so a
    /// client that pipelines past `initialize` can be answered before
    /// the negotiated version is recorded. The spec forbids that, and
    /// this test asserts the behaviour a conforming client sees.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_down_negotiated_session_is_not_served_a_newer_result_shape() {
        let handler = SkillsServer::new(
            SkillsArgs { skill_dirs: Vec::new() },
            crate::mcp::server::ConfigSource::default(),
        )
        .expect("build skills server");
        let (mut client_tx, server_rx) = tokio::io::duplex(1 << 16);
        let (server_tx, client_rx) = tokio::io::duplex(1 << 16);
        let running = crate::mcp::server::rpc::serve_from_first_byte(handler, (server_rx, server_tx));
        let mut reader = BufReader::new(client_rx).lines();

        async fn next_json(reader: &mut tokio::io::Lines<BufReader<tokio::io::DuplexStream>>) -> serde_json::Value {
            loop {
                let line = tokio::time::timeout(std::time::Duration::from_secs(5), reader.next_line())
                    .await
                    .expect("no reply within the bound")
                    .expect("read")
                    .expect("stream closed");
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
                    if v.get("result").is_some() {
                        return v;
                    }
                }
            }
        }

        client_tx
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2099-01-01\",\"capabilities\":{},\"clientInfo\":{\"name\":\"t\",\"version\":\"1\"}}}\n")
            .await
            .unwrap();
        client_tx.flush().await.unwrap();
        let init = next_json(&mut reader).await;
        assert_eq!(
            init["result"]["protocolVersion"], "2025-11-25",
            "an unsupported request negotiates down"
        );

        client_tx
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\",\"params\":{}}\n")
            .await
            .unwrap();
        client_tx.flush().await.unwrap();
        let tools = next_json(&mut reader).await;
        assert!(
            tools["result"].get("resultType").is_none(),
            "a 2025-11-25 session must not be served a 2026-07-28 shape: {tools}"
        );

        drop(client_tx);
        running.cancel().await.ok();
    }

    /// The regression. Claude Code's v2 runtime probes `server/discover`
    /// on a DISPOSABLE second process, then opens the real transport
    /// with `subscriptions/listen` as its first request — so for a
    /// server implementing subscriptions this ordering is the normal
    /// path, not an edge case.
    ///
    /// Under rmcp's pre-loop handshake it deadlocked: the opener is
    /// handled inline, its acknowledgement awaits a oneshot only the
    /// serve loop fires, and the loop is not spawned until the opener
    /// returns. Zero bytes out, forever — which a client reports as
    /// "connected, tools fetch failed".
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_subscription_opener_is_acknowledged_and_does_not_wedge_the_server() {
        let lines = opener_run(&format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":\"listen:0\",\"method\":\"subscriptions/listen\",\"params\":{{{META},\"notifications\":{{\"resourcesListChanged\":true}}}}}}\n"
        ), true)
        .await;

        assert!(
            acknowledged(&lines),
            "the subscription must be acknowledged, got: {lines:?}"
        );
        assert!(
            answered(&lines, "1"),
            "a request after the opener must still be answered, got: {lines:?}"
        );
    }

    /// The other two openers, so the fix for the one above cannot
    /// silently break the flows that already worked.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn every_opener_leaves_the_server_answering() {
        for opener in [
            "{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2026-07-28\",\"capabilities\":{},\"clientInfo\":{\"name\":\"t\",\"version\":\"1\"}}}\n".to_string(),
            format!("{{\"jsonrpc\":\"2.0\",\"id\":\"d\",\"method\":\"server/discover\",\"params\":{{{META}}}}}\n"),
        ] {
            let lines = opener_run(&opener, false).await;
            assert!(
                answered(&lines, "1"),
                "tools/list unanswered after opener {opener}: {lines:?}"
            );
        }
    }
}

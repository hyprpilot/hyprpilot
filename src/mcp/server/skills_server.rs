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
//! A skill is any directory under a root holding a `SKILL.md`, at any
//! depth, identified by its PATH (`git-commit`, `acme/billing/refunds`).
//!
//! Surfaces:
//! - SEP-2640 (`io.modelcontextprotocol/skills`, `directoryRead`):
//!   `skills/list`, `skills/get` and `resources/directory/read`, with
//!   every file of a bundle served RAW as `skill://<skill-path>/<file>`
//!   and listed with its sha256 digest and size. Raw because a host
//!   verifies the bytes against the digest and re-parses the frontmatter
//!   against the listing — a rendered body would fail both.
//! - Resources: the `hyprpilot://skills` catalogue index, one
//!   `skill://<path>/SKILL.md` per skill, one `hyprpilot://prompts/<name>`
//!   per prompt. Supporting files and shared references are readable but
//!   NOT listed — enumerating them is the listing bloat measured before.
//! - Shared references (frontmatter `references:`, files outside every
//!   bundle) stay addressed by canonical PATH, readable as `file://`
//!   resources for declared paths only. `skill://` cannot name them: the
//!   SEP scheme addresses files inside one skill.
//! - Prompts: the profile's `system_prompt` files and every `*.md` in a
//!   `[[mcp.skills.prompts]]` directory, served through `prompts/list` /
//!   `prompts/get` and as resources.
//! - Tools, for clients that reach the model only through tools:
//!   `list_skills`, `read_skill` (rendered body plus a manifest of its
//!   references and files), `list_skill_references`,
//!   `read_skill_references` (bodies by path), `read_skill_files`
//!   (bundle files by `skill://` URI) and `reload`.
//!
//! The harness tools (`spawn` / `session_*`) live on a SEPARATE server
//! — `hyprpilot mcp harness`, see `super::harness_server`, which makes
//! the gate structural: this server cannot serve a harness tool because
//! it does not implement one. The tool list is fixed for the process,
//! which is why `tools.list_changed` stays `false`.
//!
//! Metadata is de-duplicated to a SINGLE block (`metadata` in tool
//! output, `io.hyprpilot/skill` in resource `_meta`): the WHOLE parsed
//! YAML frontmatter projected losslessly to JSON, minus the keys another
//! field already carries, plus the runtime-derived `path`, `bundleDir`,
//! `size`, `modified` and `created`. `skills/list` carries the
//! frontmatter itself verbatim, as the SEP requires.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

use clap::Args;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CustomRequest, CustomResult, ErrorCode, GetPromptRequestParams,
    GetPromptResponse, GetPromptResult, Implementation, ListPromptsResult, ListResourceTemplatesResult,
    ListResourcesResult, ListToolsResult, PaginatedRequestParams, PromptMessage, ProtocolVersion,
    ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, ResourceContents, Role, ServerCapabilities,
    ServerConfig, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ServerHandler, ServiceExt};
use tokio::sync::RwLock;

use crate::config::mcp::DEFAULT_SKILLS_SERVER_NAME;
use crate::config::ResolvedSkillEntry;
use crate::mcp::skills::prompts::{Prompt, PromptSources};
use crate::mcp::skills::wire_files::{read_bundle, BundleFile};
use crate::mcp::skills::SkillsRegistry;

/// The receiver half `arm_watch` hands the relay.
type WatchSignals = tokio::sync::mpsc::UnboundedReceiver<crate::watch::WatchSignal>;

use super::rpc::{
    empty_object_schema, require_string, structured_with_text, tool_error, wait_for_shutdown, RESULT_CACHE_SCOPE,
};
use crate::mcp::skills::wire_metadata::{frontmatter_json, skill_block, skill_meta};
use crate::mcp::skills::wire_references::{
    self, append_references, file_uri, frontmatter_references, path_from_file_uri, FrontmatterRefs, ReferenceEntry,
};

/// The SEP-2640 extension identifier, declared under
/// `capabilities.extensions`.
const SKILLS_EXTENSION_ID: &str = "io.modelcontextprotocol/skills";

/// Args for `hyprpilot mcp skills`. Skills are discovered by directory
/// scan — the launcher passes `--skill-dir <json>` once per configured
/// root, each carrying that root's globs — and prompts the same way,
/// plus one `--prompt-file` per `system_prompt` file the profile passes
/// through.
#[derive(Debug, Args, Clone)]
pub struct SkillsArgs {
    #[command(flatten)]
    pub serve: super::serve_args::ServeArgs,

    /// JSON-encoded skill root entry. Repeatable — directories are
    /// searched in declaration order; first path wins on collision.
    ///
    /// Shape: `{ "dir": "<abs-path>", "include": [...], "ignore": [...], "watch": true }`
    #[arg(long = "skill-dir", value_parser = parse_dir_arg)]
    pub skill_dirs: Vec<SkillDirEntry>,

    /// JSON-encoded prompt directory, the `--skill-dir` shape. Every
    /// `*.md` directly inside is served as a prompt.
    #[arg(long = "prompt-dir", value_parser = parse_dir_arg)]
    pub prompt_dirs: Vec<SkillDirEntry>,

    /// A single prompt file, served as a prompt named by its frontmatter
    /// `name` or its stem. Repeatable; earlier files win a name.
    #[arg(long = "prompt-file")]
    pub prompt_files: Vec<PathBuf>,
}

/// One decoded `--skill-dir` / `--prompt-dir` entry. The launcher
/// serializes `ResolvedSkillEntry` as JSON; the sidecar deserializes
/// back.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct SkillDirEntry {
    pub dir: PathBuf,
    #[serde(default)]
    pub ignore: Vec<String>,
    /// Allow-list globs. Absent decodes as empty, which means "no
    /// allow-list" — never "allow nothing".
    #[serde(default)]
    pub include: Vec<String>,
    /// Defaults ON, so a hand-written MCP catalogue entry that omits it
    /// still gets a watched root.
    #[serde(default = "watch_default")]
    pub watch: bool,
}

fn watch_default() -> bool {
    true
}

impl SkillDirEntry {
    /// Rebuild the launcher's `ResolvedSkillEntry`, globs compiled.
    fn resolve(self) -> ResolvedSkillEntry {
        let ignore = compile_arg_globs(&self.ignore, &self.dir, "ignore");
        let include = compile_arg_globs(&self.include, &self.dir, "include");
        ResolvedSkillEntry {
            // Absolutized here because notify joins a relative watch
            // path onto the process cwd while we would keep the relative
            // form — every event would then fail `strip_prefix` and be
            // dropped, which reads as a root that is watched and never
            // fires. The launcher already absolutizes (`resolve_user`),
            // so this only covers a hand-written catalogue entry.
            dir: crate::paths::resolve_user(&self.dir.to_string_lossy()),
            ignore_patterns: self.ignore,
            ignore,
            include_patterns: self.include,
            include,
            watch: self.watch,
        }
    }
}

/// Compile one `--skill-dir` glob list. An empty list is `None` — no
/// filter at all, which for `include` is the difference between "allow
/// everything" and "allow nothing". A bad glob is logged and skipped
/// rather than aborting startup (graceful degradation).
fn compile_arg_globs(patterns: &[String], dir: &std::path::Path, kind: &str) -> Option<globset::GlobSet> {
    if patterns.is_empty() {
        return None;
    }
    let mut builder = globset::GlobSetBuilder::new();
    for pat in patterns {
        match globset::Glob::new(pat) {
            Ok(g) => {
                builder.add(g);
            }
            Err(err) => {
                tracing::warn!(
                    %err,
                    pattern = %pat,
                    dir = %dir.display(),
                    kind,
                    "mcp::server: bad skill glob — skipping"
                );
            }
        }
    }
    builder.build().ok()
}

fn parse_dir_arg(raw: &str) -> Result<SkillDirEntry, String> {
    serde_json::from_str::<SkillDirEntry>(raw)
        .map_err(|e| format!("must be a JSON object `{{\"dir\":\"...\",\"include\":[...],\"ignore\":[...]}}`: {e}"))
}

/// Run the rmcp stdio server in the foreground. Returns when the
/// vendor closes the pipe (or on init error).
pub async fn run_skills(args: SkillsArgs, config: super::ConfigSource) -> anyhow::Result<()> {
    tracing::info!(
        dirs = args.skill_dirs.len(),
        prompt_dirs = args.prompt_dirs.len(),
        prompt_files = args.prompt_files.len(),
        "mcp: starting the skills server"
    );
    let serve = args.serve.clone();
    run(SkillsServer::new(args, config)?, &serve).await
}

async fn run(handler: SkillsServer, serve: &super::serve_args::ServeArgs) -> anyhow::Result<()> {
    // Armed BEFORE the startup scan, so an edit landing between the scan
    // and the first drain is queued rather than lost.
    let (watcher, signals) = handler.arm_watch(crate::watch::DEBOUNCE).await;

    // Startup scan. Nothing is connected yet, so the delta has no
    // one to notify — discard it deliberately rather than by accident.
    let _ = handler.reload_skills().await;

    // Cloned before serving consumes the handler — Arcs only, the same
    // shape the harness uses to keep a handle on its session table.
    let relay_server = handler.clone();

    if serve.transport == super::serve_args::Transport::Http {
        // No peer to hand the relay: every HTTP request under
        // `2026-07-28` is stateless, so nothing outlives a response to
        // broadcast through. An edit still reaches every client holding
        // a `subscriptions/listen` stream, because the sinks live in the
        // registry this handler's clones all share.
        let relay = tokio::spawn(relay_server.relay_watch(signals, None));
        let served = super::http::serve_http(handler, serve, crate::config::mcp::DEFAULT_SKILLS_SERVER_NAME).await;
        relay.abort();
        drop(watcher);

        return served;
    }

    let running = handler.serve(rmcp::transport::io::stdio()).await?;

    // The peer exists only once the service is running, which is also
    // the earliest a notification could reach anyone — so this ordering
    // is correct, not merely convenient. An edit before then is queued
    // on the armed channel, not lost.
    let relay = tokio::spawn(relay_server.relay_watch(signals, Some(running.peer().clone())));

    // Race the transport against SIGTERM/SIGHUP. Without this a
    // supervisor stopping the sidecar would skip every destructor.
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
    skills: HashMap<String, LoadedSkill>,
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
    /// served content.
    declared: HashMap<String, DeclaredReference>,
    /// Every served bundle file, keyed by its full skill path
    /// (`<skill-path>/<file-path>`, the URI minus its scheme). A nested
    /// skill's files are also its enclosing skill's, and land here once.
    /// Ordered, so a directory's children are one contiguous range.
    files: BTreeMap<String, BundleFile>,
    prompts: Vec<Prompt>,
}

impl SkillsCache {
    /// The file a `skill://` URI's path names.
    fn file(&self, path: &str) -> Option<&BundleFile> {
        self.files.get(path)
    }

    fn prompt(&self, name: &str) -> Option<&Prompt> {
        self.prompts.iter().find(|p| p.name == name)
    }

    /// The direct children of the directory `path` names, as resource
    /// descriptors: files with their own metadata, subdirectories as
    /// `inode/directory`. `None` when no served file lives under it —
    /// which covers both an unknown path and one naming a file.
    ///
    /// Every directory counts: a skill's root, any subdirectory, and the
    /// organizational prefixes above a nested skill (`skill://acme`).
    fn directory(&self, path: &str) -> Option<Vec<rmcp::model::Resource>> {
        let prefix = format!("{path}/");
        let mut children: BTreeMap<&str, Option<&BundleFile>> = BTreeMap::new();
        for (full, file) in self.files.range(prefix.clone()..) {
            let Some(rest) = full.strip_prefix(&prefix) else { break };
            match rest.split_once('/') {
                None => {
                    children.insert(rest, Some(file));
                }
                Some((dir, _)) => {
                    children.entry(dir).or_insert(None);
                }
            }
        }
        if children.is_empty() {
            return None;
        }
        Some(
            children
                .into_iter()
                .map(|(name, file)| {
                    let uri = format!("skill://{prefix}{name}");
                    match file {
                        Some(file) => rmcp::model::Resource::new(uri, name)
                            .with_mime_type(file.mime_type())
                            .with_size(file.size()),
                        None => rmcp::model::Resource::new(uri, name).with_mime_type("inode/directory"),
                    }
                })
                .collect(),
        )
    }
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
    /// The frontmatter, verbatim, as `skills/list` must serve it.
    frontmatter: serde_json::Map<String, serde_json::Value>,
    /// The single de-duplicated metadata block, built once here (not
    /// per request) — see `skills::wire_metadata::{skill_block,
    /// skill_meta}`.
    pub(crate) meta_block: serde_json::Map<String, serde_json::Value>,
    /// The `SKILL.md` body with its frontmatter fence stripped — the
    /// RENDERED view the tools serve. The raw file is in `files`.
    body: String,
    refs: FrontmatterRefs,
    /// Every file of the bundle, `SKILL.md` included, in walk order.
    files: Vec<BundleFile>,
}

impl LoadedSkill {
    fn bundle_dir(&self) -> Option<&std::path::Path> {
        self.path.parent()
    }

    /// Resolve every declared reference: its canonical path, display
    /// name, timestamps, and its own frontmatter.
    ///
    /// Resolved from disk per call rather than cached alongside the
    /// body: a reference is edited far more often than the skill that
    /// declares it, and caching here would serve a stale convention —
    /// and a stale mtime — until an unrelated reload happened to clear
    /// it.
    fn references(&self) -> Vec<ReferenceEntry> {
        if self.refs.references.is_empty() {
            return Vec::new();
        }
        self.bundle_dir()
            .map(|dir| wire_references::resolve(dir, &self.refs))
            .unwrap_or_default()
    }

    fn uri(&self) -> String {
        skill_md_uri(&self.slug)
    }

    fn skill_md(&self) -> Option<&BundleFile> {
        self.files.iter().find(|f| f.rel == "SKILL.md")
    }

    /// Supporting files — everything but `SKILL.md`.
    fn supporting(&self) -> impl Iterator<Item = &BundleFile> {
        self.files.iter().filter(|f| f.rel != "SKILL.md")
    }

    /// The SEP-2640 entry `skills/list` and `skills/get` return.
    ///
    /// `digest` at the top level is not in the final spec — it is the
    /// SKILL.md digest Claude Code's client (2.1.286) reads from an
    /// earlier draft. A result is an open map, so it costs a conforming
    /// host nothing.
    fn entry(&self) -> serde_json::Value {
        serde_json::json!({
            "uri": self.uri(),
            "frontmatter": self.frontmatter,
            "resources": self.files.iter().map(|f| serde_json::json!({
                "uri": skill_file_uri(&self.slug, &f.rel),
                "digest": f.digest,
                "size": f.size(),
            })).collect::<Vec<_>>(),
            "digest": self.skill_md().map(|f| f.digest.clone()),
        })
    }
}

/// What one scan read off disk, before the cache is built from it.
struct Scan {
    skills: Vec<(crate::mcp::skills::Skill, Vec<BundleFile>)>,
    prompts: Vec<Prompt>,
}

// ── Server ────────────────────────────────────────────────────────────

#[derive(Clone)]
struct SkillsServer {
    registry: Arc<SkillsRegistry>,
    prompts: Arc<PromptSources>,
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
    /// How this process is served, which decides how long a client may
    /// cache what it reads — see [`Transport::result_ttl_ms`].
    transport: super::serve_args::Transport,
}

impl SkillsServer {
    fn new(args: SkillsArgs, _config: super::ConfigSource) -> anyhow::Result<Self> {
        let transport = args.serve.transport;
        let entries: Vec<ResolvedSkillEntry> = args.skill_dirs.into_iter().map(SkillDirEntry::resolve).collect();
        let prompts = PromptSources {
            files: args
                .prompt_files
                .iter()
                .map(|f| crate::paths::resolve_user(&f.to_string_lossy()))
                .collect(),
            dirs: args.prompt_dirs.into_iter().map(SkillDirEntry::resolve).collect(),
        };

        Ok(Self {
            registry: Arc::new(SkillsRegistry::new(entries)),
            prompts: Arc::new(prompts),
            skills_cache: Arc::new(RwLock::new(SkillsCache::default())),
            subscriptions: super::rpc::Subscriptions::default(),
            reload_gate: Arc::new(tokio::sync::Mutex::new(())),
            watch_status: Arc::new(RwLock::new(crate::watch::WatchStatus::default())),
            transport,
        })
    }

    /// Server instructions — the one place a client learns the whole
    /// workflow before it reads any individual tool schema.
    fn instructions(&self) -> String {
        String::from(
            "Hyprpilot skills MCP server. Call `list_skills` to enumerate skills and `read_skill { slug }` \
             to load one; a slug is the skill's path, `name` or `group/name` for a nested skill. \
             `read_skill` returns the instructions plus two manifests, neither with bodies: the shared \
             REFERENCES the skill declares (fetch with `read_skill_references { references: [path] }`, \
             passing each row's `path` — a path is a file, so one call spans skills and a path you \
             already loaded needs no second fetch) and the skill's own FILES such as `scripts/` \
             (fetch with `read_skill_files { uris: [...] }`). `bundle: true` on `read_skill` returns \
             every reference body in one call. Skills are also SEP-2640 resources: \
             `skill://<path>/SKILL.md` and every bundle file as `skill://<path>/<file>`, plus \
             `skills/list`, `skills/get` and `resources/directory/read`; a shared reference reads as \
             its `file://` uri. Prompts (including this profile's system prompt) are served as MCP \
             prompts and as `hyprpilot://prompts/<name>` resources, so an edited system prompt can be \
             re-read mid-session. Roots are WATCHED: edits announce themselves as \
             `resources/updated` + `resources/list_changed` (and `prompts/list_changed`), so you \
             never need `reload` unless `list_skills` reports a root degraded or off.",
        )
    }

    /// Arm the watcher over every configured root and record what it
    /// covers.
    ///
    /// Called BEFORE the startup scan: an edit landing between the scan
    /// and the first drain is then queued rather than lost. The channel
    /// is unbounded and nothing reads it yet.
    ///
    /// Prompt sources are flat, so their roots are not recursive; a
    /// prompt FILE watches its parent, because an editor's atomic save
    /// replaces the inode and a watch on the file itself would die with
    /// the first edit.
    async fn arm_watch(&self, debounce: std::time::Duration) -> (Option<crate::watch::Watcher>, WatchSignals) {
        let mut roots: Vec<crate::watch::WatchRoot> = self
            .registry
            .dirs()
            .iter()
            .map(|entry| crate::watch::WatchRoot {
                dir: entry.dir.clone(),
                watch: entry.watch,
                recursive: true,
            })
            .collect();
        roots.extend(self.prompts.dirs.iter().map(|entry| crate::watch::WatchRoot {
            dir: entry.dir.clone(),
            watch: entry.watch,
            recursive: false,
        }));
        let mut parents: Vec<PathBuf> = self
            .prompts
            .files
            .iter()
            .filter_map(|f| f.parent().map(std::path::Path::to_path_buf))
            .collect();
        parents.sort();
        parents.dedup();
        roots.extend(parents.into_iter().map(|dir| crate::watch::WatchRoot {
            dir,
            watch: true,
            recursive: false,
        }));
        let armed = crate::watch::arm(&roots, debounce);
        *self.watch_status.write().await = armed.status;
        (armed.watcher, armed.signals)
    }

    /// Fire what one rescan invalidated.
    ///
    /// The single notification path. Both callers — the `reload` tool
    /// and the watcher relay — reach the wire only through here, so the
    /// two cannot drift into announcing different things for the same
    /// delta. `peer` is `None` over HTTP, where there is no ambient one
    /// to broadcast through — see [`super::rpc::Subscriptions`].
    async fn announce(&self, peer: Option<&rmcp::service::Peer<RoleServer>>, delta: &CatalogueDelta) {
        // Deliberately NOT gated on `peer.peer_info()`. A client that
        // opens with `subscriptions/listen` never records peer info at
        // all, so gating on it would silence every notification for
        // exactly the clients that asked for them.
        let plan = delta.plan();
        if plan.resources_list_changed {
            self.subscriptions.resource_list_changed(peer).await;
        }
        if plan.prompts_list_changed {
            self.subscriptions.prompt_list_changed(peer).await;
        }
        self.subscriptions.resources_updated(peer, plan.updated).await;
    }

    /// Turn watch signals into rescans for as long as the transport
    /// lives.
    ///
    /// Never an opener and never on a request's path: it starts once
    /// `serve` has returned, so the serve loop that drains its
    /// notifications already exists.
    async fn relay_watch(self, mut signals: WatchSignals, peer: Option<rmcp::service::Peer<RoleServer>>) {
        while let Some(first) = signals.recv().await {
            // Drain the burst before doing any work: a `git checkout`
            // that outlasts the debounce window still costs one rescan
            // per quiet window, and no signal is skipped.
            //
            // Every degradation in the burst is recorded, not just the
            // first: two roots can fail in one window, and keeping only
            // one would report the other as covered.
            let mut degradations: Vec<(Vec<std::path::PathBuf>, crate::watch::Degradation)> = Vec::new();
            let mut note = |signal: &crate::watch::WatchSignal| {
                if let Some((dirs, reason)) = signal.degraded() {
                    degradations.push((dirs.to_vec(), reason.clone()));
                }
            };
            note(&first);
            while let Ok(more) = signals.try_recv() {
                note(&more);
            }
            if !degradations.is_empty() {
                let mut status = self.watch_status.write().await;
                for (dirs, reason) in &degradations {
                    status.degrade(dirs, reason);
                }
            }
            let delta = self.reload_skills().await;
            if delta.is_empty() {
                // Sibling files beside a watched prompt and `git`
                // internals reach here and diff to nothing. Free on the
                // wire, which is why the filter does not try to guess
                // them by name.
                tracing::debug!("mcp::server: watched change rescanned — no catalogue change");
                continue;
            }
            tracing::info!(
                membership_changed = delta.membership_changed,
                updated = delta.updated.len(),
                files_changed = delta.files_changed.len(),
                references_changed = delta.references_changed.len(),
                prompts_changed = delta.prompts_changed.len(),
                "mcp::server: skills rescanned from a watched change"
            );
            self.announce(peer.as_ref(), &delta).await;
        }
        // The sender dropped. When nothing was ever armed that is the
        // ordinary shape of a config with no watchable root, not a
        // failure — warning there would fire at startup on a correct
        // deployment and teach the captain to ignore the line that
        // matters.
        let mut status = self.watch_status.write().await;
        if status
            .roots
            .iter()
            .any(|r| r.state == crate::watch::WatchState::Watching)
        {
            status.degrade(&[], &crate::watch::Degradation::BackendExited);
            tracing::warn!("mcp::server: skills watcher stopped — `reload` is the only refresh now");
        }
    }

    /// Rescan disk and report what changed, so the caller can fire the
    /// notification that matches. Returns an empty delta when the reload
    /// failed — a failed rescan leaves the cache untouched, so claiming
    /// anything changed would invalidate a client's cache for nothing.
    async fn reload_skills(&self) -> CatalogueDelta {
        let _ordered = self.reload_gate.lock().await;
        // Carried into the scan so an unchanged file is not read and
        // hashed again. Cheap: the bytes are shared, not copied.
        let previous: HashMap<PathBuf, BundleFile> = self
            .skills_cache
            .read()
            .await
            .files
            .values()
            .map(|f| (f.abs.clone(), f.clone()))
            .collect();
        let registry = self.registry.clone();
        let prompts = self.prompts.clone();
        let result = tokio::task::spawn_blocking(move || {
            registry.reload().map_err(|e| e.to_string())?;
            let mut skills = Vec::new();
            for skill in registry.list() {
                let Some(dir) = skill.path.parent() else { continue };
                match read_bundle(dir, &previous) {
                    Ok(files) => skills.push((skill, files)),
                    Err(err) => tracing::warn!(
                        slug = %skill.slug,
                        %err,
                        "mcp::server: bundle past the SEP-2640 limits — not served"
                    ),
                }
            }
            Ok::<Scan, String>(Scan {
                skills,
                prompts: prompts.load(),
            })
        })
        .await;

        let scan = match result {
            Ok(Ok(scan)) => scan,
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
        let next = build_cache(scan);
        let delta = CatalogueDelta::between(&cache, &next);
        *cache = next;
        delta
    }
}

/// What a rescan actually changed, so the right notification fires for
/// the right URI.
///
/// With `ttlMs` effectively indefinite, a client re-fetches only when
/// told to. `resources/list_changed` covers a skill appearing or
/// disappearing; it says nothing about a file that changed under an
/// unchanged skill, which is the common edit. That needs a per-URI
/// `resources/updated`.
#[derive(Debug, Default, PartialEq)]
struct CatalogueDelta {
    /// Skills added or removed — membership, so the LIST changed.
    membership_changed: bool,
    /// Slugs whose served view — body, metadata, frontmatter, declared
    /// references or any file's digest — differs from the previous scan.
    updated: Vec<String>,
    /// Full skill paths (`<skill-path>/<file>`) whose digest changed,
    /// appeared or vanished.
    files_changed: Vec<String>,
    /// Canonical reference paths whose fingerprint changed, appeared, or
    /// vanished.
    references_changed: Vec<String>,
    /// Prompt names whose content changed, appeared or vanished.
    prompts_changed: Vec<String>,
}

impl CatalogueDelta {
    fn between(before: &SkillsCache, after: &SkillsCache) -> Self {
        let digests = |skill: &LoadedSkill| -> Vec<(String, String)> {
            skill.files.iter().map(|f| (f.rel.clone(), f.digest.clone())).collect()
        };
        let updated: Vec<String> = after
            .order
            .iter()
            .filter(|slug| {
                // Every field a surface actually serves, compared
                // directly — `skill_block` STRIPS `title` /
                // `description` / `references`, and its mtime is
                // truncated to seconds, so comparing the block alone
                // missed a fast edit.
                match (before.skills.get(*slug), after.skills.get(*slug)) {
                    (Some(old), Some(new)) => {
                        old.body != new.body
                            || old.meta_block != new.meta_block
                            || old.frontmatter != new.frontmatter
                            || old.title != new.title
                            || old.description != new.description
                            || old.refs != new.refs
                            || digests(old) != digests(new)
                    }
                    _ => false,
                }
            })
            .cloned()
            .collect();

        let mut files_changed: Vec<String> = after
            .files
            .iter()
            .filter(|(path, file)| before.files.get(*path).map_or(true, |old| old.digest != file.digest))
            .map(|(path, _)| path.clone())
            .collect();
        files_changed.extend(
            before
                .files
                .keys()
                .filter(|path| !after.files.contains_key(*path))
                .cloned(),
        );
        files_changed.sort_unstable();

        let mut references_changed: Vec<String> = after
            .declared
            .iter()
            .filter(|(path, entry)| before.declared.get(*path).map_or(true, |old| old.stat != entry.stat))
            .map(|(path, _)| path.clone())
            .collect();
        // A vanished path is served text changing too: a surviving
        // citer's manifest row turns into `status: not-found`.
        references_changed.extend(
            before
                .declared
                .keys()
                .filter(|path| !after.declared.contains_key(*path))
                .cloned(),
        );
        references_changed.sort_unstable();

        let mut prompts_changed: Vec<String> = after
            .prompts
            .iter()
            .filter(|prompt| before.prompt(&prompt.name) != Some(*prompt))
            .map(|prompt| prompt.name.clone())
            .collect();
        prompts_changed.extend(
            before
                .prompts
                .iter()
                .filter(|prompt| after.prompt(&prompt.name).is_none())
                .map(|prompt| prompt.name.clone()),
        );

        Self {
            membership_changed: before.order != after.order,
            updated,
            files_changed,
            references_changed,
            prompts_changed,
        }
    }

    fn is_empty(&self) -> bool {
        !self.membership_changed
            && self.updated.is_empty()
            && self.files_changed.is_empty()
            && self.references_changed.is_empty()
            && self.prompts_changed.is_empty()
    }

    /// The URIs a rescan invalidates, decided in one pure place so the
    /// watcher and the `reload` tool cannot drift apart.
    fn plan(&self) -> Announcement {
        let mut updated: Vec<String> = Vec::new();
        let mut push = |uri: String| {
            if !updated.contains(&uri) {
                updated.push(uri);
            }
        };
        for slug in &self.updated {
            push(skill_md_uri(slug));
        }
        // A file shared by a skill and its nested child is one path, so
        // one announcement, however many entries list it.
        for path in &self.files_changed {
            push(format!("skill://{path}"));
        }
        for path in &self.references_changed {
            push(file_uri(path));
        }
        for name in &self.prompts_changed {
            push(prompt_uri(name));
        }
        // The index renders slug, title, description and reference
        // COUNT — never a file's or a reference's content, so neither
        // alone stales it.
        if self.membership_changed || !self.updated.is_empty() {
            push(catalogue_uri());
        }
        Announcement {
            // Anything at all. A pre-`2026-07-28` client cannot
            // subscribe, so `resources/updated` is not a signal it can
            // act on; `list_changed` is the only one it has.
            resources_list_changed: !self.is_empty(),
            // MCP has no per-prompt update, so a changed BODY is a list
            // change too — it is how a client learns to re-fetch.
            prompts_list_changed: !self.prompts_changed.is_empty(),
            updated,
        }
    }
}

/// What one rescan tells connected clients.
#[derive(Debug, Default, PartialEq)]
struct Announcement {
    resources_list_changed: bool,
    prompts_list_changed: bool,
    updated: Vec<String>,
}

fn build_cache(scan: Scan) -> SkillsCache {
    let mut cache = SkillsCache {
        prompts: scan.prompts,
        ..SkillsCache::default()
    };
    for (skill, files) in scan.skills {
        let slug = skill.slug.to_string();
        let refs = frontmatter_references(&skill.frontmatter);
        let frontmatter = frontmatter_json(&skill.frontmatter);
        // The single merged block, built ONCE per skill here — not per
        // request.
        let meta_block = skill_block(&frontmatter, &skill.path);
        // Title falls back to the frontmatter `name`, then the slug.
        let title = if skill.title.trim().is_empty() {
            frontmatter_string(&skill.frontmatter, "name").unwrap_or_else(|| slug.clone())
        } else {
            skill.title.clone()
        };
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
        for file in &files {
            cache.files.insert(format!("{slug}/{}", file.rel), file.clone());
        }
        cache.order.push(slug.clone());
        cache.skills.insert(
            slug.clone(),
            LoadedSkill {
                slug,
                path: skill.path,
                title,
                description: skill.description,
                frontmatter,
                meta_block,
                body: skill.body,
                refs,
                files,
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

/// The catalogue index resource. It renders every skill's slug, title
/// and description, so a skill change makes it stale.
fn catalogue_uri() -> String {
    "hyprpilot://skills".to_string()
}

fn skill_md_uri(slug: &str) -> String {
    format!("skill://{slug}/SKILL.md")
}

fn skill_file_uri(slug: &str, rel: &str) -> String {
    format!("skill://{slug}/{rel}")
}

fn prompt_uri(name: &str) -> String {
    format!("hyprpilot://prompts/{name}")
}

/// Every URI this server answers for.
enum ParsedUri<'a> {
    /// The bare `hyprpilot://skills` index.
    Catalogue,
    Prompt(&'a str),
    /// A `skill://` path — a bundle file or a directory, which only the
    /// cache can tell apart.
    Skill(&'a str),
    /// A `file://` uri, decoded to the path it names.
    Reference(String),
}

fn parse_uri(uri: &str) -> Option<ParsedUri<'_>> {
    if uri == "hyprpilot://skills" {
        return Some(ParsedUri::Catalogue);
    }
    if let Some(name) = uri.strip_prefix("hyprpilot://prompts/") {
        return (!name.is_empty()).then_some(ParsedUri::Prompt(name));
    }
    if let Some(path) = uri.strip_prefix("skill://") {
        // Directory URIs carry no trailing slash; tolerate one.
        let path = path.strip_suffix('/').unwrap_or(path);
        return (!path.is_empty()).then_some(ParsedUri::Skill(path));
    }
    path_from_file_uri(uri).map(ParsedUri::Reference)
}

/// Whether this request runs at `2026-07-28` or later, which decides
/// whether a hand-built result carries `resultType`. rmcp strips that
/// field from its own result types for an older peer but passes a
/// `CustomResult` through untouched, so the custom methods have to make
/// the same call themselves.
fn negotiated_modern(context: &RequestContext<RoleServer>) -> bool {
    context
        .protocol_version()
        .is_some_and(|v| v.as_str() >= ProtocolVersion::V_2026_07_28.as_str())
}

/// Stamp a hand-built result the way rmcp stamps its own: the cache
/// fields `2026-07-28` requires on a cacheable result, and `resultType`
/// only for a peer that negotiated that revision.
fn custom_result(
    mut body: serde_json::Map<String, serde_json::Value>,
    ttl_ms: u64,
    modern: bool,
) -> Result<CustomResult, rmcp::ErrorData> {
    body.insert("ttlMs".into(), ttl_ms.into());
    body.insert(
        "cacheScope".into(),
        serde_json::to_value(RESULT_CACHE_SCOPE).map_err(|e| rmcp::ErrorData::internal_error(e.to_string(), None))?,
    );
    if modern {
        body.insert("resultType".into(), "complete".into());
    }
    Ok(CustomResult::new(serde_json::Value::Object(body)))
}

/// `params.uri` of a custom request, required.
fn require_uri(params: Option<&serde_json::Value>) -> Result<&str, rmcp::ErrorData> {
    params
        .and_then(|p| p.get("uri"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| rmcp::ErrorData::invalid_params("`uri` is required", None))
}

/// Every listing here is a single page, so no cursor this server could
/// have issued exists — refusing one is more honest than ignoring it.
fn refuse_cursor(params: Option<&serde_json::Value>) -> Result<(), rmcp::ErrorData> {
    match params.and_then(|p| p.get("cursor")) {
        None | Some(serde_json::Value::Null) => Ok(()),
        Some(_) => Err(rmcp::ErrorData::invalid_params("unknown cursor", None)),
    }
}

fn list_skills_payload(cache: &SkillsCache) -> serde_json::Value {
    let entries: Vec<serde_json::Value> = cache
        .order
        .iter()
        .filter_map(|slug| cache.skills.get(slug))
        .map(|s| {
            // Reference DETAIL is deliberately absent: `list_skills` is
            // the routing view ("which skill?"), served purely from
            // cache. `list_skill_references` owns that question.
            serde_json::json!({
                "slug": s.slug,
                "title": s.title,
                "description": s.description,
                "uri": s.uri(),
                "referenceCount": s.refs.references.len(),
                "fileCount": s.supporting().count(),
                "metadata": s.meta_block,
            })
        })
        .collect();

    serde_json::json!({ "skills": entries })
}

fn slug_prop() -> serde_json::Value {
    serde_json::json!({
        "type": "string",
        "description": "The skill's slug: its path, `name` or `group/name` for a nested skill.",
    })
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
/// this server could hand a client.
fn list_references_object_schema() -> Arc<serde_json::Map<String, serde_json::Value>> {
    object_schema(serde_json::json!({ "slug": slug_prop() }))
}

/// An object schema with one REQUIRED string-array property.
fn string_array_schema(key: &str, description: &str) -> Arc<serde_json::Map<String, serde_json::Value>> {
    let serde_json::Value::Object(map) = serde_json::json!({
        "type": "object",
        "properties": {
            key: {
                "type": "array",
                "items": { "type": "string" },
                "description": description,
            },
        },
        "required": [key],
        "additionalProperties": false,
    }) else {
        unreachable!("json! object literal")
    };
    Arc::new(map)
}

/// `read_skill`'s schema — `slug`, plus an opt-IN for the full bundle.
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
/// tool call at all. It leads with how to load what it lists, because
/// an index whose entries the reader cannot then load is only half an
/// answer.
fn catalogue_markdown(cache: &SkillsCache) -> String {
    let mut out = String::from(
        "# hyprpilot skills\n\n\
         Each entry below is loadable by URI:\n\n\
         - `skill://<path>/SKILL.md` — the skill's raw `SKILL.md`, frontmatter included. Read this \
         first; it is the instruction set. `read_skill { slug }` returns the same body rendered, plus \
         manifests of what it references and ships.\n\
         - `skill://<path>/<file>` — a file the skill ships (`scripts/`, its own `references/`). \
         `read_skill_files` fetches them by uri.\n\n\
         A frontmatter `references:` list names SHARED files outside the bundle. Their address is a \
         canonical PATH, which `list_skill_references { slug }` resolves and `read_skill_references` \
         fetches — one call takes as many as you need, and a path is a file, so the same convention \
         cited by many skills is fetched once. Each also reads as a `file://` resource.\n\n\
         The roots are watched, so this index is kept current; `reload` forces a rescan if a root is \
         reported unwatched.\n\n",
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
        out.push_str(&format!("`{}`", skill.uri()));
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

/// A text manifest of a skill's own files, appended to its rendered
/// body — the same safety net the references footer is, for clients
/// that never surface structured content.
fn files_footer(skill: &LoadedSkill) -> String {
    let files: Vec<&BundleFile> = skill.supporting().collect();
    if files.is_empty() {
        return String::new();
    }
    let mut out = format!(
        "\n---\nskill_files:\n  skill: {}\n  count: {}\n  \
         note: bodies are NOT included above - pass the uris below to `read_skill_files`\n  available:\n",
        skill.slug,
        files.len()
    );
    for file in files {
        out.push_str(&format!(
            "    - uri: {}\n      size: {}\n",
            skill_file_uri(&skill.slug, &file.rel),
            file.size()
        ));
    }
    out.push_str("---\n");
    out
}

/// Text projection for `list_skill_references`.
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
        out.push_str(&format!("- {}: {}\n", skill.slug, skill.description));
    }
    out.push_str("Call `read_skill` with a slug to fetch the full SKILL.md body.");
    out
}

/// One fetched bundle file, as `read_skill_files` frames it: a YAML
/// header naming the file, then its text. A file that is not UTF-8 has
/// no text a model can read, so its header says where the bytes are.
fn file_block(uri: &str, file: &BundleFile) -> String {
    let mut out = format!(
        "---\nfile:\n  uri: {uri}\n  size: {}\n  mimeType: {}\n  digest: {}\n",
        file.size(),
        file.mime_type(),
        file.digest
    );
    match file.text() {
        Some(text) => {
            out.push_str("---\n");
            out.push_str(text);
            if !text.ends_with('\n') {
                out.push('\n');
            }
        }
        None => out.push_str("  status: binary - read it with resources/read\n---\n"),
    }
    out
}

// ── MCP protocol impl ─────────────────────────────────────────────────

impl ServerHandler for SkillsServer {
    fn supported_protocol_versions(&self) -> std::borrow::Cow<'static, [rmcp::model::ProtocolVersion]> {
        super::rpc::supported_protocol_versions()
    }

    fn get_info(&self) -> ServerConfig {
        let mut caps = ServerCapabilities::default();
        // rmcp marks these `#[non_exhaustive]` — no struct literal
        // outside the crate — so mutate the owned `default()` instances'
        // public fields instead.
        let mut tools = rmcp::model::ToolsCapability::default();
        tools.list_changed = Some(false);
        caps.tools = Some(tools);
        let mut resources = rmcp::model::ResourcesCapability::default();
        // Per-resource subscriptions are how a client learns that ONE
        // file changed rather than re-reading the catalogue — what makes
        // the indefinite `ttlMs` safe.
        resources.subscribe = Some(true);
        resources.list_changed = Some(true);
        caps.resources = Some(resources);
        let mut prompts = rmcp::model::PromptsCapability::default();
        prompts.list_changed = Some(true);
        caps.prompts = Some(prompts);
        // SEP-2640. Declaring it commits this server to `skills/list` and
        // `skills/get`; `directoryRead` adds `resources/directory/read`.
        caps.extensions = Some(
            [(
                SKILLS_EXTENSION_ID.to_string(),
                [("directoryRead".to_string(), serde_json::Value::Bool(true))]
                    .into_iter()
                    .collect(),
            )]
            .into_iter()
            .collect(),
        );
        ServerConfig::new(caps)
            .with_server_info(Implementation::new(
                DEFAULT_SKILLS_SERVER_NAME.to_string(),
                env!("CARGO_PKG_VERSION").to_string(),
            ))
            .with_instructions(self.instructions())
    }

    /// Accept the `subscriptions/listen` opt-in at `2026-07-28`.
    ///
    /// The acknowledgment is the client's contract, so a URI is accepted
    /// only when this server can fire for it: the catalogue, a prompt,
    /// any `skill://` path (a file can appear later and is announced when
    /// it does), and a `file://` reference some skill declares. The SDK
    /// intersects the result with the advertised capabilities, which is
    /// what refuses `toolsListChanged`.
    fn accepted_subscription_filter(
        &self,
        requested: &rmcp::model::SubscriptionFilter,
    ) -> Option<rmcp::model::SubscriptionFilter> {
        // Synchronous, so the cache is consulted only if no rescan holds
        // it; mid-rescan a declared path is accepted, since the next
        // scan is what decides it anyway.
        let cache = self.skills_cache.try_read().ok();
        let mut accepted = super::rpc::accept_resource_subscriptions(requested, |uri| match parse_uri(uri) {
            Some(ParsedUri::Reference(path)) => cache.as_ref().map_or(true, |c| c.declared.contains_key(&path)),
            Some(_) => true,
            None => false,
        })?;
        accepted.prompts_list_changed = requested.prompts_list_changed;
        Some(accepted)
    }

    /// Hold the subscription stream open so notifications can ride it.
    async fn listen(&self, context: rmcp::service::SubscriptionContext) -> Result<(), rmcp::ErrorData> {
        self.subscriptions.run(context).await;
        Ok(())
    }

    /// Legacy `resources/subscribe`, honoured so `resources.subscribe:
    /// true` is truthful at every revision we negotiate. Records
    /// nothing: a peer with no `subscriptions/listen` stream already
    /// receives these notifications as broadcasts.
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
                    "Read a skill's SKILL.md body and frontmatter metadata. The result also \
                     lists, without bodies, every shared reference the skill declares (path, \
                     name, size, when it last changed) and every file it ships (uri, size). \
                     Fetch references with `read_skill_references` and files with \
                     `read_skill_files`, or pass `bundle: true` for every reference body in one \
                     call."
                        .into(),
                ),
                read_skill_object_schema(),
            ),
            Tool::new_with_raw(
                "list_skill_references",
                Some(
                    "List one skill's reference METADATA without any bodies - canonical \
                     path, name, size and when each last changed. The `path` is both the \
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
                string_array_schema(
                    "references",
                    "Canonical paths to fetch, exactly as they appear as `path` in a skill's \
                     reference manifest. A path no skill declares is an error rather than a \
                     partial result.",
                ),
            ),
            Tool::new_with_raw(
                "read_skill_files",
                Some(
                    "Fetch files a skill ships - scripts, templates, its own references - by \
                     their `skill://` uri, as listed in `read_skill`'s file manifest. One call \
                     takes files from several skills. A file that is not text is described \
                     rather than inlined."
                        .into(),
                ),
                string_array_schema(
                    "uris",
                    "`skill://<skill-path>/<file>` uris from a skill's file manifest.",
                ),
            ),
            Tool::new_with_raw(
                "reload",
                Some(
                    "Force a rescan of every skill and prompt source. The roots are WATCHED, \
                     so an edit is rescanned and announced on its own - call this only when \
                     `list_skills` reports a root degraded or off, or after editing a \
                     reference file that lives outside every configured root."
                        .into(),
                ),
                empty_object_schema(),
            ),
        ];
        Ok(ListToolsResult::with_all_items(tools)
            .with_ttl_ms(self.transport.result_ttl_ms())
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
                // needs `reload`.
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
                // Opting into the bundle replaces the references footer
                // with the real thing; otherwise the footer is what tells
                // the reader those bodies exist and how to reach them.
                let mut text = if want_bundle {
                    let bundle = wire_references::bundle(&entries);
                    append_references(&skill.body, slug, entries.len(), &bundle)
                } else {
                    format!("{}{}", skill.body, wire_references::manifest_footer(&entries, slug))
                };
                text.push_str(&files_footer(skill));
                // `body` stays the body — appending into it would change
                // the field's meaning for anything reading the structured
                // result. The concatenation is the text projection only.
                Ok(structured_with_text(
                    text,
                    serde_json::json!({
                        "uri": skill.uri(),
                        "body": skill.body,
                        "references": wire_references::manifest(&entries),
                        "files": skill.supporting().map(|f| serde_json::json!({
                            "uri": skill_file_uri(slug, &f.rel),
                            "size": f.size(),
                            "mimeType": f.mime_type(),
                        })).collect::<Vec<_>>(),
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
                // others.
                let mut paths = Vec::with_capacity(items.len());
                let mut unknown = Vec::new();
                for item in items {
                    let Some(raw) = item.as_str() else {
                        return Ok(tool_error("`references` must be an array of strings"));
                    };
                    let raw_path = path_from_file_uri(raw).unwrap_or_else(|| raw.to_string());
                    match wire_references::canonical(&raw_path).filter(|p| cache.declared.contains_key(p)) {
                        // Repeats are collapsed: a caller assembling a
                        // selection across skills that share a file must
                        // not amplify its own response.
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
            "read_skill_files" => {
                let Some(serde_json::Value::Array(items)) = args.get("uris") else {
                    return Ok(tool_error(
                        "`uris` is required and must be an array of `skill://` uris, as listed \
                         in a skill's file manifest",
                    ));
                };
                let cache = self.skills_cache.read().await;
                let mut found: Vec<(String, &BundleFile)> = Vec::new();
                let mut unknown = Vec::new();
                for item in items {
                    let Some(uri) = item.as_str() else {
                        return Ok(tool_error("`uris` must be an array of strings"));
                    };
                    match parse_uri(uri) {
                        Some(ParsedUri::Skill(path)) if cache.file(path).is_some() => {
                            if !found.iter().any(|(seen, _)| seen == uri) {
                                found.push((uri.to_string(), cache.file(path).expect("checked above")));
                            }
                        }
                        _ => unknown.push(uri.to_string()),
                    }
                }
                if !unknown.is_empty() {
                    return Ok(tool_error(format!(
                        "no skill ships {}. Pass the `uri` values from `read_skill`'s file manifest.",
                        unknown.iter().map(|u| format!("`{u}`")).collect::<Vec<_>>().join(", ")
                    )));
                }
                let text: Vec<String> = found.iter().map(|(uri, file)| file_block(uri, file)).collect();
                Ok(structured_with_text(
                    text.join("\n"),
                    serde_json::json!({
                        "files": found.iter().map(|(uri, file)| serde_json::json!({
                            "uri": uri,
                            "size": file.size(),
                            "mimeType": file.mime_type(),
                            "digest": file.digest,
                            "text": file.text(),
                        })).collect::<Vec<_>>(),
                    }),
                ))
            }
            "reload" => {
                let delta = self.reload_skills().await;
                let (count, prompts) = {
                    let cache = self.skills_cache.read().await;
                    (cache.skills.len(), cache.prompts.len())
                };
                // The SAME path the watcher relay takes, so the two
                // callers cannot disagree about a delta. A no-op reload
                // diffs to nothing and announces nothing.
                self.announce(Some(&context.peer), &delta).await;
                tracing::info!(
                    count,
                    prompts,
                    membership_changed = delta.membership_changed,
                    updated = delta.updated.len(),
                    files_changed = delta.files_changed.len(),
                    references_changed = delta.references_changed.len(),
                    prompts_changed = delta.prompts_changed.len(),
                    "mcp::server: skills reloaded"
                );
                let watch = self.watch_status.read().await;
                Ok(structured_with_text(
                    format!("Reloaded {count} skill(s) and {prompts} prompt(s)."),
                    serde_json::json!({
                        "reloaded": count,
                        "prompts": prompts,
                        "membershipChanged": delta.membership_changed,
                        "updated": delta.updated,
                        "filesChanged": delta.files_changed.iter().map(|p| format!("skill://{p}")).collect::<Vec<_>>(),
                        // Paths, by the address `read_skill_references`
                        // already takes.
                        "referencesChanged": delta.references_changed,
                        "promptsChanged": delta.prompts_changed,
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
        let mut resources = Vec::with_capacity(cache.skills.len() + cache.prompts.len() + 1);
        // The index goes FIRST — it is the entry point, and it explains
        // how to load everything under it.
        let catalogue = catalogue_markdown(&cache);
        resources.push(
            rmcp::model::Resource::new(catalogue_uri(), "skills")
                .with_title("hyprpilot skills — catalogue")
                .with_description(format!(
                    "Every available skill with its description, and how to load one. {} skill(s).",
                    cache.order.len()
                ))
                .with_mime_type("text/markdown")
                .with_size(catalogue.len() as u64),
        );
        for slug in &cache.order {
            let Some(skill) = cache.skills.get(slug) else { continue };
            let Some(skill_md) = skill.skill_md() else { continue };
            // `name` is the frontmatter name the SEP asks for, which the
            // loader guarantees is the slug's final segment.
            let name = slug.rsplit('/').next().unwrap_or(slug);
            resources.push(
                rmcp::model::Resource::new(skill.uri(), name)
                    .with_title(skill.title.clone())
                    .with_description(skill.description.clone())
                    .with_mime_type("text/markdown")
                    .with_size(skill_md.size())
                    .with_meta(skill_meta(&skill.meta_block)),
            );
            // Supporting files and references are deliberately absent.
            // Measured against a 127-skill catalogue, one extra entry per
            // skill took the listing from ~105 KB to ~170 KB, and every
            // reference would have reached ~500 KB — most of a context
            // window before a single skill is read. Each is reachable by
            // uri, by `skills/list`, and by directory read instead.
        }
        for prompt in &cache.prompts {
            let mut resource = rmcp::model::Resource::new(prompt_uri(&prompt.name), prompt.name.clone())
                .with_mime_type("text/markdown")
                .with_size(prompt.body.len() as u64);
            if let Some(title) = &prompt.title {
                resource = resource.with_title(title.clone());
            }
            if let Some(description) = &prompt.description {
                resource = resource.with_description(description.clone());
            }
            resources.push(resource);
        }
        Ok(ListResourcesResult::with_all_items(resources)
            .with_ttl_ms(self.transport.result_ttl_ms())
            .with_cache_scope(RESULT_CACHE_SCOPE))
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, rmcp::ErrorData> {
        let templates = vec![
            rmcp::model::ResourceTemplate::new("skill://{+path}", "skill-file").with_description(
                "Any file a skill ships, `skill://<skill-path>/<file>` - its `SKILL.md` included - \
                 served raw, as SEP-2640 lists it.",
            ),
        ];
        Ok(ListResourceTemplatesResult::with_all_items(templates)
            .with_ttl_ms(self.transport.result_ttl_ms())
            .with_cache_scope(RESULT_CACHE_SCOPE))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, rmcp::ErrorData> {
        let uri = &request.uri;
        let cache = self.skills_cache.read().await;
        let contents = match parse_uri(uri) {
            Some(ParsedUri::Catalogue) => ResourceContents::TextResourceContents {
                uri: uri.clone(),
                mime_type: Some("text/markdown".into()),
                text: catalogue_markdown(&cache),
                meta: None,
            },
            Some(ParsedUri::Prompt(name)) => {
                let Some(prompt) = cache.prompt(name) else {
                    return Err(rmcp::ErrorData::invalid_params(format!("unknown prompt: {name}"), None));
                };
                ResourceContents::TextResourceContents {
                    uri: uri.clone(),
                    mime_type: Some("text/markdown".into()),
                    text: prompt.body.clone(),
                    meta: None,
                }
            }
            Some(ParsedUri::Skill(path)) => {
                let Some(file) = cache.file(path) else {
                    let reason = if cache.directory(path).is_some() {
                        format!("{uri} is a directory - list it with resources/directory/read")
                    } else {
                        format!("unknown skill file: {uri}")
                    };
                    return Err(rmcp::ErrorData::invalid_params(reason, None));
                };
                // The bytes the listing hashed, from memory — never
                // re-read, so they cannot disagree with the digest.
                let contents = match file.text() {
                    Some(text) => ResourceContents::text(text, uri.clone()),
                    None => ResourceContents::blob(file.base64(), uri.clone()),
                }
                .with_mime_type(file.mime_type());
                match path.strip_suffix("/SKILL.md").and_then(|slug| cache.skills.get(slug)) {
                    Some(skill) => contents.with_meta(skill_meta(&skill.meta_block)),
                    None => contents,
                }
            }
            Some(ParsedUri::Reference(path)) => {
                let Some(path) = wire_references::canonical(&path).filter(|p| cache.declared.contains_key(p)) else {
                    return Err(rmcp::ErrorData::invalid_params(
                        format!("no skill declares {uri}"),
                        None,
                    ));
                };
                // Read per call, like every reference body: a shared
                // convention changes far more often than the skills
                // citing it.
                let text = std::fs::read_to_string(&path)
                    .map_err(|e| rmcp::ErrorData::invalid_params(format!("{uri}: {e}"), None))?;
                ResourceContents::TextResourceContents {
                    uri: uri.clone(),
                    mime_type: Some("text/markdown".into()),
                    text,
                    meta: None,
                }
            }
            None => {
                return Err(rmcp::ErrorData::invalid_params(
                    format!("unrecognised uri: {uri}"),
                    None,
                ))
            }
        };
        Ok(ReadResourceResult::new(vec![contents])
            .with_ttl_ms(self.transport.result_ttl_ms())
            .with_cache_scope(RESULT_CACHE_SCOPE)
            .into())
    }

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, rmcp::ErrorData> {
        let cache = self.skills_cache.read().await;
        let prompts = cache
            .prompts
            .iter()
            .map(|p| {
                let prompt = rmcp::model::Prompt::new(p.name.clone(), p.description.clone(), None);
                match &p.title {
                    Some(title) => prompt.with_title(title.clone()),
                    None => prompt,
                }
            })
            .collect();
        Ok(ListPromptsResult::with_all_items(prompts)
            .with_ttl_ms(self.transport.result_ttl_ms())
            .with_cache_scope(RESULT_CACHE_SCOPE))
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResponse, rmcp::ErrorData> {
        let cache = self.skills_cache.read().await;
        let Some(prompt) = cache.prompt(&request.name) else {
            return Err(rmcp::ErrorData::invalid_params(
                format!("unknown prompt: {}", request.name),
                None,
            ));
        };
        let result = GetPromptResult::new(vec![PromptMessage::new_text(Role::User, prompt.body.clone())]);
        Ok(match &prompt.description {
            Some(description) => result.with_description(description.clone()),
            None => result,
        }
        .into())
    }

    /// The SEP-2640 methods. rmcp has no model for them, so they arrive
    /// as custom requests on every transport and are answered with
    /// hand-built results — stamped by `custom_result` the way rmcp
    /// stamps its own.
    async fn on_custom_request(
        &self,
        request: CustomRequest,
        context: RequestContext<RoleServer>,
    ) -> Result<CustomResult, rmcp::ErrorData> {
        let params = request.params.as_ref();
        let modern = negotiated_modern(&context);
        let ttl = self.transport.result_ttl_ms();
        let cache = self.skills_cache.read().await;
        let mut body = serde_json::Map::new();
        match request.method.as_str() {
            "skills/list" => {
                refuse_cursor(params)?;
                body.insert(
                    "skills".into(),
                    cache
                        .order
                        .iter()
                        .filter_map(|slug| cache.skills.get(slug))
                        .map(LoadedSkill::entry)
                        .collect(),
                );
            }
            "skills/get" => {
                let uri = require_uri(params)?;
                let Some(skill) = cache.skills.values().find(|s| s.uri() == uri) else {
                    return Err(rmcp::ErrorData::invalid_params(format!("not a skill: {uri}"), None));
                };
                body.insert("skill".into(), skill.entry());
            }
            "resources/directory/read" => {
                refuse_cursor(params)?;
                let uri = require_uri(params)?;
                let children = match parse_uri(uri) {
                    Some(ParsedUri::Skill(path)) => cache.directory(path),
                    _ => None,
                }
                .ok_or_else(|| rmcp::ErrorData::invalid_params(format!("not a directory: {uri}"), None))?;
                body.insert(
                    "resources".into(),
                    serde_json::to_value(children).map_err(|e| rmcp::ErrorData::internal_error(e.to_string(), None))?,
                );
            }
            method => {
                return Err(rmcp::ErrorData::new(
                    ErrorCode::METHOD_NOT_FOUND,
                    method.to_string(),
                    None,
                ))
            }
        }
        custom_result(body, ttl, modern)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bare index URI must not shadow anything, and a missing
    /// separator is not a prompt.
    #[test]
    fn the_catalogue_uri_cannot_shadow_anything() {
        assert!(matches!(parse_uri("hyprpilot://skills"), Some(ParsedUri::Catalogue)));
        assert!(parse_uri("hyprpilot://skillsfoo").is_none());
        assert!(
            parse_uri("hyprpilot://skills/git-commit").is_none(),
            "skills moved to skill://"
        );
        assert!(parse_uri("hyprpilot://nope").is_none());
    }

    /// The index must name both ways a skill's content is reached,
    /// because a resource read cannot discover the reference tools.
    #[test]
    fn the_catalogue_explains_how_to_load_what_it_lists() {
        let empty = SkillsCache::default();
        let out = catalogue_markdown(&empty);
        assert!(out.contains("skill://<path>/SKILL.md"), "must name the body scheme");
        assert!(
            out.contains("read_skill_references"),
            "must name how references are reached"
        );
        assert!(
            out.contains("read_skill_files"),
            "must name how bundle files are reached"
        );
        assert!(out.contains("No skills available"), "an empty catalogue still renders");
    }

    #[test]
    fn parses_known_uris() {
        assert!(matches!(
            parse_uri("skill://acme/billing/refunds/SKILL.md"),
            Some(ParsedUri::Skill("acme/billing/refunds/SKILL.md"))
        ));
        assert!(matches!(
            parse_uri("skill://acme/billing/"),
            Some(ParsedUri::Skill("acme/billing"))
        ));
        assert!(matches!(
            parse_uri("hyprpilot://prompts/AGENTS"),
            Some(ParsedUri::Prompt("AGENTS"))
        ));
        assert!(matches!(
            parse_uri("file:///refs/output%20diff.md"),
            Some(ParsedUri::Reference(path)) if path == "/refs/output diff.md"
        ));
        assert!(parse_uri("skill://").is_none());
        assert!(parse_uri("hyprpilot://prompts/").is_none());
        assert!(parse_uri("hyprpilot://unknown/x").is_none());
        assert!(parse_uri("not-our-scheme://x").is_none());
    }

    /// A `BundleFile` the way `read_bundle` would build it.
    fn bundle_file(rel: &str, bytes: &str) -> BundleFile {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        std::fs::write(&path, bytes).unwrap();
        let mut file = crate::mcp::skills::wire_files::read_bundle(dir.path(), &HashMap::new())
            .unwrap()
            .remove(0);
        file.rel = rel.to_string();
        file
    }

    fn scan_of(skills: Vec<crate::mcp::skills::Skill>) -> Scan {
        Scan {
            skills: skills.into_iter().map(|skill| (skill, Vec::new())).collect(),
            prompts: Vec::new(),
        }
    }

    fn loaded_skill(slug: &str, title: &str, description: &str, frontmatter_yaml: &str, path: &str) -> LoadedSkill {
        let frontmatter: yaml_serde::Value = yaml_serde::from_str(frontmatter_yaml).unwrap();
        let path = PathBuf::from(path);
        LoadedSkill {
            slug: slug.to_string(),
            meta_block: skill_block(&frontmatter_json(&frontmatter), &path),
            frontmatter: frontmatter_json(&frontmatter),
            path,
            title: title.to_string(),
            description: description.to_string(),
            body: String::new(),
            refs: frontmatter_references(&frontmatter),
            files: Vec::new(),
        }
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

    /// The `watch` payload and the summary line are what tell an
    /// MCP-only client it needs `reload`, and nothing asserted either.
    #[tokio::test]
    async fn list_skills_reports_watch_coverage_in_both_shapes() {
        let covered = crate::watch::WatchStatus {
            roots: vec![crate::watch::RootWatch {
                dir: std::path::PathBuf::from("/a"),
                state: crate::watch::WatchState::Watching,
            }],
        };
        assert_eq!(watch_payload(&covered)["active"], serde_json::Value::Bool(true));
        assert!(covered.summary_line().is_none(), "a covered root says nothing");

        let mut partial = covered.clone();
        partial.roots.push(crate::watch::RootWatch {
            dir: std::path::PathBuf::from("/b"),
            state: crate::watch::WatchState::Off,
        });
        let payload = watch_payload(&partial);
        assert_eq!(payload["active"], serde_json::Value::Bool(false));
        assert_eq!(payload["roots"].as_array().map(Vec::len), Some(2));
        assert_eq!(payload["roots"][1]["state"], serde_json::Value::from("off"));
        assert!(
            partial.summary_line().is_some_and(|l| l.contains("/b")),
            "an uncovered root must reach a text-only client"
        );
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

    /// notify joins a RELATIVE watch path onto the process cwd while we
    /// would keep the relative form, so every event would fail
    /// `strip_prefix` and be dropped - a root that reports `watching`
    /// and never fires. Only a hand-written catalogue entry can produce
    /// one, which is exactly the case with no launcher to fix it.
    #[test]
    fn a_relative_skill_dir_is_absolutized() {
        let server = SkillsServer::new(
            SkillsArgs {
                serve: Default::default(),
                skill_dirs: vec![SkillDirEntry {
                    dir: std::path::PathBuf::from("./relative-skills"),
                    ignore: Vec::new(),
                    include: Vec::new(),
                    watch: true,
                }],
                prompt_dirs: Vec::new(),
                prompt_files: Vec::new(),
            },
            crate::mcp::server::ConfigSource::default(),
        )
        .expect("build skills server");

        let dir = &server.registry.dirs()[0].dir;
        assert!(dir.is_absolute(), "a relative root would never match an event: {dir:?}");
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
                    raw_modified: None,
                },
            },
        );
    }

    /// A shared convention file changes; no skill moved. The raw
    /// `SKILL.md` of every citer is byte-identical, so the announcement
    /// is the reference's own `file://` uri — and `list_changed`, the
    /// only signal a client that cannot subscribe has.
    #[test]
    fn a_reference_edit_is_announced_by_its_own_uri() {
        let mut before = cache_of(&[("alpha", "same"), ("beta", "same")]);
        cite(&mut before, "/refs/output-diff.md", "t1", &["alpha", "beta"]);
        let mut after = cache_of(&[("alpha", "same"), ("beta", "same")]);
        cite(&mut after, "/refs/output-diff.md", "t2", &["alpha", "beta"]);

        let delta = CatalogueDelta::between(&before, &after);
        assert!(delta.updated.is_empty(), "no skill moved");
        assert!(!delta.membership_changed);
        assert_eq!(delta.references_changed, vec!["/refs/output-diff.md".to_string()]);

        let plan = delta.plan();
        assert!(plan.resources_list_changed, "an older client has no other signal");
        assert_eq!(plan.updated, vec![file_uri("/refs/output-diff.md")]);
        assert!(
            !plan.updated.contains(&catalogue_uri()),
            "the index renders a count, not content"
        );
    }

    /// A body edit DOES stale the index — it renders the description,
    /// which frontmatter can move without touching membership.
    #[test]
    fn a_body_edit_stales_the_catalogue_index() {
        let plan = CatalogueDelta::between(&cache_of(&[("alpha", "v1")]), &cache_of(&[("alpha", "v2")])).plan();
        assert_eq!(plan.updated, vec![skill_md_uri("alpha"), catalogue_uri()]);
    }

    /// Appearing and vanishing are both changes in served content: a
    /// citer's manifest row flips between a path and `status: not-found`.
    #[test]
    fn a_reference_appearing_or_vanishing_is_a_change() {
        let before = cache_of(&[("alpha", "same")]);
        let mut after = cache_of(&[("alpha", "same")]);
        cite(&mut after, "/refs/new.md", "t1", &["alpha"]);

        assert_eq!(
            CatalogueDelta::between(&before, &after).references_changed,
            vec!["/refs/new.md".to_string()]
        );
        assert_eq!(
            CatalogueDelta::between(&after, &before).references_changed,
            vec!["/refs/new.md".to_string()]
        );
    }

    /// One URI is announced once, however many reasons it has. A file a
    /// skill shares with the skill nested inside it is one path.
    #[test]
    fn a_uri_with_several_reasons_is_announced_once() {
        let mut before = cache_of(&[("alpha", "v1")]);
        let mut after = cache_of(&[("alpha", "v2")]);
        before
            .files
            .insert("alpha/SKILL.md".into(), bundle_file("SKILL.md", "v1"));
        after
            .files
            .insert("alpha/SKILL.md".into(), bundle_file("SKILL.md", "v2"));

        let plan = CatalogueDelta::between(&before, &after).plan();
        assert_eq!(plan.updated, vec![skill_md_uri("alpha"), catalogue_uri()]);
    }

    /// The bundle-file gap: a script changes, the skill's text does not.
    /// Its uri is announced, and so is the skill — its `resources` set
    /// (and with it the host's approval) is what changed.
    #[test]
    fn a_bundle_file_edit_announces_the_file_and_its_skill() {
        let mut before = cache_of(&[("alpha", "same")]);
        let mut after = cache_of(&[("alpha", "same")]);
        before.skills.get_mut("alpha").unwrap().files = vec![bundle_file("scripts/run.py", "v1")];
        after.skills.get_mut("alpha").unwrap().files = vec![bundle_file("scripts/run.py", "v2")];
        before
            .files
            .insert("alpha/scripts/run.py".into(), bundle_file("scripts/run.py", "v1"));
        after
            .files
            .insert("alpha/scripts/run.py".into(), bundle_file("scripts/run.py", "v2"));

        let delta = CatalogueDelta::between(&before, &after);
        assert_eq!(delta.updated, vec!["alpha".to_string()]);
        assert_eq!(delta.files_changed, vec!["alpha/scripts/run.py".to_string()]);
        assert!(delta
            .plan()
            .updated
            .contains(&"skill://alpha/scripts/run.py".to_string()));
    }

    /// MCP has no per-prompt update notification, so an edited BODY is
    /// a prompt list change — the only way a client learns to re-fetch.
    #[test]
    fn a_prompt_edit_is_a_prompt_list_change() {
        let prompt = |body: &str| Prompt {
            name: "AGENTS".into(),
            title: None,
            description: None,
            body: body.into(),
            path: PathBuf::from("/p/AGENTS.md"),
        };
        let mut before = cache_of(&[]);
        let mut after = cache_of(&[]);
        before.prompts.push(prompt("v1"));
        after.prompts.push(prompt("v2"));

        let plan = CatalogueDelta::between(&before, &after).plan();
        assert!(plan.prompts_list_changed);
        assert_eq!(plan.updated, vec![prompt_uri("AGENTS")]);

        let same = CatalogueDelta::between(&after, &after);
        assert!(same.is_empty() && !same.plan().prompts_list_changed);
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

    /// The silent-staleness pin. `modified` is truncated to SECONDS for
    /// the wire, so comparing on it made two same-length edits inside
    /// one second identical - the rescan diffed to nothing and every
    /// citer stayed stale for the full 24h ttl with no notification.
    /// `FileStat` therefore compares the RAW mtime.
    #[test]
    fn two_edits_inside_one_second_are_still_two_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shared.md");

        std::fs::write(&path, "aaaa").unwrap();
        let before = crate::mcp::skills::wire_time::FileStat::read(&path);
        // Same length, so `size` cannot distinguish them either.
        std::fs::write(&path, "bbbb").unwrap();
        let after = crate::mcp::skills::wire_time::FileStat::read(&path);

        assert_eq!(before.size, after.size, "the fixture must not differ by size");
        assert_ne!(before, after, "a same-second same-size edit must still register");
        // And the wire form stays human-readable seconds.
        assert!(after.modified.as_deref().is_some_and(|m| m.ends_with('Z')));
    }

    /// A frontmatter-only edit changes `title` / `description` /
    /// `references`, all of which `skill_block` STRIPS because other
    /// fields carry them. Detecting it must not depend on the SKILL.md
    /// mtime happening to land in a different second.
    #[test]
    fn a_description_only_edit_updates_the_skill() {
        let mut before = cache_of(&[("alpha", "same")]);
        let mut after = cache_of(&[("alpha", "same")]);
        after.skills.get_mut("alpha").unwrap().description = "a new description".to_string();

        let delta = CatalogueDelta::between(&before, &after);
        assert_eq!(delta.updated, vec!["alpha".to_string()]);

        // Same for the declared-reference list.
        before = cache_of(&[("beta", "same")]);
        let mut after2 = cache_of(&[("beta", "same")]);
        after2.skills.get_mut("beta").unwrap().refs.references = vec!["../refs/new.md".to_string()];
        assert_eq!(
            CatalogueDelta::between(&before, &after2).updated,
            vec!["beta".to_string()]
        );
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

        let cache = build_cache(scan_of(skills));
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

        let cache = build_cache(scan_of(vec![skill]));
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

        let cache = build_cache(scan_of(vec![skill]));
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

        let cache = build_cache(scan_of(vec![skill]));
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
                    "uri": "skill://plan-hard/SKILL.md",
                    // A count, not the names: this view is served
                    // purely from cache, and resolving names would mean
                    // reading every reference of every skill per call.
                    "referenceCount": 1,
                    "fileCount": 0,
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

    /// The SEP-2640 entry: the frontmatter VERBATIM (including the keys
    /// the metadata block strips) and every file with its digest and
    /// size, `SKILL.md` among them.
    #[test]
    fn a_skills_list_entry_is_the_spec_shape() {
        let mut skill = loaded_skill(
            "acme/refunds",
            "Refunds",
            "Process refunds",
            "name: refunds\ndescription: Process refunds\nlicense: MIT\n",
            "/tmp/acme/refunds/SKILL.md",
        );
        skill.files = vec![bundle_file("SKILL.md", "raw"), bundle_file("scripts/x.py", "x")];
        let entry = skill.entry();

        assert_eq!(entry["uri"], "skill://acme/refunds/SKILL.md");
        assert_eq!(
            entry["frontmatter"],
            serde_json::json!({ "name": "refunds", "description": "Process refunds", "license": "MIT" })
        );
        assert_eq!(entry["resources"][0]["uri"], "skill://acme/refunds/SKILL.md");
        assert_eq!(entry["resources"][1]["uri"], "skill://acme/refunds/scripts/x.py");
        assert_eq!(entry["resources"][1]["size"], 1);
        assert_eq!(
            entry["resources"][1]["digest"],
            crate::mcp::skills::wire_files::digest(b"x")
        );
        assert_eq!(entry["digest"], entry["resources"][0]["digest"]);
    }

    /// Every directory in the namespace lists its direct children —
    /// a skill root, a subdirectory and an organizational prefix alike —
    /// and a file or an unknown path is not a directory.
    #[test]
    fn a_directory_read_lists_direct_children_only() {
        let mut cache = SkillsCache::default();
        for (path, body) in [
            ("acme/refunds/SKILL.md", "s"),
            ("acme/refunds/templates/invoice.md", "i"),
            ("acme/refunds/templates/regional/eu.md", "e"),
            ("acme/other/SKILL.md", "o"),
        ] {
            let rel = path.rsplit('/').next().unwrap();
            cache.files.insert(path.to_string(), bundle_file(rel, body));
        }
        let names = |path: &str| -> Vec<(String, Option<String>)> {
            cache
                .directory(path)
                .unwrap()
                .into_iter()
                .map(|r| (r.uri.clone(), r.mime_type.clone()))
                .collect()
        };

        assert_eq!(
            names("acme/refunds/templates"),
            vec![
                (
                    "skill://acme/refunds/templates/invoice.md".to_string(),
                    Some("text/markdown".to_string())
                ),
                (
                    "skill://acme/refunds/templates/regional".to_string(),
                    Some("inode/directory".to_string())
                ),
            ]
        );
        assert_eq!(names("acme").len(), 2, "an organizational prefix is a directory");
        assert!(
            cache.directory("acme/refunds/SKILL.md").is_none(),
            "a file is not a directory"
        );
        assert!(cache.directory("nope").is_none());
        // `acme/ref` is a string prefix of `acme/refunds`, not a directory.
        assert!(cache.directory("acme/ref").is_none());
    }

    /// A single page, so there is no cursor this server issued.
    #[test]
    fn a_cursor_is_refused() {
        assert!(refuse_cursor(None).is_ok());
        assert!(refuse_cursor(Some(&serde_json::json!({ "cursor": null }))).is_ok());
        let err = refuse_cursor(Some(&serde_json::json!({ "cursor": "x" }))).unwrap_err();
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
    }

    /// rmcp strips `resultType` from its own results for an older peer
    /// and passes a custom result through untouched, so ours must make
    /// the same call — and always carry the cache stamps.
    #[test]
    fn a_custom_result_carries_result_type_only_when_modern() {
        let legacy = custom_result(serde_json::Map::new(), 5, false).unwrap().0;
        assert!(legacy.get("resultType").is_none());
        assert_eq!(legacy["ttlMs"], 5);
        assert_eq!(legacy["cacheScope"], "private");

        let modern = custom_result(serde_json::Map::new(), 5, true).unwrap().0;
        assert_eq!(modern["resultType"], "complete");
    }
}
#[cfg(test)]
mod watch_tests {
    use super::{SkillDirEntry, SkillsArgs, SkillsServer};
    use rmcp::ServiceExt;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    const META: &str = r#""_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientInfo":{"name":"t","version":"1"},"io.modelcontextprotocol/clientCapabilities":{}}"#;

    fn write_skill(root: &std::path::Path, slug: &str, body: &str, refs: &str) {
        let bundle = root.join(slug);
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(
            bundle.join("SKILL.md"),
            format!("---\nname: {slug}\ndescription: d\n{refs}---\n\n# {slug}\n\n{body}\n"),
        )
        .unwrap();
    }

    /// Serve a real skills server over a duplex with the watcher armed
    /// and the relay running, exactly as `run()` wires it: arm, scan,
    /// serve, then spawn the relay with the peer. `serve` returns once
    /// the connection's first request has arrived, so the client's
    /// `opener` is written before it.
    async fn serve_watched(
        root: &std::path::Path,
        opener: &str,
    ) -> (
        tokio::io::DuplexStream,
        tokio::io::Lines<BufReader<tokio::io::DuplexStream>>,
        Option<crate::watch::Watcher>,
        tokio::task::JoinHandle<()>,
        rmcp::service::RunningService<rmcp::service::RoleServer, SkillsServer>,
    ) {
        let handler = SkillsServer::new(
            SkillsArgs {
                serve: Default::default(),
                skill_dirs: vec![SkillDirEntry {
                    dir: root.to_path_buf(),
                    ignore: Vec::new(),
                    include: Vec::new(),
                    watch: true,
                }],
                prompt_dirs: Vec::new(),
                prompt_files: Vec::new(),
            },
            crate::mcp::server::ConfigSource::default(),
        )
        .expect("build skills server");

        let (watcher, signals) = handler.arm_watch(std::time::Duration::from_millis(50)).await;
        let _ = handler.reload_skills().await;
        let relay_server = handler.clone();

        let (mut client_tx, server_rx) = tokio::io::duplex(1 << 16);
        let (server_tx, client_rx) = tokio::io::duplex(1 << 16);
        client_tx.write_all(opener.as_bytes()).await.unwrap();
        client_tx.flush().await.unwrap();
        let running = handler.serve((server_rx, server_tx)).await.expect("serve");
        let relay = tokio::spawn(relay_server.relay_watch(signals, Some(running.peer().clone())));

        (client_tx, BufReader::new(client_rx).lines(), watcher, relay, running)
    }

    /// A `subscriptions/listen` request for `uris` — the opener a v2
    /// client sends.
    fn listen(uris: &[&str]) -> String {
        let subs = serde_json::to_string(uris).unwrap();
        format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":\"l\",\"method\":\"subscriptions/listen\",\"params\":{{{META},\"notifications\":{{\"resourcesListChanged\":true,\"resourceSubscriptions\":{subs}}}}}}}\n"
        )
    }

    /// Wait for the listen acknowledgment, so the edit that follows
    /// cannot race the subscription.
    async fn acknowledged(lines: &mut tokio::io::Lines<BufReader<tokio::io::DuplexStream>>) {
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
        let (client_tx, mut lines, watcher, relay, running) =
            serve_watched(root.path(), &listen(&["skill://alpha/SKILL.md"])).await;
        acknowledged(&mut lines).await;

        write_skill(root.path(), "alpha", "v2 edited", "");

        let seen = collect_until(&mut lines, |seen| {
            updated_for(seen, "skill://alpha/SKILL.md")
                && seen.iter().any(|l| l.contains("notifications/resources/list_changed"))
        })
        .await;
        assert!(
            updated_for(&seen, "skill://alpha/SKILL.md"),
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

        let shared = std::fs::canonicalize(root.path().join("references/shared.md")).unwrap();
        let shared_uri = crate::mcp::skills::wire_references::file_uri(&shared.display().to_string());
        let (client_tx, mut lines, watcher, relay, running) =
            serve_watched(root.path(), &listen(&[&shared_uri, "skill://alpha/SKILL.md"])).await;
        acknowledged(&mut lines).await;

        // Only the reference moves. The skill body is untouched.
        std::fs::write(root.path().join("references/shared.md"), "v2 edited").unwrap();

        let seen = collect_until(&mut lines, |seen| updated_for(seen, &shared_uri)).await;
        assert!(
            updated_for(&seen, &shared_uri),
            "a reference edit reached its subscriber as silence: {seen:?}"
        );
        assert!(
            !updated_for(&seen, "skill://alpha/SKILL.md"),
            "the citing skill's raw SKILL.md did not change: {seen:?}"
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
        let (client_tx, mut lines, watcher, relay, running) = serve_watched(
            root.path(),
            &format!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\",\"params\":{{{META}}}}}\n"),
        )
        .await;
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
        let (client_tx, mut lines, watcher, relay, running) =
            serve_watched(root.path(), &listen(&["skill://alpha/SKILL.md"])).await;
        acknowledged(&mut lines).await;

        std::fs::write(root.path().join("alpha/.SKILL.md.swp"), "editor scratch").unwrap();

        let quiet = tokio::time::timeout(std::time::Duration::from_millis(1500), lines.next_line()).await;
        assert!(quiet.is_err(), "editor noise produced a notification: {quiet:?}");

        relay.abort();
        drop(watcher);
        drop(client_tx);
        running.cancel().await.ok();
    }

    /// A prompt file — the profile's system prompt — is watched through
    /// its PARENT directory, so an editor's atomic save (a new inode)
    /// still announces it as a prompt list change.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_edited_prompt_file_announces_a_prompt_list_change() {
        let prompts = tempfile::tempdir().unwrap();
        let file = prompts.path().join("AGENTS.md");
        std::fs::write(&file, "v1").unwrap();

        let handler = SkillsServer::new(
            SkillsArgs {
                serve: Default::default(),
                skill_dirs: Vec::new(),
                prompt_dirs: Vec::new(),
                prompt_files: vec![file.clone()],
            },
            crate::mcp::server::ConfigSource::default(),
        )
        .expect("build skills server");
        let (watcher, signals) = handler.arm_watch(std::time::Duration::from_millis(50)).await;
        let _ = handler.reload_skills().await;
        let relay_server = handler.clone();
        let (mut client_tx, server_rx) = tokio::io::duplex(1 << 16);
        let (server_tx, client_rx) = tokio::io::duplex(1 << 16);
        client_tx
            .write_all(
                format!(
                    "{{\"jsonrpc\":\"2.0\",\"id\":\"l\",\"method\":\"subscriptions/listen\",\"params\":{{{META},\"notifications\":{{\"promptsListChanged\":true,\"resourceSubscriptions\":[\"hyprpilot://prompts/AGENTS\"]}}}}}}\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let running = handler.serve((server_rx, server_tx)).await.expect("serve");
        let relay = tokio::spawn(relay_server.relay_watch(signals, Some(running.peer().clone())));
        let mut lines = BufReader::new(client_rx).lines();
        acknowledged(&mut lines).await;

        // An atomic save: write beside, rename over.
        let tmp = prompts.path().join(".AGENTS.md.tmp");
        std::fs::write(&tmp, "v2 edited").unwrap();
        std::fs::rename(&tmp, &file).unwrap();

        let seen = collect_until(&mut lines, |seen| {
            seen.iter().any(|l| l.contains("notifications/prompts/list_changed"))
                && updated_for(seen, "hyprpilot://prompts/AGENTS")
        })
        .await;
        assert!(
            seen.iter().any(|l| l.contains("notifications/prompts/list_changed")),
            "no prompt list change reached the client: {seen:?}"
        );
        assert!(updated_for(&seen, "hyprpilot://prompts/AGENTS"), "{seen:?}");

        relay.abort();
        drop(watcher);
        drop(client_tx);
        running.cancel().await.ok();
    }
}
#[cfg(test)]
mod opener_tests {
    use super::{SkillsArgs, SkillsServer};
    use rmcp::ServiceExt;
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
            SkillsArgs {
                skill_dirs: Vec::new(),
                serve: Default::default(),
                prompt_dirs: Vec::new(),
                prompt_files: Vec::new(),
            },
            crate::mcp::server::ConfigSource::default(),
        )
        .expect("build skills server");

        let (mut client_tx, server_rx) = tokio::io::duplex(1 << 16);
        let (server_tx, client_rx) = tokio::io::duplex(1 << 16);

        client_tx.write_all(opener.as_bytes()).await.unwrap();
        client_tx
            .write_all(
                format!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\",\"params\":{{{META}}}}}\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        client_tx.flush().await.unwrap();
        let running = handler.serve((server_rx, server_tx)).await.expect("serve");

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
    /// what it asked for, or a client told `2025-11-25` still receives
    /// `2026-07-28` result shapes — and one validating the revision it
    /// agreed rejects the listing, which is the same failure the `ttlMs`
    /// stamp exists for.
    ///
    /// Both requests negotiate down. An unsupported one obviously; the
    /// current revision because sending `initialize` IS the choice of
    /// legacy semantics — `2026-07-28` replaced the handshake with
    /// per-request metadata, so echoing it would tell the client it
    /// agreed result shapes it never asked to parse.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_down_negotiated_session_is_not_served_a_newer_result_shape() {
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

        for requested in ["2099-01-01", "2026-07-28"] {
            let handler = SkillsServer::new(
                SkillsArgs {
                    skill_dirs: Vec::new(),
                    serve: Default::default(),
                    prompt_dirs: Vec::new(),
                    prompt_files: Vec::new(),
                },
                crate::mcp::server::ConfigSource::default(),
            )
            .expect("build skills server");
            let (mut client_tx, server_rx) = tokio::io::duplex(1 << 16);
            let (server_tx, client_rx) = tokio::io::duplex(1 << 16);
            let mut reader = BufReader::new(client_rx).lines();

            client_tx
                .write_all(
                    format!("{{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"initialize\",\"params\":{{\"protocolVersion\":\"{requested}\",\"capabilities\":{{}},\"clientInfo\":{{\"name\":\"t\",\"version\":\"1\"}}}}}}\n")
                        .as_bytes(),
                )
                .await
                .unwrap();
            client_tx.flush().await.unwrap();
            let running = handler.serve((server_rx, server_tx)).await.expect("serve");
            let init = next_json(&mut reader).await;
            assert_eq!(
                init["result"]["protocolVersion"], "2025-11-25",
                "an `initialize` naming {requested} negotiates down"
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
    }

    /// The regression. Claude Code's v2 runtime probes `server/discover`
    /// on a DISPOSABLE second process, then opens the real transport
    /// with `subscriptions/listen` as its first request — so for a
    /// server implementing subscriptions this ordering is the normal
    /// path, not an edge case.
    ///
    /// Before rmcp 3.4 the pre-loop handshake deadlocked on it: the
    /// opener was handled inline, its acknowledgement awaits a oneshot
    /// only the serve loop fires, and the loop was not spawned until the
    /// opener returned. Zero bytes out, forever — which a client reports
    /// as "connected, tools fetch failed". This pins that `serve` keeps
    /// dispatching the opener from inside the loop.
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

#[cfg(test)]
mod sep_tests {
    use super::{SkillDirEntry, SkillsArgs, SkillsServer};
    use rmcp::ServiceExt;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    const META: &str = r#""_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientInfo":{"name":"t","version":"1"},"io.modelcontextprotocol/clientCapabilities":{}}"#;

    /// A root with a skill shipping a script, and a skill nested under
    /// an organizational prefix.
    fn seed() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let alpha = root.path().join("alpha");
        std::fs::create_dir_all(alpha.join("scripts")).unwrap();
        std::fs::write(
            alpha.join("SKILL.md"),
            "---\nname: alpha\ndescription: a\n---\n\nalpha body\n",
        )
        .unwrap();
        std::fs::write(alpha.join("scripts/run.py"), "print('hi')\n").unwrap();
        let refunds = root.path().join("acme/refunds");
        std::fs::create_dir_all(&refunds).unwrap();
        std::fs::write(
            refunds.join("SKILL.md"),
            "---\nname: refunds\ndescription: r\n---\n\nrefunds body\n",
        )
        .unwrap();
        root
    }

    /// Serve the seeded root, send `requests` (one JSON-RPC line each,
    /// ids 1..), and collect each id's reply.
    async fn exchange(root: &std::path::Path, opener: Option<&str>, requests: &[String]) -> Vec<serde_json::Value> {
        let handler = SkillsServer::new(
            SkillsArgs {
                serve: Default::default(),
                skill_dirs: vec![SkillDirEntry {
                    dir: root.to_path_buf(),
                    ignore: Vec::new(),
                    include: Vec::new(),
                    watch: false,
                }],
                prompt_dirs: Vec::new(),
                prompt_files: Vec::new(),
            },
            crate::mcp::server::ConfigSource::default(),
        )
        .expect("build skills server");
        let _ = handler.reload_skills().await;

        let (mut client_tx, server_rx) = tokio::io::duplex(1 << 20);
        let (server_tx, client_rx) = tokio::io::duplex(1 << 20);
        if let Some(opener) = opener {
            client_tx.write_all(format!("{opener}\n").as_bytes()).await.unwrap();
        }
        for (i, request) in requests.iter().enumerate() {
            let line = request.replacen('{', &format!("{{\"jsonrpc\":\"2.0\",\"id\":{},", i + 1), 1);
            client_tx.write_all(format!("{line}\n").as_bytes()).await.unwrap();
        }
        client_tx.flush().await.unwrap();
        let running = handler.serve((server_rx, server_tx)).await.expect("serve");

        let mut replies = std::collections::BTreeMap::new();
        let mut lines = BufReader::new(client_rx).lines();
        while replies.len() < requests.len() {
            let line = tokio::time::timeout(std::time::Duration::from_secs(5), lines.next_line())
                .await
                .expect("a reply within the bound")
                .unwrap()
                .expect("the stream stays open");
            let value: serde_json::Value = serde_json::from_str(&line).unwrap();
            if let Some(id) = value.get("id").and_then(serde_json::Value::as_u64) {
                replies.insert(id, value);
            }
        }
        drop(client_tx);
        running.cancel().await.ok();
        replies.into_values().collect()
    }

    fn modern(method: &str, params: &str) -> String {
        format!("{{\"method\":\"{method}\",\"params\":{{{META}{params}}}}}")
    }

    /// The host side of the contract, end to end: list, fetch the raw
    /// `SKILL.md` and a script, and check each against the digest and
    /// size the listing promised.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn every_listed_file_reads_back_to_its_digest() {
        let root = seed();
        let replies = exchange(
            root.path(),
            None,
            &[
                modern("skills/list", ""),
                modern("resources/read", r#","uri":"skill://alpha/SKILL.md""#),
                modern("resources/read", r#","uri":"skill://alpha/scripts/run.py""#),
            ],
        )
        .await;

        let list = &replies[0]["result"];
        assert_eq!(list["resultType"], "complete");
        assert!(list["ttlMs"].is_u64());
        let skills = list["skills"].as_array().expect("skills");
        let uris: Vec<&str> = skills.iter().filter_map(|s| s["uri"].as_str()).collect();
        assert_eq!(uris, ["skill://acme/refunds/SKILL.md", "skill://alpha/SKILL.md"]);

        let alpha = &skills[1];
        assert_eq!(
            alpha["frontmatter"],
            serde_json::json!({ "name": "alpha", "description": "a" })
        );
        for (reply, listed) in replies[1..].iter().zip(alpha["resources"].as_array().unwrap()) {
            let contents = &reply["result"]["contents"][0];
            assert_eq!(contents["uri"], listed["uri"]);
            let text = contents["text"].as_str().expect("text content");
            assert_eq!(text.len() as u64, listed["size"].as_u64().unwrap());
            assert_eq!(
                crate::mcp::skills::wire_files::digest(text.as_bytes()),
                listed["digest"].as_str().unwrap()
            );
        }
        assert!(
            replies[1]["result"]["contents"][0]["text"]
                .as_str()
                .unwrap()
                .starts_with("---\nname: alpha"),
            "SKILL.md is served raw, frontmatter included"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn skills_get_and_directory_read_answer_by_uri() {
        let root = seed();
        let replies = exchange(
            root.path(),
            None,
            &[
                modern("skills/get", r#","uri":"skill://acme/refunds/SKILL.md""#),
                modern("skills/get", r#","uri":"skill://nope/SKILL.md""#),
                modern("resources/directory/read", r#","uri":"skill://acme""#),
                modern("resources/directory/read", r#","uri":"skill://alpha/SKILL.md""#),
            ],
        )
        .await;

        assert_eq!(replies[0]["result"]["skill"]["frontmatter"]["name"], "refunds");
        assert_eq!(replies[1]["error"]["code"], -32602, "an unknown skill: {}", replies[1]);
        assert_eq!(
            replies[2]["result"]["resources"],
            serde_json::json!([{ "uri": "skill://acme/refunds", "name": "refunds", "mimeType": "inode/directory" }])
        );
        assert_eq!(replies[3]["error"]["code"], -32602, "a file is not a directory");
    }

    /// The declared capability is the host's only way to know the
    /// methods exist — and a session that agreed an older revision must
    /// not be handed the newer result shape.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_legacy_session_sees_the_extension_without_result_type() {
        let root = seed();
        let replies = exchange(
            root.path(),
            None,
            &[
                r#"{"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#.to_string(),
                r#"{"method":"skills/list","params":{}}"#.to_string(),
            ],
        )
        .await;

        assert_eq!(
            replies[0]["result"]["capabilities"]["extensions"]["io.modelcontextprotocol/skills"],
            serde_json::json!({ "directoryRead": true })
        );
        assert_eq!(replies[0]["result"]["capabilities"]["prompts"]["listChanged"], true);
        assert!(replies[1]["result"].get("resultType").is_none(), "{}", replies[1]);
        assert_eq!(replies[1]["result"]["skills"].as_array().map(Vec::len), Some(2));
    }
}

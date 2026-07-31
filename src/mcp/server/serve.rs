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
//!   - `hyprpilot://references/<slug>` — bundled references
//!     (parallel top-level scheme, NOT a `/references` segment
//!     nested under the slug — the nested form broke client URI
//!     autocomplete)
//!   - Both carry ONE namespaced `_meta` key, `io.hyprpilot/skill`:
//!     the verbatim frontmatter MINUS `title`/`description` (already
//!     carried by the spec `Resource` fields) PLUS the runtime-derived
//!     `path` + `bundleDir`. Nothing in that block repeats a
//!     spec-compliant `Resource` field. See `skills/metadata.rs`.
//! - Tools
//!   - `list_skills` — `{ skills: [{ slug, title, description, uri, metadata }] }`
//!   - `read_skill { slug }` — `{ uri, body, metadata }`
//!   - `load_skill_references { slug }` — `{ uri, body, metadata }`
//!   - `reload` — rescan dirs, push a resource list-changed
//!     notification (skills back the resource list; the tool list is
//!     static, so no tool-list-changed fires)
//!   - `open { path }` — open a URL, file, or directory in the
//!     OS-default handler (`xdg-open` / `open` / `start`) via the
//!     cross-platform `open` crate.
//!
//! Metadata is de-duplicated to a SINGLE block (`metadata` in tool
//! output, `io.hyprpilot/skill` in resource `_meta`): the WHOLE parsed
//! YAML frontmatter projected losslessly to JSON, minus the two keys
//! (`title`/`description`) the spec fields already carry byte-for-byte,
//! plus the runtime-derived `path` + `bundleDir`. An author can add any
//! new frontmatter key and it reaches the agent verbatim with zero
//! server changes. `skills/metadata.rs` owns the conversion + the
//! merge + the `_meta` namespacing; this module wires it into the cache
//! + the wire shapes.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use clap::Args;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ContentBlock, ErrorCode, Implementation, ListResourceTemplatesResult,
    ListResourcesResult, ListToolsResult, PaginatedRequestParams, ReadResourceRequestParams, ReadResourceResult,
    ResourceContents, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::ServerHandler;
use rmcp::ServiceExt;
use tokio::sync::RwLock;

use crate::config::ResolvedSkillEntry;
use crate::mcp::auto_inject::SKILLS_SERVER_NAME;
use crate::skills::SkillsRegistry;

use super::skills::metadata::{frontmatter_json, skill_block, skill_meta};
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

    /// Expose the agent harness (`list_profiles`, `spawn`, `resume`,
    /// `session_*`) alongside the skills surface.
    ///
    /// **Off by default, deliberately.** `mcp::auto_inject` puts this
    /// sidecar inside EVERY launch, so an ungated spawn surface would
    /// let a claude session spawn nested claude sessions without bound.
    /// It is also a security boundary: a profile's `command` is an
    /// arbitrary binary, so anything that can call `spawn` can execute
    /// commands as this user. Enable it only where that is intended —
    /// e.g. a gateway host whose MCP config opts in explicitly.
    #[arg(long = "with-harness", default_value_t = false)]
    pub with_harness: bool,
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
pub async fn run(args: ServeArgs, config: super::ConfigSource) -> anyhow::Result<()> {
    tracing::info!(
        dirs = args.skill_dirs.len(),
        harness = args.with_harness,
        "mcp::server::serve: starting hyprpilot MCP server"
    );

    if args.with_harness {
        // Reclaim anything a crashed predecessor left behind before
        // starting our own. A non-empty sweep logs at `warn`.
        super::sessions::sweep_stale_sessions();
    }

    let handler = HyprpilotServer::new(args, config)?;
    handler.reload_skills().await;

    // Clone the session table BEFORE `serve()` — it consumes the
    // handler, and `waiting()` consumes the `RunningService`, so this is
    // the only chance to keep a handle for the shutdown reap.
    let sessions = handler.harness.as_ref().map(|harness| Arc::clone(&harness.sessions));

    let (stdin, stdout) = rmcp::transport::io::stdio();
    let running = handler
        .serve((stdin, stdout))
        .await
        .context("mcp::server::serve: serve failed at init")?;

    // Race the transport against SIGTERM/SIGHUP. Without this a
    // supervisor stopping the sidecar would skip every destructor and
    // strand live sessions — `PR_SET_PDEATHSIG` still covers the
    // SIGKILL case, but only after the kernel notices, and it cannot
    // remove the session directories.
    wait_for_shutdown(running).await;

    if let Some(sessions) = sessions {
        sessions.shutdown().await;
    }

    Ok(())
}

/// Return once the MCP transport closes or a termination signal
/// arrives, whichever comes first.
async fn wait_for_shutdown(running: rmcp::service::RunningService<RoleServer, HyprpilotServer>) {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        let mut term = signal(SignalKind::terminate()).ok();
        let mut hup = signal(SignalKind::hangup()).ok();
        let transport = running.waiting();
        tokio::pin!(transport);

        // Every arm is terminal — the first of transport-close, SIGTERM,
        // or SIGHUP wins and the caller reaps.
        tokio::select! {
            _ = &mut transport => {}
            Some(()) = async { match term.as_mut() { Some(s) => s.recv().await, None => None } } => {
                tracing::info!("mcp::server::serve: SIGTERM received; reaping sessions");
            }
            Some(()) = async { match hup.as_mut() { Some(s) => s.recv().await, None => None } } => {
                tracing::info!("mcp::server::serve: SIGHUP received; reaping sessions");
            }
        }
    }
    #[cfg(not(unix))]
    {
        running.waiting().await.ok();
    }
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
    /// `skills::metadata::{skill_block, skill_meta}`.
    pub(crate) meta_block: serde_json::Map<String, serde_json::Value>,
    body: String,
    refs: FrontmatterRefs,
}

impl LoadedSkill {
    fn bundle_dir(&self) -> Option<&std::path::Path> {
        self.path.parent()
    }
}

// ── Server ────────────────────────────────────────────────────────────

#[derive(Clone)]
struct HyprpilotServer {
    registry: Arc<SkillsRegistry>,
    skills_cache: Arc<RwLock<SkillsCache>>,
    /// `Some` only under `--with-harness`. `None` keeps the historical
    /// skills-only surface — and gates `call_tool` too, not just
    /// `list_tools`, since `call_tool` dispatches on name alone and
    /// would otherwise stay reachable by any client that knows one.
    harness: Option<Arc<crate::mcp::server::harness::Harness>>,
}

impl HyprpilotServer {
    fn new(args: ServeArgs, config: super::ConfigSource) -> anyhow::Result<Self> {
        let harness = args
            .with_harness
            .then(|| Arc::new(crate::mcp::server::harness::Harness::new(config)));
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
            harness,
        })
    }

    /// Dispatch one harness tool. Recoverable failures come back as
    /// `Ok(tool_error(..))` so the agent can read and act on them;
    /// `Err` stays reserved for protocol faults (bad params).
    async fn call_harness_tool(
        &self,
        harness: &crate::mcp::server::harness::Harness,
        name: &str,
        args: serde_json::Map<String, serde_json::Value>,
        context: &RequestContext<RoleServer>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        match name {
            "list_profiles" => match harness.list_profiles() {
                Ok((summary, payload)) => Ok(structured_with_text(summary, payload)),
                Err(msg) => Ok(tool_error(msg)),
            },
            "spawn" => {
                let profile = require_string(&args, "profile")?.to_string();
                let launch = match decode_launch_args(&args, profile, context) {
                    Ok(launch) => launch,
                    Err(msg) => return Ok(tool_error(msg)),
                };
                match harness.spawn(launch).await {
                    Ok(payload) => Ok(structured_with_text(launch_summary(&payload), payload)),
                    Err(msg) => Ok(tool_error(msg)),
                }
            }
            "resume" => {
                let session = require_string(&args, "session")?.to_string();
                // The profile is inherited from the original spawn; the
                // placeholder is replaced inside `resume`.
                let launch = match decode_launch_args(&args, String::new(), context) {
                    Ok(launch) => launch,
                    Err(msg) => return Ok(tool_error(msg)),
                };
                match harness.resume(&session, launch).await {
                    Ok(payload) => Ok(structured_with_text(launch_summary(&payload), payload)),
                    Err(msg) => Ok(tool_error(msg)),
                }
            }
            "session_list" => {
                let (summary, payload) = harness.session_list();
                Ok(structured_with_text(summary, payload))
            }
            "session_read" => {
                let session = require_string(&args, "session")?;
                let tail = optional_usize(&args, "tail")?.unwrap_or(200);
                let offset = optional_u64(&args, "offset")?;
                // `watch` opts into following; `watch_seconds` is an
                // optional self-imposed cap on top of it. Cancelling the
                // request also ends a follow, which is how an agent that
                // has seen enough stops without waiting out a timer.
                let watch = optional_bool(&args, "watch")?.unwrap_or(false);
                let watch_seconds = optional_u64(&args, "watch_seconds")?;
                let watch = (watch || watch_seconds.is_some()).then(|| {
                    crate::mcp::server::harness::WatchOptions {
                        seconds: watch_seconds,
                        // Only stream when the caller actually asked for
                        // progress — an unsolicited notification stream
                        // is noise to a client that cannot render it.
                        sink: context.meta.get_progress_token().map(|token| {
                            crate::mcp::server::harness::ProgressSink {
                                peer: context.peer.clone(),
                                token,
                            }
                        }),
                        cancel: context.ct.clone(),
                    }
                });
                match harness.session_read(session, tail, offset, watch).await {
                    Ok(payload) => {
                        let summary = payload
                            .get("lines")
                            .and_then(serde_json::Value::as_str)
                            .filter(|lines| !lines.is_empty())
                            .map_or_else(
                                || format!("Session {session} has produced no output yet."),
                                str::to_string,
                            );
                        Ok(structured_with_text(summary, payload))
                    }
                    Err(msg) => Ok(tool_error(msg)),
                }
            }
            "session_kill" => {
                let session = require_string(&args, "session")?;
                match harness.session_kill(session).await {
                    Ok(payload) => {
                        let was_running = payload
                            .get("wasRunning")
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(false);
                        let summary = if was_running {
                            format!("Terminated session {session}.")
                        } else {
                            format!("Session {session} had already finished; nothing to terminate.")
                        };
                        Ok(structured_with_text(summary, payload))
                    }
                    Err(msg) => Ok(tool_error(msg)),
                }
            }
            other => Err(rmcp::ErrorData::new(
                ErrorCode::METHOD_NOT_FOUND,
                format!("unknown tool: {other}"),
                None,
            )),
        }
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
                "metadata": s.meta_block,
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
    // Two parallel top-level forms — the slug is a single trailing
    // segment in both, so the references scheme no longer nests a
    // `/references` suffix under the slug (that nesting broke client
    // URI autocomplete).
    if let Some(slug) = rest.strip_prefix("skills/") {
        Some(ParsedUri::Skill(slug))
    } else {
        rest.strip_prefix("references/").map(ParsedUri::SkillReferences)
    }
}

/// Compact builder emitting the same shape the hand-rolled schemas
/// above produce — `type` / `properties` / `required` (omitted when
/// empty, matching `empty_object_schema`) / `additionalProperties:
/// false`. Worth the helper once a tool has more than a field or two.
fn object_schema(props: serde_json::Value, required: &[&str]) -> Arc<serde_json::Map<String, serde_json::Value>> {
    let mut map = serde_json::Map::new();
    map.insert("type".into(), serde_json::json!("object"));
    map.insert("properties".into(), props);
    if !required.is_empty() {
        map.insert("required".into(), serde_json::json!(required));
    }
    map.insert("additionalProperties".into(), serde_json::Value::Bool(false));
    Arc::new(map)
}

/// Shared `spawn` / `resume` parameters. Every one mirrors a CLI flag
/// so the two surfaces cannot drift, and every one carries its unit and
/// default — the calling agent has no other documentation.
fn launch_props(extra: &[(&str, serde_json::Value)]) -> serde_json::Value {
    let mut props = serde_json::json!({
        "prompt": {
            "type": "string",
            "description": "The instruction to send. Mutually exclusive with `file`.",
        },
        "file": {
            "type": "string",
            "description": "Path to a file whose contents become the prompt (`~` and `$VAR` expanded). Mutually exclusive with `prompt`.",
        },
        "cwd": {
            "type": "string",
            "description": "Working directory for the agent. Defaults to the profile's own cwd.",
        },
        "mode": {
            "type": "string",
            "description": "Vendor mode override (e.g. claude's `plan`). Overrides the profile.",
        },
        "with_config": {
            "type": "array",
            "items": { "type": "object" },
            "description": "Ad-hoc profile overlays, same strategic-merge semantics as the CLI's `--with-config`. Use for a one-off model or setting swap.",
        },
        "args": {
            "type": "array",
            "items": { "type": "string" },
            "description": "Extra arguments forwarded verbatim to the vendor CLI — the equivalent of the CLI's trailing `-- <args>`.",
        },
        "wait": {
            "type": "boolean",
            "description": "Block until the turn finishes (default true). When false, returns immediately with the handle; poll `session_read`.",
        },
        "timeout_seconds": {
            "type": "integer",
            "description": "Seconds to wait when `wait` is true (default 300). On timeout the agent KEEPS RUNNING and the result reports status `running` — poll `session_read`, do not spawn again.",
        },
    });
    let map = props.as_object_mut().expect("literal is an object");
    for (key, value) in extra {
        map.insert((*key).to_string(), value.clone());
    }

    props
}

/// Names owned by the harness. Kept next to [`harness_tools`] so the
/// gate and the listing cannot fall out of step.
fn is_harness_tool(name: &str) -> bool {
    matches!(
        name,
        "list_profiles" | "spawn" | "resume" | "session_list" | "session_read" | "session_kill"
    )
}

/// The harness tool set. Every description states how the tool
/// COMPOSES with its siblings, not just what it does — these strings
/// are the only documentation the calling agent ever sees.
fn harness_tools() -> Vec<Tool> {
    vec![
        Tool::new_with_raw(
            "list_profiles",
            Some(
                "START HERE. List the agent profiles you can launch, with the vendor, model, effort, mode, \
                 cwd, and how many MCP servers and skills each one resolves to. Pass a profile's `id` as \
                 `spawn`'s `profile`. A profile already carries its agent/model/effort/mode/MCP/skills, so every \
                 other `spawn` argument is an override, not a requirement. Rows marked `!` failed to resolve — \
                 do not launch those."
                    .into(),
            ),
            empty_object_schema(),
        ),
        Tool::new_with_raw(
            "spawn",
            Some(
                "Start a NEW agent session from a profile and send it a prompt. Returns a `session` handle. \
                 With `wait` true (the default) it blocks and returns the transcript; if the turn outlives \
                 `timeout_seconds` the result comes back with status `running` and the agent KEEPS WORKING — \
                 poll `session_read` with the handle, do NOT call `spawn` again. Use `resume` (not `spawn`) for \
                 every follow-up turn on the same conversation. Sessions live only as long as this MCP server: \
                 if it restarts, running agents are killed and transcripts are lost."
                    .into(),
            ),
            object_schema(
                launch_props(&[(
                    "profile",
                    serde_json::json!({
                        "type": "string",
                        "description": "Profile id from `list_profiles`.",
                    }),
                )]),
                &["profile"],
            ),
        ),
        Tool::new_with_raw(
            "resume",
            Some(
                "Send another turn to an EXISTING session, continuing the same conversation via the vendor's \
                 own session store. Takes the `session` handle returned by `spawn`. The session must have \
                 finished its previous turn — resuming a `running` session is refused, because no vendor \
                 supports two concurrent turns on one conversation. The profile is inherited from the \
                 original `spawn`; you cannot switch profiles mid-conversation."
                    .into(),
            ),
            object_schema(
                launch_props(&[(
                    "session",
                    serde_json::json!({
                        "type": "string",
                        "description": "Session handle from `spawn` or `session_list`.",
                    }),
                )]),
                &["session"],
            ),
        ),
        Tool::new_with_raw(
            "session_list",
            Some(
                "List this server's agent sessions — handle, profile, vendor, status (`running` / `exited`), \
                 exit code, and timestamps. Use it to recover a handle you lost, or to find what is still \
                 running before spawning more. Only sessions started by THIS server appear; it holds no state \
                 across restarts."
                    .into(),
            ),
            empty_object_schema(),
        ),
        Tool::new_with_raw(
            "session_read",
            Some(
                "Read a session's transcript — the vendor's structured JSON event stream, whole lines only. \
                 Works while the agent is still running (poll this after a `spawn` that returned status \
                 `running`) and afterwards, for as long as this server lives. Pass `offset` from a previous \
                 result's `nextOffset` to page forward without re-reading; omit it to get the tail."
                    .into(),
            ),
            object_schema(
                serde_json::json!({
                    "session": {
                        "type": "string",
                        "description": "Session handle from `spawn` or `session_list`.",
                    },
                    "tail": {
                        "type": "integer",
                        "description": "Number of trailing lines to return when `offset` is omitted (default 200).",
                    },
                    "offset": {
                        "type": "integer",
                        "description": "Byte offset to read forward from — pass the `nextOffset` of a previous call to stream new output only.",
                    },
                    "watch": {
                        "type": "boolean",
                        "description": "Follow the session live from `offset` instead of returning immediately. Streams each new chunk as a progress notification (when you pass a progressToken) and returns everything it saw. Ends when the agent finishes, when you cancel the request, or after `watch_seconds`.",
                    },
                    "watch_seconds": {
                        "type": "integer",
                        "description": "Optional cap on a `watch` follow, in seconds. Omit to follow until the agent finishes or you cancel — there is no server-side limit.",
                    },
                }),
                &["session"],
            ),
        ),
        Tool::new_with_raw(
            "session_kill",
            Some(
                "Terminate a running session and everything it started (SIGTERM, then SIGKILL after a grace \
                 period). Use it to stop a runaway agent or to free a slot when `spawn` reports the \
                 concurrency limit. Killing an already-finished session is harmless — it reports \
                 `wasRunning: false`. The transcript stays readable via `session_read` afterwards."
                    .into(),
            ),
            object_schema(
                serde_json::json!({
                    "session": {
                        "type": "string",
                        "description": "Session handle from `spawn` or `session_list`.",
                    },
                }),
                &["session"],
            ),
        ),
    ]
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
    CallToolResult::error(vec![ContentBlock::text(msg)])
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
    result.content = vec![ContentBlock::text(summary)];
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

/// Optional-argument siblings of [`require_string`]. A present-but-wrong
/// type is a protocol fault (`invalid_params`), not a recoverable tool
/// error — the caller sent something the schema forbids.
fn optional_string(
    args: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Option<String>, rmcp::ErrorData> {
    match args.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(rmcp::ErrorData::invalid_params(
            format!("argument `{key}` must be a string"),
            None,
        )),
    }
}

fn optional_bool(
    args: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Option<bool>, rmcp::ErrorData> {
    match args.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(rmcp::ErrorData::invalid_params(
            format!("argument `{key}` must be a boolean"),
            None,
        )),
    }
}

fn optional_u64(args: &serde_json::Map<String, serde_json::Value>, key: &str) -> Result<Option<u64>, rmcp::ErrorData> {
    match args.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => value.as_u64().map(Some).ok_or_else(|| {
            rmcp::ErrorData::invalid_params(format!("argument `{key}` must be a non-negative integer"), None)
        }),
    }
}

fn optional_usize(
    args: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Option<usize>, rmcp::ErrorData> {
    Ok(optional_u64(args, key)?.map(|n| n as usize))
}

fn optional_string_array(
    args: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Vec<String>, rmcp::ErrorData> {
    match args.get(key) {
        None | Some(serde_json::Value::Null) => Ok(Vec::new()),
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str().map(str::to_string).ok_or_else(|| {
                    rmcp::ErrorData::invalid_params(format!("every entry in `{key}` must be a string"), None)
                })
            })
            .collect(),
        Some(_) => Err(rmcp::ErrorData::invalid_params(
            format!("argument `{key}` must be an array of strings"),
            None,
        )),
    }
}

/// Decode the shared `spawn` / `resume` argument set.
///
/// Returns `Err(String)` for things the caller can fix and retry (an
/// unreadable prompt file, `prompt` and `file` together) — those come
/// back as tool errors, not protocol faults.
fn decode_launch_args(
    args: &serde_json::Map<String, serde_json::Value>,
    profile: String,
    context: &RequestContext<RoleServer>,
) -> Result<crate::mcp::server::harness::LaunchToolArgs, String> {
    let inline = optional_string(args, "prompt").map_err(|err| err.to_string())?;
    let file = optional_string(args, "file").map_err(|err| err.to_string())?;

    // Mirrors clap's `conflicts_with` on the CLI's `-p` / `-f`.
    let prompt = match (inline, file) {
        (Some(_), Some(_)) => {
            return Err("`prompt` and `file` are mutually exclusive — pass exactly one.".into());
        }
        (Some(inline), None) => inline,
        (None, Some(file)) => {
            let path = crate::paths::resolve_user(&file);
            std::fs::read_to_string(&path).map_err(|err| format!("could not read `file` {}: {err}", path.display()))?
        }
        (None, None) => {
            return Err("a prompt is required: pass `prompt` or `file`.".into());
        }
    };
    if prompt.trim().is_empty() {
        return Err("the prompt is empty.".into());
    }

    let with_config = match args.get("with_config") {
        None | Some(serde_json::Value::Null) => Vec::new(),
        Some(serde_json::Value::Array(items)) => items.clone(),
        Some(_) => return Err("`with_config` must be an array of overlay objects.".into()),
    };

    Ok(crate::mcp::server::harness::LaunchToolArgs {
        profile,
        prompt,
        cwd: optional_string(args, "cwd")
            .map_err(|err| err.to_string())?
            .map(|cwd| crate::paths::resolve_user(&cwd)),
        mode: optional_string(args, "mode").map_err(|err| err.to_string())?,
        with_config,
        args: optional_string_array(args, "args").map_err(|err| err.to_string())?,
        wait: optional_bool(args, "wait")
            .map_err(|err| err.to_string())?
            .unwrap_or(true),
        timeout_seconds: optional_u64(args, "timeout_seconds")
            .map_err(|err| err.to_string())?
            .unwrap_or_else(crate::mcp::server::harness::LaunchToolArgs::default_timeout),
        // A waiting spawn streams the transcript when the caller asked
        // for progress, so a long turn is visible as it happens rather
        // than arriving all at once at the end.
        sink: context
            .meta
            .get_progress_token()
            .map(|token| crate::mcp::server::harness::ProgressSink {
                peer: context.peer.clone(),
                token,
            }),
        cancel: Some(context.ct.clone()),
    })
}

/// Human-readable summary for a `spawn` / `resume` result.
///
/// The agent's own output comes FIRST and the session's terminal state
/// LAST, so a reader (human or model) hits the answer immediately and
/// finds the exit status where a terminal would put it. A `running`
/// result says plainly to poll rather than re-spawn — the single most
/// expensive mistake a calling agent can make here.
fn launch_summary(payload: &serde_json::Value) -> String {
    let handle = payload
        .get("session")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("?");
    let body = payload
        .get("text")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .trim_end();
    let timed_out = payload
        .get("timedOut")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);

    let mut out = String::new();
    if !body.is_empty() {
        out.push_str(body);
        out.push_str("\n\n");
    }
    if timed_out {
        out.push_str(&format!(
            "── session {handle} — STILL RUNNING (turn outlived its timeout). \
             Poll `session_read` with this handle; do NOT spawn again."
        ));

        return out;
    }
    match payload.get("exitCode").and_then(serde_json::Value::as_i64) {
        Some(0) => out.push_str(&format!("── session {handle} — exited (exit 0)")),
        Some(code) => out.push_str(&format!("── session {handle} — exited (exit {code}) — check `stderr`")),
        None => out.push_str(&format!(
            "── session {handle} — {} (no exit code yet)",
            payload.get("status").and_then(serde_json::Value::as_str).unwrap_or("?")
        )),
    }

    out
}

// ── MCP protocol impl ─────────────────────────────────────────────────

impl ServerHandler for HyprpilotServer {
    fn get_info(&self) -> ServerInfo {
        let mut caps = ServerCapabilities::default();
        // The tool set is static (`list_tools` always returns the same
        // five) — do NOT advertise tool-list-changed. Skills back the
        // resource list, which `reload` can change, so resources DO
        // advertise list-changed (and `reload` fires it).
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
                 resource and tool result carries the skill's frontmatter \
                 verbatim in ONE block (as `metadata` in tool output, and as the \
                 `io.hyprpilot/skill` key in resource `_meta`) — minus `title` / \
                 `description` (already in the spec Resource fields) and plus the \
                 runtime `path` + `bundleDir`.",
            )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, rmcp::ErrorData> {
        let mut tools = vec![
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
                     `hyprpilot://references/<slug>`."
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
        if self.harness.is_some() {
            tools.extend(harness_tools());
        }
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
                        "metadata": skill.meta_block,
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
            "open" => {
                let path = require_string(&args, "path")?;
                match open::that_detached(path) {
                    Ok(()) => Ok(structured_with_text(
                        format!("Opened {path}"),
                        serde_json::json!({ "opened": path }),
                    )),
                    Err(err) => Ok(tool_error(format!("open failed: {err}"))),
                }
            }
            // The harness arms are reached ONLY through this guard.
            // Gating `list_tools` alone would be cosmetic: `call_tool`
            // dispatches on the name, so an unlisted tool would still be
            // callable by any client that knows it exists.
            name if is_harness_tool(name) => match self.harness.as_ref() {
                Some(harness) => self.call_harness_tool(harness, name, args, &context).await,
                None => Err(rmcp::ErrorData::new(
                    ErrorCode::METHOD_NOT_FOUND,
                    format!("unknown tool: {name}"),
                    None,
                )),
            },
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
                    "Bundle of every reference declared in the skill's frontmatter, \
                     concatenated with `--- <basename> ---` delimiters.",
                )
                .with_mime_type("text/markdown"),
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
                    meta: Some(skill_meta(&skill.meta_block)),
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
                    meta: Some(skill_meta(&skill.meta_block)),
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

    /// `build_cache` builds the single merged block once: verbatim
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
                    "metadata": {
                        "name": "plan-hard",
                        "argument-hint": "[goal]",
                        "references": ["../references/plan-mode.md"],
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

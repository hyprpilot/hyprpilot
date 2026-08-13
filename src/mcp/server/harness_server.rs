//! `hyprpilot mcp harness` — the agent-harness MCP server.
//!
//! A server in its own right, not a flag on the skills one. That is
//! what makes the surfaces independent: its own process (so a panic
//! under the release profile's `panic = "abort"` takes down only the
//! harness), its own catalog entry, and therefore its own tool-approval
//! policy — auto-accepting a skill read and auto-accepting `spawn` are
//! very different decisions.
//!
//! It also makes the old gate structural. There is no `is_harness_tool`
//! check and no conditional `call_tool` arm to keep in sync across two
//! call sites: a skills server cannot serve `spawn` because it does not
//! implement it.

use std::sync::Arc;

use clap::Args;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, ErrorCode, Implementation, ListToolsResult, PaginatedRequestParams,
    ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::ServerHandler;
use rmcp::ServiceExt;

use super::harness::{DelegatePolicy, Harness};
use super::rpc::{
    empty_object_schema, object_schema, optional_bool, optional_string, optional_string_array, optional_u64,
    optional_usize, require_string, structured_with_text, tool_error, wait_for_shutdown, RESULT_CACHE_SCOPE,
    RESULT_TTL_MS,
};
use crate::config::mcp::DEFAULT_HARNESS_SERVER_NAME;

#[derive(Clone)]
pub struct HarnessServer {
    harness: Arc<Harness>,
}

impl HarnessServer {
    fn new(
        config: super::ConfigSource,
        max_sessions: usize,
        max_depth: usize,
        delegates: DelegatePolicy,
        delegate_mcp: Option<crate::config::McpConfig>,
    ) -> Self {
        Self {
            harness: Arc::new(Harness::new(config, max_sessions, max_depth, delegates, delegate_mcp)),
        }
    }
}

impl ServerHandler for HarnessServer {
    fn supported_protocol_versions(&self) -> std::borrow::Cow<'static, [rmcp::model::ProtocolVersion]> {
        super::rpc::supported_protocol_versions()
    }

    fn get_info(&self) -> ServerInfo {
        let mut caps = ServerCapabilities::default();
        // Fixed for the life of the process.
        let mut tools = rmcp::model::ToolsCapability::default();
        tools.list_changed = Some(false);
        caps.tools = Some(tools);
        // Claude Code registers a channel listener only when it sees
        // this key. Declaring it costs nothing elsewhere: the spec says
        // unknown capabilities are ignored, and a client that never
        // registers simply drops whatever we push.
        //
        // `ServerCapabilities` is `#[non_exhaustive]`, so this is field
        // assignment on the owned `default()`, not a struct literal.
        caps.experimental = Some(
            [("claude/channel".to_string(), serde_json::Map::new())]
                .into_iter()
                .collect(),
        );
        // SEP-2663. Advertising is REQUIRED for `tasks/*` to route at
        // all: `validate_tasks_capability` answers `method_not_found`
        // unless the server declares it. Field assignment rather than
        // `ServerCapabilities::builder().enable_tasks()` — the builder
        // cannot express `tools.list_changed = Some(false)` above
        // (`enable_tool_list_changed` only sets `Some(true)`), so
        // migrating would quietly change an advertised capability.
        //
        // This is the one part of the feature NOT gated on the client:
        // the key appears for every peer. Harmless by construction —
        // unknown capabilities are ignored per spec — and verified
        // against all three vendor CLIs.
        caps.extensions = Some(
            [(rmcp::model::TASKS_EXTENSION_ID.to_string(), serde_json::Map::new())]
                .into_iter()
                .collect(),
        );

        ServerInfo::new(caps)
            .with_server_info(Implementation::new(
                DEFAULT_HARNESS_SERVER_NAME.to_string(),
                env!("CARGO_PKG_VERSION").to_string(),
            ))
            .with_instructions(instructions())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, rmcp::ErrorData> {
        Ok(ListToolsResult::with_all_items(harness_tools())
            .with_ttl_ms(RESULT_TTL_MS)
            .with_cache_scope(RESULT_CACHE_SCOPE))
    }

    /// SEP-2663 poll. The task id names one TURN of one session, so a
    /// finished turn keeps reporting its terminal state even after the
    /// conversation moves on — which is what the spec requires and what
    /// `session_status` (deliberately) does not do, since it answers
    /// about *now*.
    async fn get_task(
        &self,
        request: rmcp::model::GetTaskParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::GetTaskResult, rmcp::ErrorData> {
        let (handle, _) = crate::mcp::server::harness::parse_task_id(&request.task_id)
            .ok_or_else(|| rmcp::ErrorData::invalid_params(format!("`{}` is not a task id.", request.task_id), None))?;
        let task = self
            .harness
            .task_view(&request.task_id)
            .map_err(|msg| rmcp::ErrorData::invalid_params(msg, None))?;
        let mut result = rmcp::model::GetTaskResult::new(task);
        // Also on the poll result, not just the mint: a client that
        // persisted only the task id can recover the session handle
        // without taking the id apart.
        let mut meta = serde_json::Map::new();
        meta.insert("io.hyprpilot/session".into(), serde_json::json!(handle));
        result.meta = Some(rmcp::model::MetaObject(meta));
        Ok(result)
    }

    /// Cancels the addressed TURN, not the session.
    ///
    /// Terminal tasks are immutable, so cancelling one is a no-op rather
    /// than an error — and crucially it must not reach the session, whose
    /// next turn may be running. An unknown handle is `-32602`, matching
    /// `tasks/get` and SEP-2663's "SHOULD" for cancel.
    async fn cancel_task(
        &self,
        request: rmcp::model::CancelTaskParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<(), rmcp::ErrorData> {
        let (handle, turn) = crate::mcp::server::harness::parse_task_id(&request.task_id)
            .ok_or_else(|| rmcp::ErrorData::invalid_params(format!("`{}` is not a task id.", request.task_id), None))?;
        self.harness
            .cancel_turn(handle, turn)
            .await
            .map_err(|msg| rmcp::ErrorData::invalid_params(msg, None))
    }

    /// Dispatch one harness tool. Recoverable failures come back as
    /// `Ok(tool_error(..))` so the agent can read and act on them;
    /// `Err` stays reserved for protocol faults (bad params).
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, rmcp::ErrorData> {
        let harness = self.harness.as_ref();
        let context = &context;
        let args = request.arguments.unwrap_or_default();
        match request.name.as_ref() {
            "list_profiles" => match harness.list_profiles() {
                Ok((summary, payload)) => Ok(structured_with_text(summary, payload)),
                Err(msg) => Ok(tool_error(msg)),
            },
            "spawn" => {
                let profile = require_string(&args, "profile")?.to_string();
                let launch = match decode_launch_args(&args, profile, context, true) {
                    Ok(launch) => launch,
                    Err(msg) => return Ok(tool_error(msg)),
                };
                match harness.spawn(launch).await {
                    // Only the Ok arm can become a task. A refused launch
                    // must stay a tool error — minting a task id for work
                    // that never started hands the caller a handle that
                    // can never resolve.
                    Ok(payload) => Ok(as_task_or_result(harness, context, payload)),
                    Err(msg) => Ok(tool_error(msg)),
                }
            }
            "session_send" => {
                let session = require_string(&args, "session")?.to_string();
                // The profile is inherited from the original spawn; the
                // placeholder is replaced inside `session_send`.
                let launch = match decode_launch_args(&args, String::new(), context, false) {
                    Ok(launch) => launch,
                    Err(msg) => return Ok(tool_error(msg)),
                };
                match harness.session_send(&session, launch).await {
                    Ok(payload) => Ok(as_task_or_result(harness, context, payload)),
                    Err(msg) => Ok(tool_error(msg)),
                }
            }
            "session_status" => {
                let session = require_string(&args, "session")?;
                match harness.session_status(session) {
                    Ok((summary, payload)) => Ok(structured_with_text(summary, payload)),
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
                let cursor = optional_string(&args, "cursor")?;
                // Deliberately the same two knobs `spawn` takes, meaning
                // the same thing: `wait` follows, `timeout_seconds` caps
                // it. Cancelling the request also ends a follow, which is
                // how an agent that has seen enough stops without waiting
                // out a timer.
                // `wait` alone decides whether this follows. Letting a
                // bare `timeout_seconds` imply it turned a defensive
                // timeout into a surprise blocking call — the schema
                // presents it as a cap ON a follow, not a trigger for one.
                let watch = optional_bool(&args, "wait")?.unwrap_or(false);
                let watch_seconds = optional_u64(&args, "timeout_seconds")?;
                let watch = watch.then(|| {
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
                match harness.session_read(session, tail, cursor, watch).await {
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
                        let summary = match payload.get("action").and_then(serde_json::Value::as_str) {
                            Some("terminated") => format!(
                                "Terminated session {session}. Its transcript is still readable — \
                                 call `session_kill` again to reap it."
                            ),
                            Some("reaped") => {
                                format!("Session {session} had already finished; reaped it and its transcript.")
                            }
                            _ => format!("Session {session} is gone."),
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
}

/// Args for `hyprpilot mcp harness`.
///
/// No `--skill-dir`: the harness tools never read the skill catalog, so
/// the flag would be inert. That asymmetry is the reason these are two
/// subcommands rather than one behind a flag.
#[derive(Debug, Args, Clone)]
pub struct HarnessArgs {
    /// How many agent sessions to retain before evicting the oldest
    /// FINISHED ones (with their transcripts).
    ///
    /// A conversation reuses its session however many turns it runs, so
    /// only distinct `spawn`s grow the table — this bounds a long-lived
    /// server's memory and temp directories. A running session is never
    /// evicted. Raise it on a busy gateway that wants deeper history;
    /// lower it where temp space is tight.
    #[arg(
        long = "max-sessions",
        default_value_t = crate::config::mcp::DEFAULT_MAX_SESSIONS,
        value_name = "N"
    )]
    pub max_sessions: usize,

    /// Suppress the per-turn completion channel.
    ///
    /// A flag rather than a config read: the launcher resolved the
    /// PICKED profile's `[mcp.harness]` block and passes the answer
    /// down, the same way `--max-sessions` arrives. A sidecar cannot
    /// work out which profile spawned it.
    #[arg(long = "no-notify-on-complete")]
    pub no_notify_on_complete: bool,

    /// Glob over profile ids this session may delegate to. Repeatable;
    /// omitted entirely means no allow-filter.
    ///
    /// The launcher resolves the PICKED profile's `[mcp.harness]
    /// includeProfiles` and emits one occurrence per pattern, the same
    /// way `--skill-dir` carries the skill roots.
    #[arg(long = "include-profile", value_name = "GLOB", conflicts_with = "no_delegates")]
    pub include_profiles: Vec<String>,

    /// Glob over profile ids this session may NOT delegate to. Beats
    /// `--include-profile` on overlap. Repeatable.
    #[arg(long = "exclude-profile", value_name = "GLOB")]
    pub exclude_profiles: Vec<String>,

    /// Delegate to nothing — `includeProfiles = []`.
    ///
    /// Its own flag because zero `--include-profile` occurrences is
    /// exactly what "unset" looks like on the wire, and unset means
    /// unrestricted. An empty list must not decay into its opposite.
    #[arg(long = "no-delegates")]
    pub no_delegates: bool,

    /// How many levels of delegation to allow — `[mcp.harness] maxDepth`
    /// resolved by the launcher.
    ///
    /// The same number gated whether this sidecar was injected at all,
    /// so the two answers cannot disagree. A hand-started sidecar falls
    /// back to the seeded default.
    #[arg(
        long = "max-depth",
        default_value_t = crate::config::mcp::DEFAULT_MAX_SPAWN_DEPTH,
        value_name = "N"
    )]
    pub max_depth: usize,

    /// `[mcp.harness.mcp]` as JSON — the `[mcp]` overlay every delegate
    /// receives on top of its own resolved block.
    ///
    /// Rides argv for the reason the delegate globs do: the block is
    /// per-profile and a sidecar cannot work out which profile spawned
    /// it. Omitted entirely when the captain declared none, which leaves
    /// every delegate on its own configuration.
    #[arg(long = "delegate-mcp", value_name = "JSON")]
    pub delegate_mcp: Option<String>,
}

/// Shared `spawn` / `session_send` parameters. Every one mirrors a CLI flag
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
            "description": "Block until the turn finishes (default FALSE). Left off, this returns as soon as the agent starts — poll `session_status`, then `session_read` once it reports `exited`. Turn it on only to hold the call open for a turn you expect to be short.",
        },
        "timeout_seconds": {
            "type": "integer",
            "description": "Seconds to wait when `wait` is true (default 300). Ignored otherwise. On timeout the agent KEEPS RUNNING and the result reports status `running` — poll `session_status`, do not spawn again.",
        },
    });
    let map = props.as_object_mut().expect("literal is an object");
    for (key, value) in extra {
        map.insert((*key).to_string(), value.clone());
    }

    props
}

/// The `session_send` parameter set: only what can meaningfully differ
/// between turns of ONE conversation.
///
/// `cwd` / `args` / `with_config` are deliberately absent. They are the
/// session's launch shape, replayed from the `spawn`, and offering them
/// per turn is how the conversation gets corrupted: claude keys its
/// conversation store by project directory, so a different `cwd` on a
/// follow-up made `--resume` fail with a bare "No conversation found"
/// for a healthy session. `args` / `with_config` are quieter — they
/// change the agent's model or flags mid-conversation while the result
/// keeps reporting the profile, so nobody reading back can tell.
///
/// `mode` stays because a per-turn permission change is a real workflow
/// ("now go read-only") and it does not touch session lookup. `wait` /
/// `timeout_seconds` are about how the CALLER waits, not about the
/// session at all.
fn session_send_props() -> serde_json::Value {
    serde_json::json!({
        "session": {
            "type": "string",
            "description": "Session handle from `spawn` or `session_list`.",
        },
        "prompt": {
            "type": "string",
            "description": "The instruction to send. Mutually exclusive with `file`.",
        },
        "file": {
            "type": "string",
            "description": "Path to a file whose contents become the prompt (`~` and `$VAR` expanded). Mutually exclusive with `prompt`.",
        },
        "mode": {
            "type": "string",
            "description": "Vendor mode override for THIS turn (e.g. claude's `plan` for a read-only follow-up). Otherwise the session's own mode carries over.",
        },
        "wait": {
            "type": "boolean",
            "description": "Block until the turn finishes (default FALSE). Left off, this returns as soon as the turn starts — poll `session_status`, then `session_read` once it reports `exited`. Turn it on only to hold the call open for a turn you expect to be short.",
        },
        "timeout_seconds": {
            "type": "integer",
            "description": "Seconds to wait when `wait` is true (default 300). Ignored otherwise. On timeout the agent KEEPS RUNNING and the result reports status `running` — poll `session_status`, do not send again.",
        },
    })
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
                 and cwd. Pass a profile's `id` as `spawn`'s `profile`. A profile already carries its agent/model/effort/mode/MCP/skills, so every \
                 other `spawn` argument is an override, not a requirement. Rows marked `!` failed to resolve — \
                 do not launch those."
                    .into(),
            ),
            empty_object_schema(),
        ),
        Tool::new_with_raw(
            "spawn",
            Some(
                "Start a NEW agent session from a profile and send it a prompt. Returns a `session` handle \
                 immediately, while the agent works — that handle is how you address it from here on, and it \
                 stays the same for the life of the conversation. Poll `session_status { session }` until it \
                 reports `exited`, then `session_read` for the transcript. Pass `wait: true` to block instead, \
                 for a turn you expect to be short. Use `session_send` (not `spawn`) for every follow-up turn on \
                 the same conversation. Sessions live only as long as this MCP server: if it restarts, running \
                 agents are killed and transcripts are lost."
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
            "session_send",
            Some(
                "Send another message to an existing session, continuing the same conversation via the vendor's \
                 own session store. Takes the `session` handle from `spawn` or `session_list`. The result's \
                 `delivery` field says what happened. The handle does NOT change — it stays valid for the whole \
                 conversation, however many turns you send, and the transcript keeps accumulating under it. \
                 Like `spawn`, it returns as soon as the turn starts unless you pass `wait: true`. \
                 The session must have finished its previous turn: sending to a `running` session is refused, \
                 because no vendor supports two concurrent turns on one conversation — poll `session_status` or \
                 `session_kill` it first. The whole launch — profile, working directory, arguments, config \
                 overlays — is inherited from the `spawn` and cannot be changed here; only the prompt, the \
                 `mode`, and how long you wait are per-turn."
                    .into(),
            ),
            object_schema(session_send_props(), &["session"]),
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
                 result's `nextOffset` to page forward without re-reading; omit it to get the tail. \
                 Pass `wait: true` to follow the session live instead of returning immediately — the same \
                 knob, with the same meaning, as on `spawn`."
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
                        "description": "Number of trailing lines to return when `cursor` is omitted (default 200).",
                    },
                    "cursor": {
                        "type": "string",
                        "description": "Pagination cursor. Pass the `nextCursor` from a previous result VERBATIM to continue exactly where that read stopped; omit it to read the tail. Opaque — do not parse or construct one. A result with no `nextCursor` means the session is finished and you have all of it.",
                    },
                    "wait": {
                        "type": "boolean",
                        "description": "Follow the session live from `cursor` instead of returning immediately (default false). Same semantics as `spawn`'s `wait`: it streams each new chunk as a progress notification when you pass a progressToken, and returns everything it saw. Ends when the agent finishes, when you cancel the request, or after `timeout_seconds`.",
                    },
                    "timeout_seconds": {
                        "type": "integer",
                        "description": "Optional cap on a `wait` follow, in seconds. Omit to follow until the agent finishes or you cancel — there is no server-side limit.",
                    },
                }),
                &["session"],
            ),
        ),
        Tool::new_with_raw(
            "session_status",
            Some(
                "Check ONE session's state without reading its transcript — status (`running` / `exited`), \
                 exit code, how many bytes it has written, and whether the agent's final answer has landed \
                 (`hasResult`). This is the cheap poll: `session_list` returns every session and \
                 `session_read` returns the transcript itself, which runs to tens of kilobytes. Use it after \
                 a `spawn` or `session_send` that came back `running`, then call `session_read` once it \
                 reports `exited`. Note a session is `exited` after every TURN, not only when the \
                 conversation is over."
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
        Tool::new_with_raw(
            "session_kill",
            Some(
                "Stop a session, or forget one that has already stopped. On a RUNNING session it terminates \
                 the agent and everything it started (SIGTERM, then SIGKILL after a grace period) and KEEPS \
                 the transcript, so you can still `session_read` why it was killed. On an already-FINISHED \
                 session it reaps it — the transcript and the handle both go. So calling it twice is the \
                 natural stop-then-clean-up, and calling it on a finished session is how you free memory \
                 early instead of waiting for the retention limit. The result's `action` says which happened."
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

/// Whether the caller wants the tool call held open until the turn ends.
///
/// **Detached by default.** A turn runs for minutes, and a blocking call
/// that outlives `timeout_seconds` comes back `running` regardless — so
/// waiting never guaranteed a finished answer, it only cost the caller
/// the ability to do anything else meanwhile. `session_status` reports
/// the same thing for the price of one `stat`.
fn wait_flag(args: &serde_json::Map<String, serde_json::Value>) -> Result<bool, String> {
    Ok(optional_bool(args, "wait")
        .map_err(|err| err.to_string())?
        .unwrap_or(false))
}

/// Decode the shared `spawn` / `session_send` argument set.
///
/// Returns `Err(String)` for things the caller can fix and retry (an
/// unreadable prompt file, `prompt` and `file` together) — those come
/// back as tool errors, not protocol faults.
/// Decode a launch. `accept_launch_shape` is false for `session_send`,
/// which does not advertise `cwd` / `args` / `with_config` — those are
/// replayed from the session, and reading them here would let a stale
/// or hand-written caller corrupt a conversation the schema already
/// refuses to let it touch.
fn decode_launch_args(
    args: &serde_json::Map<String, serde_json::Value>,
    profile: String,
    context: &RequestContext<RoleServer>,
    accept_launch_shape: bool,
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

    // Reject rather than ignore. A caller that passes `cwd` believes it
    // took effect; silently dropping it is the same silent-wrong-result
    // class as the bug that made these inherit in the first place.
    if !accept_launch_shape {
        for key in ["cwd", "args", "with_config"] {
            if args.get(key).is_some_and(|value| !value.is_null()) {
                return Err(format!(
                    "`{key}` is not accepted on `session_send` — it is inherited from the `spawn` that started \
                     this conversation and cannot change mid-stream. Start a new session to launch differently."
                ));
            }
        }
    }

    let with_config = match args.get("with_config").filter(|_| accept_launch_shape) {
        None | Some(serde_json::Value::Null) => Vec::new(),
        Some(serde_json::Value::Array(items)) => items.clone(),
        Some(_) => return Err("`with_config` must be an array of overlay objects.".into()),
    };

    Ok(crate::mcp::server::harness::LaunchToolArgs {
        profile,
        prompt,
        cwd: if accept_launch_shape {
            optional_string(args, "cwd")
                .map_err(|err| err.to_string())?
                .map(|cwd| crate::paths::resolve_user(&cwd))
        } else {
            None
        },
        mode: optional_string(args, "mode").map_err(|err| err.to_string())?,
        with_config,
        args: if accept_launch_shape {
            optional_string_array(args, "args").map_err(|err| err.to_string())?
        } else {
            Vec::new()
        },
        wait: wait_flag(args)?,
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

/// Human-readable summary for a `spawn` / `session_send` result.
///
/// The agent's own output comes FIRST and the session's terminal state
/// LAST, so a reader (human or model) hits the answer immediately and
/// finds the exit status where a terminal would put it. A `running`
/// result says plainly to poll rather than re-spawn — the single most
/// expensive mistake a calling agent can make here.
/// The fork between the two launch paths.
///
/// A client that declared SEP-2663 gets a task handle; everyone else
/// gets exactly what they got before this existed. There is no third
/// behaviour and no config switch — the client's own declaration is the
/// whole gate, which is why an ordinary `spawn` cannot change shape
/// underneath a caller that never asked.
///
/// `client_capabilities()` is `None` for a peer that has not finished
/// `initialize` (`service.rs:1242-1249`); `None` means *not declared*,
/// never "probably supported".
fn as_task_or_result(
    harness: &Harness,
    context: &RequestContext<RoleServer>,
    payload: serde_json::Value,
) -> CallToolResponse {
    let declared = context.client_capabilities().is_some_and(|caps| caps.supports_tasks());
    if !declared {
        return structured_with_text(launch_summary(&payload), payload);
    }
    let Some((handle, turn)) = payload
        .get("session")
        .and_then(serde_json::Value::as_str)
        .and_then(|handle| harness.current_turn(handle).map(|turn| (handle, turn)))
    else {
        // The launch succeeded but the session is already gone. Fall back
        // rather than mint a task id nothing can resolve.
        return structured_with_text(launch_summary(&payload), payload);
    };
    CallToolResponse::Task(harness.new_task(handle, turn))
}

pub(super) fn launch_summary(payload: &serde_json::Value) -> String {
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
    // Which path was taken comes first: "resumed a finished conversation"
    // and "the agent was already going" are different situations, and a
    // caller deciding what to do next needs to know which it got.
    if payload.get("delivery").and_then(serde_json::Value::as_str) == Some("resumed") {
        out.push_str(&format!("Resumed session {handle} and sent.\n\n"));
    }
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

/// Parse `--delegate-mcp` and run it through garde.
///
/// Both halves fail the sidecar at startup rather than degrading to
/// "no overlay": the overlay is what NARROWS a delegate's reach, so
/// dropping it silently widens what a delegate can do — the same reason
/// a malformed delegate glob is fatal rather than skipped. Validation
/// matters most exactly where a hand-started sidecar bypasses the
/// launcher's own config-load check.
fn parse_delegate_mcp(raw: &str) -> anyhow::Result<crate::config::McpConfig> {
    let overlay: crate::config::McpConfig = serde_json::from_str(raw)
        .map_err(|err| anyhow::anyhow!("mcp harness: `--delegate-mcp` is not a valid [mcp] block: {err}"))?;
    garde::Validate::validate(&overlay)
        .map_err(|err| anyhow::anyhow!("mcp harness: `--delegate-mcp` failed validation: {err}"))?;

    Ok(overlay)
}

/// Run the harness server over stdio.
pub async fn run_harness(args: HarnessArgs, config: super::ConfigSource) -> anyhow::Result<()> {
    // `--no-delegates` is `includeProfiles = []`: an allow-filter that
    // matches nothing, which an empty glob set already is.
    let include = (args.no_delegates || !args.include_profiles.is_empty()).then_some(&args.include_profiles[..]);
    let delegates = DelegatePolicy::new(include, &args.exclude_profiles)
        .map_err(|err| anyhow::anyhow!("mcp harness: delegate scope: {err}"))?;
    let delegate_mcp = args.delegate_mcp.as_deref().map(parse_delegate_mcp).transpose()?;
    tracing::info!(
        max_sessions = args.max_sessions,
        max_depth = args.max_depth,
        include_profiles = ?args.include_profiles,
        exclude_profiles = ?args.exclude_profiles,
        no_delegates = args.no_delegates,
        delegate_mcp = args.delegate_mcp.is_some(),
        "mcp: starting the harness server"
    );
    // Reclaim anything a crashed predecessor left behind before starting
    // our own. A non-empty sweep logs at `warn`.
    super::sessions::sweep_stale_sessions();

    let handler = HarnessServer::new(config, args.max_sessions, args.max_depth, delegates, delegate_mcp);
    // Clone the table BEFORE `serve()` — it consumes the handler, and
    // `waiting()` consumes the `RunningService`, so this is the only
    // chance to keep a handle for the shutdown reap.
    let sessions = Arc::clone(&handler.harness.sessions);
    let harness_for_hook = Arc::clone(&handler.harness);

    let (stdin, stdout) = rmcp::transport::io::stdio();
    let running = handler
        .serve((stdin, stdout))
        .await
        .map_err(|err| anyhow::anyhow!("mcp harness: serve failed at init: {err}"))?;

    // The peer exists only once `serve()` has returned, which is also
    // the earliest a session can exist — so installing the hook here is
    // ordered correctly, not merely convenient.
    if !args.no_notify_on_complete {
        let peer = running.peer().clone();
        let name = DEFAULT_HARNESS_SERVER_NAME.to_string();
        let harness = Arc::clone(&harness_for_hook);
        let table = Arc::clone(&sessions);
        sessions.set_exit_hook(Arc::new(move |handle: String, turn: u32, code: i32| {
            let peer = peer.clone();
            let name = name.clone();
            let harness = Arc::clone(&harness);
            // Seal SYNCHRONOUSLY, before the spawned notifier runs: this
            // is the only moment the real finish time is known, and a
            // `session_send` can start the next turn while the notifier
            // is still queued.
            table.seal_turn(&handle, turn, code);
            tokio::spawn(async move {
                super::harness::notify_session_finished(&peer, &name, &handle, code).await;

                // SEP-2663 status push — GATED, and it has to be. The
                // hook fires for every turn of every session, so an
                // ungated push here would send `notifications/tasks` to a
                // client that never declared the extension, for an
                // ordinary `spawn`. That is the one behaviour change this
                // feature must not make.
                //
                // Gated on ONE recorded fact: did this turn actually hand
                // the caller a task handle. That already implies the
                // caller opted in, and it is the only form available here
                // — a request sees per-request `_meta` capabilities, while
                // this hook has nothing but the peer's `initialize` info.
                // Re-deriving from `peer_info()` would silently skip the
                // push for a client that declared tasks the way the spec
                // documents: per request.
                // The turn that EXITED, not whatever is current now — a
                // `session_send` landing first would otherwise make this
                // announce turn N+1 as `working` in place of turn N's
                // completion.
                if !harness.turn_minted_task(&handle, turn) {
                    return;
                }
                if let Ok(task) = harness.task_view(&super::harness::task_id(&handle, turn)) {
                    super::harness::notify_task_finished(&peer, task).await;
                }
            });
        }));
    }

    wait_for_shutdown(running).await;
    sessions.shutdown().await;

    Ok(())
}

/// The end-to-end workflow, in one place, for a client that reads server
/// instructions before it reads any individual tool schema.
fn instructions() -> String {
    "Hyprpilot agent harness. You can launch and drive hyprpilot agent \
     profiles. Typical flow:\n\
     1. `list_profiles` — see what can be launched and with which model.\n\
     2. `spawn { profile, prompt }` — start an agent. It returns a \
     `session` handle right away and the agent keeps working. That handle \
     is the session's id: use it for every later call, and do NOT call \
     `spawn` again for the same conversation.\n\
     3. `session_status { session }` — the cheap poll: state, and whether \
     the answer has landed, without reading the transcript. Repeat until \
     it reports `exited`.\n\
     4. `session_read { session }` — the transcript, any time, including \
     after the run finished. Pass `wait: true` to follow live; pass a \
     progressToken to receive output as progress notifications as it \
     arrives.\n\
     5. `session_send { session, prompt }` — next turn in the same \
     conversation. The handle does not change.\n\
     6. `session_kill { session }` — stops a running session; on an \
     already-finished one it reaps it.\n\
     `spawn` and `session_send` take `wait: true` to block until the turn \
     ends instead — worth it only for a turn you expect to be short, since \
     a turn that outlives `timeout_seconds` comes back `running` anyway.\n\
     If your client supports channels, a `<channel \
     source=\"hyprpilot_harness\">` block appears in your context when a \
     session finishes — read that session with `session_read`. It fires \
     per TURN, not only when a conversation ends.\n\
     Sessions are children of THIS server and die with it: they do not \
     survive a restart, and their transcripts go too."
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_delegate_overlay_parses_from_the_json_the_launcher_emits() {
        let overlay = parse_delegate_mcp(r#"{"skills":{"enabled":false}}"#).expect("a well-formed block parses");

        assert_eq!(overlay.skills.and_then(|s| s.enabled), Some(false));
    }

    /// Dropping a malformed overlay would WIDEN what delegates reach —
    /// the same reason a malformed delegate glob kills the sidecar
    /// instead of being skipped. Both failure modes are fatal.
    #[test]
    fn a_malformed_delegate_overlay_kills_the_sidecar() {
        let err = parse_delegate_mcp("{not json").expect_err("garbage must not degrade to `no overlay`");
        assert!(err.to_string().contains("not a valid [mcp] block"), "got: {err}");

        let err = parse_delegate_mcp(r#"{"nonsense":true}"#).expect_err("deny_unknown_fields rejects a typo");
        assert!(err.to_string().contains("not a valid [mcp] block"), "got: {err}");
    }

    /// garde runs on the overlay too. A hand-started sidecar never went
    /// through the launcher's config-load check, so this is the only
    /// place a malformed glob inside the overlay can be caught before it
    /// reaches match time — where an uncompilable exclude silently stops
    /// excluding.
    #[test]
    fn a_well_formed_overlay_carrying_a_bad_glob_still_fails() {
        let err = parse_delegate_mcp(r#"{"autoAcceptTools":["[unterminated"]}"#)
            .expect_err("valid JSON is not enough — it has to validate");

        assert!(err.to_string().contains("failed validation"), "got: {err}");
    }

    /// A harness tool's description is the ONLY guidance the calling
    /// agent gets — there is no README in its context. Pin that each one
    /// exists, is substantial, and names at least one sibling tool, so
    /// the composition contract ("call this first", "poll that, don't
    /// re-spawn") cannot silently rot into a bare one-liner.
    #[test]
    fn every_harness_tool_description_names_a_sibling() {
        let names: Vec<&str> = vec![
            "list_profiles",
            "spawn",
            "session_send",
            "session_list",
            "session_status",
            "session_read",
            "session_kill",
        ];

        for tool in harness_tools() {
            let description = tool
                .description
                .as_deref()
                .unwrap_or_else(|| panic!("{} has no description", tool.name));
            assert!(
                description.len() > 80,
                "{}'s description is too thin to guide a caller: {description}",
                tool.name
            );
            let mentions_sibling = names
                .iter()
                .any(|sibling| *sibling != tool.name.as_ref() && description.contains(sibling));
            assert!(
                mentions_sibling,
                "{}'s description names no sibling tool, so an agent cannot learn the workflow from it: {description}",
                tool.name
            );
        }
    }

    /// Detached is the default, and both launch schemas must SAY so —
    /// the description is the only place a caller learns it, and one
    /// left claiming `default true` is worse than none at all.
    #[test]
    fn a_launch_is_detached_unless_the_caller_asks_to_wait() {
        assert!(!wait_flag(&serde_json::Map::new()).expect("an absent flag is not an error"));

        let mut asked = serde_json::Map::new();
        asked.insert("wait".into(), serde_json::json!(true));
        assert!(wait_flag(&asked).expect("an explicit true still waits"));

        for props in [launch_props(&[]), session_send_props()] {
            let described = props["wait"]["description"].as_str().expect("wait is documented");
            assert!(
                described.contains("default FALSE"),
                "the schema must state the default a caller gets: {described}"
            );
        }
    }

    /// A description that names a tool which no longer exists sends the
    /// calling agent after nothing. This caught a real one: `spawn` kept
    /// saying "use `resume` for every follow-up turn" after `resume`
    /// became `session_send`. The sibling-naming test above missed it
    /// precisely because a retired name is not a sibling.
    #[test]
    fn no_description_names_a_retired_tool() {
        // Names this server used to expose. Add to this list on every
        // rename — it is the cheap half of not stranding callers.
        const RETIRED: &[&str] = &["resume"];

        for tool in harness_tools() {
            let description = tool.description.as_deref().unwrap_or_default();
            for retired in RETIRED {
                let mention = format!("`{retired}`");
                assert!(
                    !description.contains(&mention),
                    "{}'s description points at retired tool `{retired}` — a caller following it would call nothing",
                    tool.name
                );
            }
        }
    }
}

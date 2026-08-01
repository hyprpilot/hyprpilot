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
    CallToolRequestParams, CallToolResult, ErrorCode, Implementation, ListToolsResult, PaginatedRequestParams,
    ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::ServerHandler;
use rmcp::ServiceExt;

use super::harness::Harness;
use super::rpc::{
    empty_object_schema, object_schema, optional_bool, optional_string, optional_string_array, optional_u64,
    optional_usize, require_string, structured_with_text, tool_error, wait_for_shutdown,
};
use crate::config::mcp::DEFAULT_HARNESS_SERVER_NAME;

#[derive(Clone)]
pub struct HarnessServer {
    harness: Arc<Harness>,
}

impl HarnessServer {
    fn new(config: super::ConfigSource, max_sessions: usize) -> Self {
        Self {
            harness: Arc::new(Harness::new(config, max_sessions)),
        }
    }
}

impl ServerHandler for HarnessServer {
    fn get_info(&self) -> ServerInfo {
        let mut caps = ServerCapabilities::default();
        // Fixed for the life of the process.
        let mut tools = rmcp::model::ToolsCapability::default();
        tools.list_changed = Some(false);
        caps.tools = Some(tools);

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
        Ok(ListToolsResult::with_all_items(harness_tools()))
    }

    /// Dispatch one harness tool. Recoverable failures come back as
    /// `Ok(tool_error(..))` so the agent can read and act on them;
    /// `Err` stays reserved for protocol faults (bad params).
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
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
                    Ok(payload) => Ok(structured_with_text(launch_summary(&payload), payload)),
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
        default_value_t = super::harness::DEFAULT_MAX_SESSIONS,
        value_name = "N"
    )]
    pub max_sessions: usize,
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
            "description": "Block until the turn finishes (default true). When false, returns immediately; poll `session_read`.",
        },
        "timeout_seconds": {
            "type": "integer",
            "description": "Seconds to wait when `wait` is true (default 300). On timeout the agent KEEPS RUNNING and the result reports status `running` — poll `session_read`, do not send again.",
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
                "Start a NEW agent session from a profile and send it a prompt. Returns a `session` handle. \
                 With `wait` true (the default) it blocks and returns the transcript; if the turn outlives \
                 `timeout_seconds` the result comes back with status `running` and the agent KEEPS WORKING — \
                 poll `session_read` with the handle, do NOT call `spawn` again. Use `session_send` (not `spawn`) for \
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
            "session_send",
            Some(
                "Send another message to an existing session, continuing the same conversation via the vendor's \
                 own session store. Takes the `session` handle from `spawn` or `session_list`. The result's \
                 `delivery` field says what happened. The handle does NOT change — it stays valid for the whole \
                 conversation, however many turns you send, and the transcript keeps accumulating under it. \
                 The session must have finished its previous turn: sending to a `running` session is refused, \
                 because no vendor supports two concurrent turns on one conversation — poll `session_read` or \
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

/// Human-readable summary for a `spawn` / `session_send` result.
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

/// Run the harness server over stdio.
pub async fn run_harness(args: HarnessArgs, config: super::ConfigSource) -> anyhow::Result<()> {
    tracing::info!(max_sessions = args.max_sessions, "mcp: starting the harness server");
    // Reclaim anything a crashed predecessor left behind before starting
    // our own. A non-empty sweep logs at `warn`.
    super::sessions::sweep_stale_sessions();

    let handler = HarnessServer::new(config, args.max_sessions);
    // Clone the table BEFORE `serve()` — it consumes the handler, and
    // `waiting()` consumes the `RunningService`, so this is the only
    // chance to keep a handle for the shutdown reap.
    let sessions = Arc::clone(&handler.harness.sessions);

    let (stdin, stdout) = rmcp::transport::io::stdio();
    let running = handler
        .serve((stdin, stdout))
        .await
        .map_err(|err| anyhow::anyhow!("mcp harness: serve failed at init: {err}"))?;

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
     2. `spawn { profile, prompt }` — start an agent. It blocks and \
     returns the transcript.\n\
     3. If that result says status `running`, the turn outlived its \
     timeout and the agent is STILL WORKING. Poll \
     `session_read { session, offset }` — do NOT call `spawn` again.\n\
     4. `session_send { session, prompt }` — next turn in the same \
     conversation. The handle does not change.\n\
     5. `session_read` any time, including after the run finished. Pass \
     `wait: true` to follow live; pass a progressToken to receive output \
     as progress notifications as it arrives.\n\
     6. `session_kill { session }` — stops a running session; on an \
     already-finished one it reaps it.\n\
     Sessions are children of THIS server and die with it: they do not \
     survive a restart, and their transcripts go too."
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

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

//! One handler per `ctl` subcommand. Mirrors the server-side
//! `rpc::RpcHandler` / `RpcDispatcher` split: each wire operation gets
//! its own struct that owns its behavior end-to-end, and the clap
//! enum-to-handler mapping in `ctl::run` is the (compile-time) dispatch.
//!
//! Contract: every handler consumes a connected `CtlConnection` and is
//! responsible for its own output + exit semantics. Simple RPC handlers
//! (`session/submit`, `session/cancel`, `session/info`, `window/toggle`,
//! `daemon/kill`) share one helper that pretty-prints the JSON result
//! and calls `exit(1)` on any RPC or transport error. `StatusHandler`
//! is the odd one — it never exits non-zero (waybar's `exec` needs a
//! valid payload even when the daemon is down) and owns a
//! reconnect-with-back-off loop for `--watch`.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use anyhow::Result;
use serde_json::{json, Value};
use tracing::{error, warn};

use crate::ctl::client::{CtlClient, CtlConnection};
use crate::rpc::protocol::{EventsNotifyNotification, Outcome, StatusChangedNotification};

/// Every `ctl` subcommand implements this. `run` receives a
/// [`CtlClient`] — a connection factory — and is responsible for
/// opening whatever connections it needs, handling transport failure
/// per its own semantics, and producing output. Plain subcommands
/// open once and exit 1 on failure; `status` loops the factory with
/// back-off so waybar keeps rendering through daemon restarts.
pub trait CtlHandler {
    fn run(self, client: &CtlClient) -> Result<()>;
}

/// Open a connection, run one JSON-RPC round-trip, pretty-print the
/// `result` payload. Shared body for the plain subcommands that
/// differ only in method + params. Any transport failure, RPC error,
/// or serialization failure logs + writes to stderr + `exit(1)`.
fn emit(client: &CtlClient, method: &str, params: Value) -> Result<()> {
    let mut conn = match client.connect() {
        Ok(c) => c,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    };
    match conn.call(method, params) {
        Ok(Outcome::Success { result }) => {
            println!("{}", serde_json::to_string_pretty(&result)?);
            Ok(())
        }
        Ok(Outcome::Error { error: err }) => {
            error!(code = err.code, message = %err.message, "ctl: rpc error");
            eprintln!("rpc error {}: {}", err.code, err.message);
            std::process::exit(1);
        }
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    }
}

pub struct SubmitHandler {
    pub text: String,
    pub agent_id: Option<String>,
    pub profile_id: Option<String>,
}

impl CtlHandler for SubmitHandler {
    fn run(self, client: &CtlClient) -> Result<()> {
        let mut params = json!({ "text": self.text });
        let obj = params.as_object_mut().expect("json! produces a map");
        if let Some(id) = self.agent_id {
            obj.insert("agentId".into(), Value::String(id));
        }
        if let Some(id) = self.profile_id {
            obj.insert("profileId".into(), Value::String(id));
        }
        emit(client, "session/submit", params)
    }
}

pub struct CancelHandler;

impl CtlHandler for CancelHandler {
    fn run(self, client: &CtlClient) -> Result<()> {
        emit(client, "session/cancel", Value::Null)
    }
}

pub struct ToggleHandler;

impl CtlHandler for ToggleHandler {
    fn run(self, client: &CtlClient) -> Result<()> {
        emit(client, "window/toggle", Value::Null)
    }
}

pub struct KillHandler;

impl CtlHandler for KillHandler {
    fn run(self, client: &CtlClient) -> Result<()> {
        emit(client, "daemon/kill", Value::Null)
    }
}

pub struct SessionInfoHandler;

impl CtlHandler for SessionInfoHandler {
    fn run(self, client: &CtlClient) -> Result<()> {
        emit(client, "session/info", Value::Null)
    }
}

pub struct OverlayPresentHandler {
    pub instance_id: Option<String>,
}

impl CtlHandler for OverlayPresentHandler {
    fn run(self, client: &CtlClient) -> Result<()> {
        let params = match self.instance_id {
            Some(id) => json!({ "instanceId": id }),
            None => Value::Null,
        };
        emit(client, "overlay/present", params)
    }
}

pub struct OverlayHideHandler;

impl CtlHandler for OverlayHideHandler {
    fn run(self, client: &CtlClient) -> Result<()> {
        emit(client, "overlay/hide", Value::Null)
    }
}

pub struct OverlayToggleHandler;

impl CtlHandler for OverlayToggleHandler {
    fn run(self, client: &CtlClient) -> Result<()> {
        emit(client, "overlay/toggle", Value::Null)
    }
}

/// `ctl skills list [--instance <id>]`. Pretty-prints a slug/title/
/// description table. Empty list renders the literal "no skills
/// loaded" message on stderr + exits 0.
pub struct SkillsListHandler {
    pub instance_id: Option<String>,
}

impl CtlHandler for SkillsListHandler {
    fn run(self, client: &CtlClient) -> Result<()> {
        let mut params = json!({});
        if let Some(id) = self.instance_id {
            params
                .as_object_mut()
                .expect("json! produces a map")
                .insert("instanceId".into(), Value::String(id));
        }
        let mut conn = match client.connect() {
            Ok(c) => c,
            Err(err) => {
                eprintln!("{err}");
                std::process::exit(1);
            }
        };
        match conn.call("skills/list", params) {
            Ok(Outcome::Success { result }) => {
                let empty: Vec<Value> = Vec::new();
                let skills = result.get("skills").and_then(Value::as_array).unwrap_or(&empty);
                if skills.is_empty() {
                    eprintln!("no skills loaded");
                    return Ok(());
                }
                let widest_slug = skills
                    .iter()
                    .filter_map(|s| s.get("slug").and_then(Value::as_str))
                    .map(str::len)
                    .max()
                    .unwrap_or(4);
                let widest_title = skills
                    .iter()
                    .filter_map(|s| s.get("title").and_then(Value::as_str))
                    .map(str::len)
                    .max()
                    .unwrap_or(5);
                for s in skills {
                    let slug = s.get("slug").and_then(Value::as_str).unwrap_or("");
                    let title = s.get("title").and_then(Value::as_str).unwrap_or("");
                    let desc = s.get("description").and_then(Value::as_str).unwrap_or("");
                    println!(
                        "{slug:<wslug$}  {title:<wtitle$}  {desc}",
                        wslug = widest_slug,
                        wtitle = widest_title
                    );
                }
                Ok(())
            }
            Ok(Outcome::Error { error: err }) => {
                error!(code = err.code, message = %err.message, "ctl skills list: rpc error");
                eprintln!("rpc error {}: {}", err.code, err.message);
                std::process::exit(1);
            }
            Err(err) => {
                eprintln!("{err}");
                std::process::exit(1);
            }
        }
    }
}

/// `ctl skills get --slug <slug>` — prints the full JSON response.
pub struct SkillsGetHandler {
    pub slug: String,
}

impl CtlHandler for SkillsGetHandler {
    fn run(self, client: &CtlClient) -> Result<()> {
        emit(client, "skills/get", json!({ "slug": self.slug }))
    }
}

/// `ctl skills reload` — prints `{"reloaded": N}` count.
pub struct SkillsReloadHandler;

impl CtlHandler for SkillsReloadHandler {
    fn run(self, client: &CtlClient) -> Result<()> {
        emit(client, "skills/reload", Value::Null)
    }
}

pub struct AgentsListHandler;

impl CtlHandler for AgentsListHandler {
    fn run(self, client: &CtlClient) -> Result<()> {
        emit(client, "agents/list", Value::Null)
    }
}

pub struct CommandsListHandler {
    pub instance_id: String,
}

impl CtlHandler for CommandsListHandler {
    fn run(self, client: &CtlClient) -> Result<()> {
        emit(client, "commands/list", json!({ "instanceId": self.instance_id }))
    }
}

pub struct ModesListHandler {
    pub instance_id: String,
}

impl CtlHandler for ModesListHandler {
    fn run(self, client: &CtlClient) -> Result<()> {
        emit(client, "modes/list", json!({ "instanceId": self.instance_id }))
    }
}

pub struct ModesSetHandler {
    pub instance_id: String,
    pub mode_id: String,
}

impl CtlHandler for ModesSetHandler {
    fn run(self, client: &CtlClient) -> Result<()> {
        emit(
            client,
            "modes/set",
            json!({ "instanceId": self.instance_id, "modeId": self.mode_id }),
        )
    }
}

pub struct ModelsListHandler {
    pub instance_id: String,
}

impl CtlHandler for ModelsListHandler {
    fn run(self, client: &CtlClient) -> Result<()> {
        emit(client, "models/list", json!({ "instanceId": self.instance_id }))
    }
}

pub struct ModelsSetHandler {
    pub instance_id: String,
    pub model_id: String,
}

impl CtlHandler for ModelsSetHandler {
    fn run(self, client: &CtlClient) -> Result<()> {
        emit(
            client,
            "models/set",
            json!({ "instanceId": self.instance_id, "modelId": self.model_id }),
        )
    }
}

/// `ctl prompts send --instance <id> <text>`. Pass `-` for `text` to
/// read from stdin.
pub struct PromptsSendHandler {
    pub instance_id: String,
    pub text: String,
}

impl CtlHandler for PromptsSendHandler {
    fn run(self, client: &CtlClient) -> Result<()> {
        let text = if self.text.trim() == "-" {
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            buf
        } else {
            self.text
        };
        emit(
            client,
            "prompts/send",
            json!({ "instanceId": self.instance_id, "text": text }),
        )
    }
}

pub struct PromptsCancelHandler {
    pub instance_id: String,
}

impl CtlHandler for PromptsCancelHandler {
    fn run(self, client: &CtlClient) -> Result<()> {
        emit(client, "prompts/cancel", json!({ "instanceId": self.instance_id }))
    }
}

/// `ctl permissions pending [--instance <id>]`. Pretty-prints a
/// requestId / instance / tool / args table; empty list renders the
/// literal "no pending permissions" message on stderr + exits 0.
pub struct PermissionsPendingHandler {
    pub instance_id: Option<String>,
}

impl CtlHandler for PermissionsPendingHandler {
    fn run(self, client: &CtlClient) -> Result<()> {
        let mut params = json!({});
        if let Some(id) = self.instance_id {
            params
                .as_object_mut()
                .expect("json! produces a map")
                .insert("instanceId".into(), Value::String(id));
        }
        let mut conn = match client.connect() {
            Ok(c) => c,
            Err(err) => {
                eprintln!("{err}");
                std::process::exit(1);
            }
        };
        match conn.call("permissions/pending", params) {
            Ok(Outcome::Success { result }) => {
                let empty: Vec<Value> = Vec::new();
                let pending = result.get("pending").and_then(Value::as_array).unwrap_or(&empty);
                if pending.is_empty() {
                    eprintln!("no pending permissions");
                    return Ok(());
                }
                let widest_req = pending
                    .iter()
                    .filter_map(|p| p.get("requestId").and_then(Value::as_str))
                    .map(str::len)
                    .max()
                    .unwrap_or(9);
                let widest_inst = pending
                    .iter()
                    .filter_map(|p| p.get("instanceId").and_then(Value::as_str))
                    .map(str::len)
                    .max()
                    .unwrap_or(8);
                let widest_tool = pending
                    .iter()
                    .filter_map(|p| p.get("tool").and_then(Value::as_str))
                    .map(str::len)
                    .max()
                    .unwrap_or(4);
                for p in pending {
                    let req = p.get("requestId").and_then(Value::as_str).unwrap_or("");
                    let inst = p.get("instanceId").and_then(Value::as_str).unwrap_or("");
                    let tool = p.get("tool").and_then(Value::as_str).unwrap_or("");
                    let args = p.get("args").and_then(Value::as_str).unwrap_or("");
                    println!(
                        "{req:<wreq$}  {inst:<winst$}  {tool:<wtool$}  {args}",
                        wreq = widest_req,
                        winst = widest_inst,
                        wtool = widest_tool
                    );
                }
                Ok(())
            }
            Ok(Outcome::Error { error: err }) => {
                error!(code = err.code, message = %err.message, "ctl permissions pending: rpc error");
                eprintln!("rpc error {}: {}", err.code, err.message);
                std::process::exit(1);
            }
            Err(err) => {
                eprintln!("{err}");
                std::process::exit(1);
            }
        }
    }
}

/// `ctl sessions list [--instance <id>] [--agent <id>] [--profile <id>] [--cwd <path>]`.
/// Pretty-prints the JSON response — wire shape lands as
/// `{ sessions: [{ id, title, cwd, lastTurnAt, messageCount }] }`.
pub struct SessionsListHandler {
    pub instance_id: Option<String>,
    pub agent_id: Option<String>,
    pub profile_id: Option<String>,
    pub cwd: Option<std::path::PathBuf>,
}

impl CtlHandler for SessionsListHandler {
    fn run(self, client: &CtlClient) -> Result<()> {
        let mut params = json!({});
        let obj = params.as_object_mut().expect("json! produces a map");
        if let Some(id) = self.instance_id {
            obj.insert("instanceId".into(), Value::String(id));
        }
        if let Some(id) = self.agent_id {
            obj.insert("agentId".into(), Value::String(id));
        }
        if let Some(id) = self.profile_id {
            obj.insert("profileId".into(), Value::String(id));
        }
        if let Some(cwd) = self.cwd {
            obj.insert("cwd".into(), Value::String(cwd.display().to_string()));
        }
        emit(client, "sessions/list", params)
    }
}

/// `ctl sessions forget --id <id>`. Client-side stub — ACP 0.12
/// doesn't expose a session-delete verb yet, so round-tripping
/// through `sessions/forget` would panic the daemon. Surface the
/// gap loudly per CLAUDE.md "stubs panic, don't pretend"; flip
/// to a real `emit(...)` call when ACP lands the underlying verb.
pub struct SessionsForgetHandler {
    pub id: String,
}

impl CtlHandler for SessionsForgetHandler {
    fn run(self, _client: &CtlClient) -> Result<()> {
        unimplemented!(
            "ctl sessions forget '{}': ACP 0.12 does not expose a session-delete verb (track upstream)",
            self.id
        )
    }
}

/// `ctl sessions info --id <id>` — pretty-prints the full record.
/// `-32602` for unknown ids surfaces via the shared `emit` body
/// (stderr + exit 1).
pub struct SessionsInfoHandler {
    pub id: String,
}

impl CtlHandler for SessionsInfoHandler {
    fn run(self, client: &CtlClient) -> Result<()> {
        emit(client, "sessions/info", json!({ "id": self.id }))
    }
}

pub struct PermissionsRespondHandler {
    pub request_id: String,
    pub option_id: String,
}

impl CtlHandler for PermissionsRespondHandler {
    fn run(self, client: &CtlClient) -> Result<()> {
        emit(
            client,
            "permissions/respond",
            json!({ "requestId": self.request_id, "optionId": self.option_id }),
        )
    }
}

/// Drives `ctl status [--watch]`. Always emits a waybar-shaped JSON
/// object (`{text, class, tooltip, alt}`) and always exits 0 — waybar
/// needs a valid payload on stdout even when the daemon is
/// unreachable, so transport / RPC errors fall back to the client-side
/// `"offline"` sentinel rather than propagating. All status-specific
/// knowledge (the waybar line format, the offline sentinel, the
/// `status/subscribe` round-trip + notification stream) lives as
/// associated functions on this struct; `client.rs` stays generic.
pub struct StatusHandler {
    pub watch: bool,
}

impl CtlHandler for StatusHandler {
    fn run(self, client: &CtlClient) -> Result<()> {
        if self.watch {
            Self::watch_loop(client)
        } else {
            Self::one_shot(client)
        }
    }
}

impl StatusHandler {
    /// `--watch = false`: single `status/get`, one line, exit 0.
    /// Transport / RPC errors fall through to the offline sentinel.
    fn one_shot(client: &CtlClient) -> Result<()> {
        let value = match client.connect().and_then(|mut c| c.call("status/get", Value::Null)) {
            Ok(Outcome::Success { result }) => result,
            Ok(Outcome::Error { error: err }) => {
                warn!(code = err.code, message = %err.message, "status/get rpc error — emitting offline");
                Self::offline()
            }
            Err(err) => {
                warn!(%err, "status/get transport error — emitting offline");
                Self::offline()
            }
        };
        println!("{}", Self::format(&value));
        Ok(())
    }

    /// `--watch = true`: loop forever. Each iteration opens a
    /// subscription, streams `status/changed` notifications until EOF
    /// or read error, then sleeps with 1s → 2s → 5s back-off and
    /// reconnects. Emits an offline line between attempts so waybar
    /// reflects the transport gap.
    fn watch_loop(client: &CtlClient) -> Result<()> {
        let backoffs = [Duration::from_secs(1), Duration::from_secs(2), Duration::from_secs(5)];
        let mut backoff_idx = 0usize;

        loop {
            match Self::stream_once(client) {
                Ok(()) => backoff_idx = 0, // clean EOF, reconnect immediately
                Err(err) => warn!(%err, "ctl status --watch: connection lost, emitting offline"),
            }

            println!("{}", Self::format(&Self::offline()));
            let _ = std::io::stdout().flush();

            let delay = backoffs[backoff_idx.min(backoffs.len() - 1)];
            if backoff_idx < backoffs.len() - 1 {
                backoff_idx += 1;
            }
            std::thread::sleep(delay);
        }
    }

    /// One subscription lifecycle: connect, send `status/subscribe`,
    /// print the initial snapshot, then drain notifications until EOF
    /// or read error.
    fn stream_once(client: &CtlClient) -> Result<()> {
        let conn = client.connect()?;
        let (snapshot, stream) = Self::subscribe(conn)?;

        println!("{}", Self::format(&snapshot));
        let _ = std::io::stdout().flush();

        for sr in stream {
            let value = sr?;
            println!("{}", Self::format(&value));
            let _ = std::io::stdout().flush();
        }
        Ok(())
    }

    /// Send `status/subscribe` on `conn`, return the initial snapshot
    /// plus a blocking iterator over server-pushed `status/changed`
    /// notifications. Consumes the connection — once subscribed, the
    /// server won't reply to further requests on the same socket until
    /// the subscription ends, so the writer half is dropped here.
    fn subscribe(mut conn: CtlConnection) -> Result<(Value, StatusStream)> {
        let initial: Value = conn.fire("status/subscribe", Value::Null)?;
        Ok((
            initial,
            StatusStream {
                reader: conn.into_reader(),
            },
        ))
    }

    /// Client-side sentinel emitted whenever the daemon is unreachable
    /// or an RPC error prevents a real status snapshot from landing.
    /// Shaped to match the server-side `StatusResult` so `Self::format`
    /// treats it like any other state.
    fn offline() -> Value {
        json!({ "state": "offline", "visible": false, "active_session": null })
    }

    /// Format a `StatusResult`-shaped value as one line of waybar
    /// custom-module JSON. Picks `text` / `class` / `tooltip` from the
    /// state, uses the raw state string as the `alt` label. The
    /// `"offline"` state is a client-side sentinel (see `Self::offline`)
    /// — it is *not* an `AgentState` variant on the server side.
    fn format(status: &Value) -> String {
        let state = status["state"].as_str().unwrap_or("unknown");
        let (text, class, tooltip) = match state {
            "idle" => ("", "idle", "hyprpilot: idle"),
            "streaming" => ("\u{25cf}", "streaming", "hyprpilot: agent is responding"),
            "awaiting" => ("?", "awaiting", "hyprpilot: awaiting input"),
            "error" => ("!", "error", "hyprpilot: last session errored"),
            "offline" => ("", "offline", "hyprpilot: offline"),
            other => ("", other, "hyprpilot: unknown state"),
        };
        json!({ "text": text, "class": class, "tooltip": tooltip, "alt": state }).to_string()
    }
}

/// Blocking iterator over server-pushed `status/changed` notifications.
/// Yielded by `StatusHandler::subscribe`. Each `next` call blocks on
/// the underlying reader until a line arrives or EOF closes the
/// stream. Malformed / unexpected lines are logged and skipped so a
/// single bad line doesn't kill the watcher.
struct StatusStream {
    reader: BufReader<UnixStream>,
}

impl Iterator for StatusStream {
    type Item = Result<Value>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let mut line = String::new();
            match self.reader.read_line(&mut line) {
                Ok(0) => return None, // clean EOF
                Ok(_) => {
                    let trimmed = line.trim_end();
                    if trimmed.is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<StatusChangedNotification>(trimmed) {
                        Ok(notif) => return Some(Ok(serde_json::to_value(&notif.params).expect("serializes"))),
                        Err(_) => {
                            warn!("ctl status --watch: unexpected line from daemon: {trimmed}");
                            continue;
                        }
                    }
                }
                Err(err) => return Some(Err(anyhow::Error::new(err).context("read notification"))),
            }
        }
    }
}

/// `ctl events tail [--topics A,B,C] [--instance <id>]` — opens a
/// connection, calls `events/subscribe`, then loops `into_reader().lines()`
/// printing one JSON line per `events/notify` notification. No
/// reconnect — events are live-only per the issue. Ctrl-C tears the
/// connection down and exits.
pub struct EventsTailHandler {
    pub topics: Vec<String>,
    pub instance_id: Option<String>,
}

impl CtlHandler for EventsTailHandler {
    fn run(self, client: &CtlClient) -> Result<()> {
        let mut conn = client.connect()?;

        let mut params = json!({});
        let obj = params.as_object_mut().expect("json! produces a map");
        if !self.topics.is_empty() {
            obj.insert(
                "topics".into(),
                Value::Array(self.topics.into_iter().map(Value::String).collect()),
            );
        }
        if let Some(id) = self.instance_id {
            obj.insert("instanceId".into(), Value::String(id));
        }

        let initial: Value = match conn.fire("events/subscribe", params) {
            Ok(v) => v,
            Err(err) => {
                eprintln!("{err}");
                std::process::exit(1);
            }
        };
        // Echo the subscription id once so a reviewer can spot the
        // connection in the daemon logs.
        eprintln!("subscribed: {}", initial);
        let _ = std::io::stdout().flush();

        let mut reader = conn.into_reader();
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => return Ok(()),
                Ok(_) => {
                    let trimmed = line.trim_end();
                    if trimmed.is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<EventsNotifyNotification>(trimmed) {
                        Ok(notif) => {
                            let v = serde_json::to_value(&notif.params).expect("EventsNotifyParams serializes");
                            println!("{v}");
                            let _ = std::io::stdout().flush();
                        }
                        Err(_) => {
                            warn!("ctl events tail: unexpected line from daemon: {trimmed}");
                            continue;
                        }
                    }
                }
                Err(err) => {
                    return Err(anyhow::Error::new(err).context("read events/notify"));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Covers every known `AgentState` variant plus the `"offline"` client
    /// sentinel and an unknown-state fallback in one pass. Asserts the
    /// emitted line parses as JSON and carries the expected waybar fields.
    #[test]
    fn status_format_renders_waybar_json_per_state() {
        let cases: &[(&str, &str, &str, &str)] = &[
            ("idle", "", "idle", "hyprpilot: idle"),
            ("streaming", "\u{25cf}", "streaming", "hyprpilot: agent is responding"),
            ("awaiting", "?", "awaiting", "hyprpilot: awaiting input"),
            ("error", "!", "error", "hyprpilot: last session errored"),
            ("offline", "", "offline", "hyprpilot: offline"),
            ("made-up", "", "made-up", "hyprpilot: unknown state"),
        ];

        for (state, text, class, tooltip) in cases {
            let sr = json!({ "state": state, "visible": false, "active_session": null });
            let line = StatusHandler::format(&sr);
            let v: Value = serde_json::from_str(&line)
                .unwrap_or_else(|e| panic!("StatusHandler::format({state}) produced invalid JSON: {e} — line: {line}"));
            assert_eq!(v["text"], *text, "text mismatch for state {state}");
            assert_eq!(v["class"], *class, "class mismatch for state {state}");
            assert_eq!(v["tooltip"], *tooltip, "tooltip mismatch for state {state}");
            assert_eq!(v["alt"], *state, "alt mismatch for state {state}");
        }
    }
}

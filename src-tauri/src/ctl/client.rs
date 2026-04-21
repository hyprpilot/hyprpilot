use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};
use tracing::warn;

use crate::rpc::protocol::{Outcome, RequestId, Response, StatusChangedNotification};

/// Synchronous client owning one unix-socket connection. Every CLI
/// round-trip — `status/get`, `status/subscribe`, `submit`, `toggle`,
/// `kill`, `cancel`, `session-info`, and every future namespace —
/// goes through `CtlConnection::call` / `call_outcome`.
///
/// Request ids are per-call UUID v4 strings. There is no monotonic
/// counter on the client side: each call embeds a fresh
/// `uuid::Uuid::new_v4().to_string()` into `RequestId::String`, which
/// the daemon echoes back verbatim. The one-connection-per-process
/// lifecycle of `ctl` makes id uniqueness trivial; UUIDs keep that
/// true even if we ever start multiplexing connections.
///
/// Connection errors for "daemon not running" (`ENOENT` /
/// `ECONNREFUSED`) surface as a friendly message; everything else
/// bubbles up as-is.
pub struct CtlConnection {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
}

impl CtlConnection {
    /// Connect to `socket`. Returns an error mapped to the friendly
    /// "hyprpilot daemon is not running" message on `ENOENT` /
    /// `ECONNREFUSED`.
    pub fn connect(socket: &Path) -> Result<Self> {
        let stream = UnixStream::connect(socket).map_err(|err| {
            if matches!(err.kind(), ErrorKind::NotFound | ErrorKind::ConnectionRefused) {
                anyhow!("hyprpilot daemon is not running")
            } else {
                anyhow::Error::new(err).context(format!("failed to connect to {}", socket.display()))
            }
        })?;
        let writer = stream.try_clone().context("clone socket for write")?;
        Ok(Self {
            writer,
            reader: BufReader::new(stream),
        })
    }

    /// Single JSON-RPC round-trip. Serializes the `method` + `params`,
    /// writes one NDJSON line, reads one line back, parses the outcome.
    pub fn call<Req: Serialize, Resp: DeserializeOwned>(&mut self, method: &str, params: Req) -> Result<Resp> {
        let id = RequestId::String(uuid::Uuid::new_v4().to_string());
        let envelope = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        self.write_line(&envelope)?;

        let line = self.read_line_context("read response line")?;
        let response: Response =
            serde_json::from_str(line.trim_end()).with_context(|| format!("parse response: {}", line.trim_end()))?;
        match response.outcome {
            Outcome::Success { result } => serde_json::from_value(result).context("deserialize result"),
            Outcome::Error { error } => bail!("rpc error {}: {}", error.code, error.message),
        }
    }

    /// Variant of `call` that preserves the `Outcome` instead of
    /// collapsing errors into `anyhow`. Used by paths that want to
    /// inspect the JSON-RPC error code (e.g. `status/get` in one-shot
    /// mode falls back to the offline payload on error).
    pub fn call_outcome(&mut self, method: &str, params: Value) -> Result<Outcome> {
        let id = RequestId::String(uuid::Uuid::new_v4().to_string());
        let envelope = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        self.write_line(&envelope)?;

        let line = self.read_line_context("read response line")?;
        let response: Response =
            serde_json::from_str(line.trim_end()).with_context(|| format!("parse response: {}", line.trim_end()))?;
        Ok(response.outcome)
    }

    /// Convert this connection into a subscriber stream: send
    /// `status/subscribe`, return the initial snapshot + a blocking
    /// iterator over `status/changed` notifications.
    pub fn subscribe_status(mut self) -> Result<(Value, StatusStream)> {
        let initial: Value = self.call("status/subscribe", Value::Null)?;
        Ok((initial, StatusStream { reader: self.reader }))
    }

    fn write_line(&mut self, envelope: &Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(envelope).context("serialize request")?;
        bytes.push(b'\n');
        self.writer.write_all(&bytes).context("write request")?;
        self.writer.flush().context("flush request")?;
        Ok(())
    }

    fn read_line_context(&mut self, ctx: &'static str) -> Result<String> {
        let mut line = String::new();
        let n = self.reader.read_line(&mut line).context(ctx)?;
        if n == 0 {
            bail!("daemon closed the connection without responding");
        }
        Ok(line)
    }
}

/// Blocking iterator over server-pushed `status/changed` notifications.
/// Returned by `CtlConnection::subscribe_status`. Each `next` call
/// blocks on the underlying reader until a line arrives or EOF closes
/// the stream. Malformed / unexpected lines are logged and skipped so
/// a single bad line doesn't kill the watcher.
pub struct StatusStream {
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

/// Format a `StatusResult` (or offline sentinel) as the output line — always
/// the waybar custom-module JSON shape.
fn format_line(sr: &Value) -> String {
    let waybar = to_waybar(sr);
    serde_json::to_string(&waybar).unwrap_or_default()
}

/// Convert a `StatusResult`-shaped JSON value to a waybar custom-module JSON
/// object with `text`, `class`, `tooltip`, and `alt` fields. The `"offline"`
/// state is a client-side sentinel emitted when the daemon is unreachable —
/// it is *not* an `AgentState` variant on the server side.
pub fn to_waybar(status: &Value) -> Value {
    let state = status["state"].as_str().unwrap_or("unknown");
    let (text, class, tooltip) = match state {
        "idle" => ("", "idle", "hyprpilot: idle"),
        "streaming" => ("\u{25cf}", "streaming", "hyprpilot: agent is responding"),
        "awaiting" => ("?", "awaiting", "hyprpilot: awaiting input"),
        "error" => ("!", "error", "hyprpilot: last session errored"),
        "offline" => ("", "offline", "hyprpilot: offline"),
        other => ("", other, "hyprpilot: unknown state"),
    };
    json!({
        "text": text,
        "class": class,
        "tooltip": tooltip,
        "alt": state
    })
}

/// Entry point for `ctl status [--watch]`.
///
/// One-shot (`watch = false`): connect, call `status/get`, print one
/// waybar line, exit 0. On transport / RPC error, emit the offline
/// sentinel instead — waybar's `exec` field needs a valid payload.
///
/// Watch (`watch = true`): loop forever. Each iteration opens a
/// subscription via `stream_once`; when that returns (clean EOF or
/// transport error) the loop prints an offline sentinel, sleeps with
/// 1s → 2s → 5s back-off, and reconnects.
pub fn run_status(socket: &Path, watch: bool) -> Result<()> {
    if !watch {
        let value = match CtlConnection::connect(socket).and_then(|mut c| c.call_outcome("status/get", Value::Null)) {
            Ok(Outcome::Success { result }) => result,
            Ok(Outcome::Error { error }) => {
                warn!(code = error.code, message = %error.message, "status/get rpc error — emitting offline");
                json!({ "state": "offline", "visible": false, "active_session": null })
            }
            Err(err) => {
                warn!(%err, "status/get transport error — emitting offline");
                json!({ "state": "offline", "visible": false, "active_session": null })
            }
        };
        println!("{}", format_line(&value));
        return Ok(());
    }

    // Watch mode: loop forever; back off on transport errors.
    let backoffs = [Duration::from_secs(1), Duration::from_secs(2), Duration::from_secs(5)];
    let mut backoff_idx = 0usize;
    loop {
        match stream_once(socket) {
            Ok(()) => {
                // Clean EOF — reconnect immediately (shouldn't happen normally).
                backoff_idx = 0;
                continue;
            }
            Err(err) => warn!(%err, "ctl status --watch: connection lost, emitting offline"),
        }

        println!(
            "{}",
            format_line(&json!({ "state": "offline", "visible": false, "active_session": null }))
        );
        let _ = std::io::Write::flush(&mut std::io::stdout());

        let delay = backoffs[backoff_idx.min(backoffs.len() - 1)];
        if backoff_idx < backoffs.len() - 1 {
            backoff_idx += 1;
        }
        std::thread::sleep(delay);
    }
}

/// One subscription lifecycle: connect, subscribe, print the initial
/// snapshot, then drain the `StatusStream` until EOF or read error.
fn stream_once(socket: &Path) -> Result<()> {
    let conn = CtlConnection::connect(socket)?;
    let (snapshot, stream) = conn.subscribe_status()?;

    println!("{}", format_line(&snapshot));
    let _ = std::io::Write::flush(&mut std::io::stdout());

    for sr in stream {
        let value = sr?;
        println!("{}", format_line(&value));
        let _ = std::io::Write::flush(&mut std::io::stdout());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_waybar_idle() {
        let sr = json!({ "state": "idle", "visible": true, "active_session": null });
        let w = to_waybar(&sr);
        assert_eq!(w["text"], "");
        assert_eq!(w["class"], "idle");
        assert_eq!(w["tooltip"], "hyprpilot: idle");
        assert_eq!(w["alt"], "idle");
    }

    #[test]
    fn to_waybar_streaming() {
        let sr = json!({ "state": "streaming", "visible": true, "active_session": null });
        let w = to_waybar(&sr);
        assert_eq!(w["text"], "\u{25cf}");
        assert_eq!(w["class"], "streaming");
        assert_eq!(w["tooltip"], "hyprpilot: agent is responding");
        assert_eq!(w["alt"], "streaming");
    }

    #[test]
    fn to_waybar_awaiting() {
        let sr = json!({ "state": "awaiting", "visible": false, "active_session": null });
        let w = to_waybar(&sr);
        assert_eq!(w["text"], "?");
        assert_eq!(w["class"], "awaiting");
        assert_eq!(w["tooltip"], "hyprpilot: awaiting input");
        assert_eq!(w["alt"], "awaiting");
    }

    #[test]
    fn to_waybar_error() {
        let sr = json!({ "state": "error", "visible": false, "active_session": null });
        let w = to_waybar(&sr);
        assert_eq!(w["text"], "!");
        assert_eq!(w["class"], "error");
        assert_eq!(w["tooltip"], "hyprpilot: last session errored");
        assert_eq!(w["alt"], "error");
    }

    #[test]
    fn to_waybar_offline() {
        let sr = json!({ "state": "offline", "visible": false, "active_session": null });
        let w = to_waybar(&sr);
        assert_eq!(w["class"], "offline");
        assert_eq!(w["alt"], "offline");
    }

    #[test]
    fn format_line_emits_waybar_json_object() {
        let sr = json!({ "state": "idle", "visible": true, "active_session": null });
        let line = format_line(&sr);
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["class"], "idle");
        assert_eq!(v["alt"], "idle");
    }
}

//! `ctl status [--watch]`. Always emits a waybar-shaped JSON object
//! (`{text, class, tooltip, alt}`) and always exits 0 — waybar's `exec`
//! contract requires a valid stdout payload even when the daemon is
//! unreachable, so transport / RPC errors fall back to the client-side
//! `"offline"` sentinel instead of propagating.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use anyhow::{bail, Result};
use serde_json::{json, Value};
use tracing::warn;

use crate::ctl::client::{CtlClient, CtlConnection};
use crate::rpc::protocol::{Outcome, StatusChangedNotification};

/// Entry point for the `Status { watch }` clap arm. One-shot mode does
/// a single `status/get` and exits; watch mode loops with reconnect
/// back-off and streams `status/changed` notifications.
pub(super) fn run(client: &CtlClient, watch: bool) -> Result<()> {
    if watch {
        watch_loop(client)
    } else {
        one_shot(client)
    }
}

/// `--watch = false`: single `status/get`, one line, exit 0.
/// Transport / RPC errors fall through to the offline sentinel.
fn one_shot(client: &CtlClient) -> Result<()> {
    let value = match client.connect().and_then(|mut c| c.call("status/get", Value::Null)) {
        Ok(Outcome::Success { result }) => result,
        Ok(Outcome::Error { error: err }) => {
            warn!(code = err.code, message = %err.message, "status/get rpc error — emitting offline");
            offline()
        }
        Err(err) => {
            warn!(%err, "status/get transport error — emitting offline");
            offline()
        }
    };
    println!("{}", format(&value));
    Ok(())
}

/// `--watch = true`: loop forever. Each iteration opens a subscription,
/// streams `status/changed` notifications until EOF or read error,
/// then sleeps with 1s → 2s → 5s back-off and reconnects. Emits an
/// offline line between attempts so waybar reflects the transport gap.
fn watch_loop(client: &CtlClient) -> Result<()> {
    let backoffs = [Duration::from_secs(1), Duration::from_secs(2), Duration::from_secs(5)];
    let mut backoff_idx = 0usize;

    loop {
        match stream_once(client) {
            Ok(()) => backoff_idx = 0, // clean EOF, reconnect immediately
            Err(err) => warn!(%err, "ctl status --watch: connection lost, emitting offline"),
        }

        println!("{}", format(&offline()));
        let _ = std::io::stdout().flush();

        let delay = backoffs[backoff_idx.min(backoffs.len() - 1)];
        if backoff_idx < backoffs.len() - 1 {
            backoff_idx += 1;
        }
        std::thread::sleep(delay);
    }
}

/// One subscription lifecycle: connect, send `status/subscribe`, print
/// the initial snapshot, then drain notifications until EOF or read
/// error.
fn stream_once(client: &CtlClient) -> Result<()> {
    let conn = client.connect()?;
    let (snapshot, stream) = subscribe(conn)?;

    println!("{}", format(&snapshot));
    let _ = std::io::stdout().flush();

    for sr in stream {
        let value = sr?;
        println!("{}", format(&value));
        let _ = std::io::stdout().flush();
    }
    Ok(())
}

/// Send `status/subscribe` on `conn`, return the initial snapshot plus
/// a blocking iterator over server-pushed `status/changed`
/// notifications. Consumes the connection — once subscribed, the
/// server won't reply to further requests on the same socket until the
/// subscription ends, so the writer half is dropped here.
fn subscribe(mut conn: CtlConnection) -> Result<(Value, StatusStream)> {
    let initial = match conn.call("status/subscribe", Value::Null)? {
        Outcome::Success { result } => result,
        Outcome::Error { error } => bail!("rpc error {}: {}", error.code, error.message),
    };
    Ok((
        initial,
        StatusStream {
            reader: conn.into_reader(),
        },
    ))
}

/// Client-side sentinel emitted whenever the daemon is unreachable or
/// an RPC error prevents a real status snapshot from landing. Shaped
/// to match the server-side `StatusResult` so `format` treats it like
/// any other state.
fn offline() -> Value {
    json!({ "state": "offline", "visible": false, "active_session": null })
}

/// Format a `StatusResult`-shaped value as one line of waybar
/// custom-module JSON. The `"offline"` state is a client-side sentinel
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

/// Blocking iterator over server-pushed `status/changed` notifications.
/// Yielded by `subscribe`. Each `next` call blocks on the underlying
/// reader until a line arrives or EOF closes the stream. Malformed /
/// unexpected lines are logged and skipped so a single bad line
/// doesn't kill the watcher.
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
            let line = format(&sr);
            let v: Value = serde_json::from_str(&line)
                .unwrap_or_else(|e| panic!("format({state}) produced invalid JSON: {e} — line: {line}"));
            assert_eq!(v["text"], *text, "text mismatch for state {state}");
            assert_eq!(v["class"], *class, "class mismatch for state {state}");
            assert_eq!(v["tooltip"], *tooltip, "tooltip mismatch for state {state}");
            assert_eq!(v["alt"], *state, "alt mismatch for state {state}");
        }
    }
}

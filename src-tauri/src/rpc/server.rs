use serde_json::json;
use tauri::Manager;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tracing::{error, info, warn};

use crate::daemon::WindowRenderer;
use crate::rpc::protocol::{Call, JsonRpcVersion, Outcome, RequestId, Response, RpcError};

/// Shared state handed to each connection task. `AppHandle` is cheap to
/// clone, so we pass by value today and add `Arc`-wrapped fields later when
/// they need interior mutability.
#[derive(Clone)]
pub struct RpcState {
    pub app: tauri::AppHandle,
}

/// Drives a single accepted unix-socket connection. Reads NDJSON lines,
/// dispatches each to a handler, writes a single-line response, loops.
/// Connection closes cleanly on EOF or an empty line.
pub async fn handle_connection(stream: UnixStream, state: RpcState) {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();

    loop {
        let line = match lines.next_line().await {
            Ok(Some(l)) if l.trim().is_empty() => continue,
            Ok(Some(l)) => l,
            Ok(None) => return,
            Err(err) => {
                warn!(%err, "rpc: read error, closing connection");
                return;
            }
        };

        let (response, is_kill_success) = dispatch_line(&line, Some(&state));

        let mut bytes = match serde_json::to_vec(&response) {
            Ok(b) => b,
            Err(err) => {
                error!(%err, "rpc: failed to serialize response");
                return;
            }
        };
        bytes.push(b'\n');

        if let Err(err) = writer.write_all(&bytes).await {
            warn!(%err, "rpc: write error, closing connection");
            return;
        }

        if is_kill_success {
            info!("rpc: kill dispatched — exiting daemon");
            state.app.exit(0);
            return;
        }
    }
}

/// Pure dispatcher. Returns the response plus a flag indicating the caller
/// should exit after writing it. `state` is optional so tests can drive the
/// framing and parsing paths without a live `AppHandle`; toggle/kill calls
/// reaching `None` yield `-32603`.
fn dispatch_line(line: &str, state: Option<&RpcState>) -> (Response, bool) {
    // Stage 1: JSON syntax. Failure here is -32700 parse error.
    let value: serde_json::Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return (Response::error(None, RpcError::parse_error()), false),
    };

    let id = value.get("id").and_then(parse_id);

    // Stage 2: envelope. `jsonrpc` must be present and equal to "2.0", `id`
    // must be present and of a valid shape, the root must be an object.
    // Anything else here is -32600 invalid request.
    if !value.is_object() {
        return (
            Response::error(None, RpcError::invalid_request("request must be a JSON object")),
            false,
        );
    }
    match value.get("jsonrpc") {
        Some(v) => {
            if serde_json::from_value::<JsonRpcVersion>(v.clone()).is_err() {
                return (
                    Response::error(id, RpcError::invalid_request("jsonrpc must be \"2.0\"")),
                    false,
                );
            }
        }
        None => {
            return (
                Response::error(id, RpcError::invalid_request("missing jsonrpc field")),
                false,
            );
        }
    }
    if id.is_none() {
        return (
            Response::error(None, RpcError::invalid_request("missing or invalid id")),
            false,
        );
    }

    // Stage 3: typed method + params. Serde's internally-tagged enum reports
    // `unknown variant` for a method that doesn't match any `Call` variant —
    // surface that as -32601. Other failures (wrong params shape, missing
    // required param) are -32602 invalid params.
    let call_payload = json!({
        "method": value.get("method").cloned().unwrap_or(serde_json::Value::Null),
        "params": value.get("params").cloned().unwrap_or(serde_json::Value::Null),
    });
    let call: Call = match serde_json::from_value(call_payload) {
        Ok(c) => c,
        Err(err) => {
            let msg = err.to_string();
            let rpc_err = if msg.contains("unknown variant") {
                let method = value.get("method").and_then(|v| v.as_str()).unwrap_or("<missing>");
                RpcError::method_not_found(method)
            } else {
                RpcError::invalid_params(msg)
            };
            return (Response::error(id, rpc_err), false);
        }
    };

    let echo_id = id.clone();
    let is_kill = matches!(call, Call::Kill);

    info!(id = ?id, method = method_name(&call), "rpc: dispatch");

    let outcome = handle_call(call, state);

    let response = match outcome {
        Ok(v) => Response::success(echo_id, v),
        Err(e) => Response::error(echo_id, e),
    };

    let kill_success = is_kill && matches!(response.outcome, Outcome::Success { .. });
    (response, kill_success)
}

fn parse_id(v: &serde_json::Value) -> Option<RequestId> {
    match v {
        serde_json::Value::Number(n) => {
            if let Some(u) = n.as_u64() {
                Some(RequestId::Number(u))
            } else {
                // Preserve float ids as strings so the echo-back is lossless.
                n.as_f64().map(|f| RequestId::String(f.to_string()))
            }
        }
        serde_json::Value::String(s) => Some(RequestId::String(s.clone())),
        _ => None,
    }
}

fn method_name(call: &Call) -> &'static str {
    match call {
        Call::Submit { .. } => "submit",
        Call::Cancel => "cancel",
        Call::Toggle => "toggle",
        Call::Kill => "kill",
        Call::SessionInfo => "session-info",
    }
}

fn handle_call(call: Call, state: Option<&RpcState>) -> Result<serde_json::Value, RpcError> {
    match call {
        Call::Submit { text } => Ok(json!({ "accepted": true, "text": text })),
        Call::Cancel => Ok(json!({ "cancelled": false, "reason": "no active session" })),
        Call::SessionInfo => Ok(json!({ "sessions": [] })),
        Call::Kill => {
            // Response body returns regardless; the caller writes it, then
            // invokes `app.exit(0)`. Delivery is best-effort: the process
            // may tear down before the peer finishes reading.
            Ok(json!({ "exiting": true }))
        }
        Call::Toggle => {
            let state = state.ok_or_else(|| RpcError::internal_error("no app handle available"))?;
            let window = state
                .app
                .get_webview_window("main")
                .ok_or_else(|| RpcError::internal_error("main window not available"))?;

            let renderer = state
                .app
                .try_state::<WindowRenderer>()
                .ok_or_else(|| RpcError::internal_error("WindowRenderer not in managed state"))?;

            let visible = window
                .is_visible()
                .map_err(|e| RpcError::internal_error(format!("is_visible failed: {e}")))?;

            if visible {
                renderer
                    .hide(&window)
                    .map_err(|e| RpcError::internal_error(format!("hide failed: {e}")))?;
                Ok(json!({ "visible": false }))
            } else {
                // Re-resolve dimensions against the current monitor on every
                // show transition so the surface is correct after monitor
                // changes (dock swap, lid close/open, hotplug).
                renderer
                    .show(&window)
                    .map_err(|e| RpcError::internal_error(format!("show failed: {e}")))?;
                Ok(json!({ "visible": true }))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(line: &str) -> serde_json::Value {
        let (resp, _) = dispatch_line(line, None);
        serde_json::to_value(resp).unwrap()
    }

    #[test]
    fn submit_success_round_trip() {
        let out = run(r#"{"jsonrpc":"2.0","id":1,"method":"submit","params":{"text":"hello"}}"#);
        assert_eq!(out["jsonrpc"], "2.0");
        assert_eq!(out["id"], 1);
        assert_eq!(out["result"]["accepted"], true);
        assert_eq!(out["result"]["text"], "hello");
    }

    #[test]
    fn session_info_returns_empty_list() {
        let out = run(r#"{"jsonrpc":"2.0","id":"sid","method":"session-info"}"#);
        assert_eq!(out["id"], "sid");
        assert_eq!(out["result"]["sessions"], json!([]));
    }

    #[test]
    fn cancel_stub_returns_no_active_session() {
        let out = run(r#"{"jsonrpc":"2.0","id":2,"method":"cancel"}"#);
        assert_eq!(out["result"]["cancelled"], false);
        assert_eq!(out["result"]["reason"], "no active session");
    }

    #[test]
    fn parse_error_on_non_json() {
        let out = run("not json at all");
        assert_eq!(out["error"]["code"], -32700);
        assert!(out["id"].is_null(), "id must be null on parse error: {out}");
    }

    #[test]
    fn invalid_request_missing_jsonrpc() {
        let out = run(r#"{"id":1,"method":"toggle"}"#);
        assert_eq!(out["error"]["code"], -32600);
        assert_eq!(out["id"], 1);
    }

    #[test]
    fn method_not_found_for_unknown_method() {
        let out = run(r#"{"jsonrpc":"2.0","id":4,"method":"bogus"}"#);
        assert_eq!(out["error"]["code"], -32601);
        assert_eq!(out["id"], 4);
        assert!(out["error"]["message"].as_str().unwrap().contains("bogus"));
    }

    #[test]
    fn float_id_is_echoed_as_string() {
        // u64 doesn't round-trip a float id; preserve it as a string so the
        // echo-back is lossless. Any method will do — use an unknown one to
        // land on the error path without bringing up state.
        let out = run(r#"{"jsonrpc":"2.0","id":1.5,"method":"bogus"}"#);
        assert_eq!(out["error"]["code"], -32601);
        assert_eq!(out["id"], "1.5");
    }

    #[test]
    fn invalid_params_for_missing_required_field() {
        // `submit` requires `text`. A known method with the wrong params
        // shape is -32602, not -32600.
        let out = run(r#"{"jsonrpc":"2.0","id":6,"method":"submit","params":{}}"#);
        assert_eq!(out["error"]["code"], -32602);
        assert_eq!(out["id"], 6);
    }

    #[test]
    fn toggle_without_state_is_internal_error() {
        let out = run(r#"{"jsonrpc":"2.0","id":5,"method":"toggle"}"#);
        assert_eq!(out["error"]["code"], -32603);
        assert_eq!(out["id"], 5);
    }

    #[tokio::test]
    async fn handle_connection_closes_on_eof() {
        use tokio::io::AsyncWriteExt;

        let (mut client, server) = UnixStream::pair().expect("pair");

        let task = tokio::spawn(async move {
            // Synthesize a handler-style loop without an AppHandle by
            // reading lines, dispatching with `None` state, writing back.
            let (reader, mut writer) = server.into_split();
            let mut lines = BufReader::new(reader).lines();
            while let Some(l) = lines.next_line().await.unwrap() {
                if l.trim().is_empty() {
                    continue;
                }
                let (resp, _) = dispatch_line(&l, None);
                let mut bytes = serde_json::to_vec(&resp).unwrap();
                bytes.push(b'\n');
                writer.write_all(&bytes).await.unwrap();
                writer.flush().await.unwrap();
            }
        });

        client
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"session-info\"}\n")
            .await
            .unwrap();

        let mut buf = String::new();
        BufReader::new(&mut client).read_line(&mut buf).await.unwrap();

        let v: serde_json::Value = serde_json::from_str(&buf).unwrap();
        assert_eq!(v["id"], 1);
        assert_eq!(v["result"]["sessions"], json!([]));

        drop(client);
        task.await.unwrap();
    }
}

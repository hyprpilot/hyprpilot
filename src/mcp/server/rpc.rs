//! Shared JSON-RPC plumbing for the three in-tree MCP servers.
//!
//! Schema builders, result wrappers, and argument decoders every
//! `ServerHandler` needs. These lived in `serve.rs` — the skills
//! server — for the historical reason that it was the first server
//! written; five of them had no caller there at all.

use std::sync::Arc;

use rmcp::model::{CallToolResponse, CallToolResult, ContentBlock};
use rmcp::service::RoleServer;
use rmcp::ServerHandler;

/// Return once the MCP transport closes or a termination signal
/// arrives, whichever comes first.
pub(super) async fn wait_for_shutdown<H: ServerHandler>(running: rmcp::service::RunningService<RoleServer, H>) {
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
                tracing::info!("mcp::server: SIGTERM received; shutting down");
            }
            Some(()) = async { match hup.as_mut() { Some(s) => s.recv().await, None => None } } => {
                tracing::info!("mcp::server: SIGHUP received; shutting down");
            }
        }
    }
    #[cfg(not(unix))]
    {
        running.waiting().await.ok();
    }
}

/// Compact builder emitting the shape every hand-rolled schema in the
/// servers produces — `type` / `properties` / `required` (omitted when
/// empty, matching `empty_object_schema`) / `additionalProperties:
/// false`. Worth the helper once a tool has more than a field or two.
pub(super) fn object_schema(
    props: serde_json::Value,
    required: &[&str],
) -> Arc<serde_json::Map<String, serde_json::Value>> {
    let mut map = serde_json::Map::new();
    map.insert("type".into(), serde_json::json!("object"));
    map.insert("properties".into(), props);
    if !required.is_empty() {
        map.insert("required".into(), serde_json::json!(required));
    }
    map.insert("additionalProperties".into(), serde_json::Value::Bool(false));
    Arc::new(map)
}

pub(super) fn empty_object_schema() -> Arc<serde_json::Map<String, serde_json::Value>> {
    let mut map = serde_json::Map::new();
    map.insert("type".into(), serde_json::Value::String("object".into()));
    map.insert("properties".into(), serde_json::Value::Object(serde_json::Map::new()));
    map.insert("additionalProperties".into(), serde_json::Value::Bool(false));
    Arc::new(map)
}

/// Return a tool result with `is_error: true` and a human-readable
/// message. Uses `CallToolResult::error` so the struct's `#[non_exhaustive]`
/// guard is respected — direct construction is rejected by the compiler.
///
/// Returns the `CallToolResponse` envelope rmcp 3 wraps every tool result
/// in. Converting HERE rather than at each handler is what keeps the
/// three servers' `call_tool` bodies free of the distinction: they are
/// only ever `Complete`, never a task handle or an input request.
pub(super) fn tool_error(msg: impl Into<String>) -> CallToolResponse {
    CallToolResult::error(vec![ContentBlock::text(msg)]).into()
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
pub(super) fn structured_with_text(summary: impl Into<String>, value: serde_json::Value) -> CallToolResponse {
    let mut result = CallToolResult::structured(value);
    result.content = vec![ContentBlock::text(summary)];
    result.into()
}

pub(super) fn require_string<'a>(
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
pub(super) fn optional_string(
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

pub(super) fn optional_bool(
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

pub(super) fn optional_u64(
    args: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Option<u64>, rmcp::ErrorData> {
    match args.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => value.as_u64().map(Some).ok_or_else(|| {
            rmcp::ErrorData::invalid_params(format!("argument `{key}` must be a non-negative integer"), None)
        }),
    }
}

pub(super) fn optional_usize(
    args: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Option<usize>, rmcp::ErrorData> {
    Ok(optional_u64(args, key)?.map(|n| n as usize))
}

pub(super) fn optional_string_array(
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

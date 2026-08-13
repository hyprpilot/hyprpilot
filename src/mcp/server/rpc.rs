//! Shared JSON-RPC plumbing for the three in-tree MCP servers.
//!
//! Schema builders, result wrappers, and argument decoders every
//! `ServerHandler` needs. These lived in `serve.rs` — the skills
//! server — for the historical reason that it was the first server
//! written; five of them had no caller there at all.

use std::borrow::Cow;
use std::sync::Arc;

use rmcp::model::{CacheScope, CallToolResponse, CallToolResult, ContentBlock, ProtocolVersion};
use rmcp::service::RoleServer;
use rmcp::ServerHandler;

/// The protocol versions every in-tree server negotiates.
///
/// A DECLARATION, not `KNOWN_VERSIONS` by default. rmcp echoes back
/// whatever the client asks within the supported set, so inheriting the
/// default would let an SDK upgrade silently widen what we speak.
///
/// One set for all three servers. `2026-07-28` used to belong to the
/// harness alone, on the reasoning that Tasks shipped alongside it and
/// the other two gained nothing from `resultType` on every result
/// (SEP-2322) or `ping` answering `-32601` (SEP-2260). That split was
/// wrong in the direction that matters: the revision's requirements land
/// on `tools/list`, which every server serves, so excluding it from two
/// servers only hid the work rather than avoiding it. See
/// [`RESULT_TTL_MS`] for the requirement itself.
pub(super) fn supported_protocol_versions() -> Cow<'static, [ProtocolVersion]> {
    Cow::Borrowed(&[
        ProtocolVersion::V_2024_11_05,
        ProtocolVersion::V_2025_03_26,
        ProtocolVersion::V_2025_06_18,
        ProtocolVersion::V_2025_11_25,
        ProtocolVersion::V_2026_07_28,
    ])
}

/// Freshness hint stamped on every cacheable result (SEP-2549).
///
/// **`2026-07-28` makes `ttlMs` and `cacheScope` REQUIRED**, not
/// optional: `ListToolsResult extends PaginatedResult, CacheableResult`,
/// and `CacheableResult` declares both without `?`. rmcp models them as
/// `Option` for backward compatibility and defaults them to `None`, so a
/// server that just calls `with_all_items` emits neither and a client
/// validating the revision it negotiated rejects the listing outright:
///
/// ```text
/// Invalid result for tools/list:
///   ttlMs:      expected number, received undefined
///   cacheScope: expected one of "public" | "private"
/// ```
///
/// That is not a partial failure — the session reports `connected` and
/// then has NO TOOLS AT ALL, because the listing is the door. Claude Code
/// 2.2.x negotiates `2026-07-28` and hit exactly this.
///
/// `0` because nothing we serve is safely cacheable for a duration: the
/// skills catalogue changes on `reload`, and the profile list changes
/// whenever the captain edits config. Emitting them at older revisions
/// is harmless — the spec's `Result` is an open map, so an extra key is
/// permitted everywhere.
///
/// Stamp them on EVERY list and read result, on every server. A result
/// that forgets is invisible until a client upgrades.
pub(super) const RESULT_TTL_MS: u64 = 0;

/// Companion to [`RESULT_TTL_MS`]. `Private` is the conservative choice
/// and matches what the reference SDKs default to: these results are
/// scoped to one captain's config and one process's catalogue, so no
/// shared intermediary should serve them to anyone else.
pub(super) const RESULT_CACHE_SCOPE: CacheScope = CacheScope::Private;

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

#[cfg(test)]
mod tests {
    use super::*;

    /// The set is a declaration, not an inheritance. Pin every member so
    /// an SDK upgrade that adds a revision has to break this test rather
    /// than silently widen what our servers speak.
    #[test]
    fn the_negotiable_set_is_declared_not_inherited() {
        let set = supported_protocol_versions();
        for expected in [
            ProtocolVersion::V_2024_11_05,
            ProtocolVersion::V_2025_03_26,
            ProtocolVersion::V_2025_06_18,
            ProtocolVersion::V_2025_11_25,
            ProtocolVersion::V_2026_07_28,
        ] {
            assert!(
                supported_contains(&set, &expected),
                "dropping {expected:?} would cut off a client that negotiates it — codex is on 2025-06-18"
            );
        }
        assert_eq!(
            set.len(),
            ProtocolVersion::KNOWN_VERSIONS.len(),
            "a revision rmcp added is not automatically one we speak — add it here deliberately, \
             and only once every cacheable result carries what that revision requires"
        );
    }

    /// One set, all three servers. The split that gave the harness its
    /// own was wrong in the direction that matters: `2026-07-28`'s
    /// requirements land on `tools/list`, which every server serves, so
    /// excluding two of them hid the work instead of avoiding it.
    #[test]
    fn every_server_negotiates_the_same_set() {
        assert!(
            supported_contains(&supported_protocol_versions(), &ProtocolVersion::V_2026_07_28),
            "a client on the current revision must not be silently downgraded"
        );
    }

    /// The fields `2026-07-28` makes REQUIRED on a cacheable result.
    /// Omitting them is not a partial failure — a validating client
    /// rejects `tools/list` and the session comes up with no tools at
    /// all. `0` / `private` because nothing we serve is safely cacheable
    /// for a duration or across users.
    #[test]
    fn cacheable_results_are_stamped_conservatively() {
        assert_eq!(
            RESULT_TTL_MS, 0,
            "a non-zero ttl would let a client serve a stale catalogue"
        );
        assert_eq!(RESULT_CACHE_SCOPE, CacheScope::Private);
    }

    fn supported_contains(set: &[ProtocolVersion], want: &ProtocolVersion) -> bool {
        set.iter().any(|v| v == want)
    }
}

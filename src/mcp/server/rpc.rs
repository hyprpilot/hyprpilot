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
/// **Effectively "cache until I say otherwise."** 24 hours is longer
/// than any sidecar lives — the vendor spawns one per session and it
/// dies with that session — so a client that honours this never has to
/// re-fetch on a timer, and every real invalidation arrives as a
/// notification instead.
///
/// That only works because the invalidation is real. Every mutable
/// surface pairs the ttl with a signal: `reload` diffs the catalogue and
/// fires `resources/list_changed` for membership plus
/// `resources/updated` per changed skill, and a harness turn ending
/// fires `resources/updated` for its session. `tools/list` needs no
/// signal because the tool set is compiled in and cannot change while
/// the process lives.
///
/// Delivery is two channels, picked per notification by
/// [`Subscriptions`]: the `subscriptions/listen` stream when the client
/// opened one, a raw broadcast when it did not. Both reach every client
/// that can act on them, so the ttl does not depend on the client having
/// subscribed.
///
/// **The rule this creates:** a new mutable surface must either fire a
/// notification or lower the ttl for itself. Adding one that does
/// neither is invisible until a client caches it for a day.
///
/// Stamp both fields on EVERY list and read result, on every server. A
/// result that forgets is invisible until a client upgrades.
pub(super) const RESULT_TTL_MS: u64 = 24 * 60 * 60 * 1000;

/// Companion to [`RESULT_TTL_MS`]. `Private` is the conservative choice
/// and matches what the reference SDKs default to: these results are
/// scoped to one captain's config and one process's catalogue, so no
/// shared intermediary should serve them to anyone else.
pub(super) const RESULT_CACHE_SCOPE: CacheScope = CacheScope::Private;

/// The live `subscriptions/listen` stream, when the client opened one.
///
/// A stdio sidecar serves exactly ONE client, so at most one stream is
/// open at a time — which is what lets a notification choose its channel
/// instead of fanning out to a set.
///
/// This exists because `Peer::notify_*` is an unconditional pipe send.
/// It reaches every client regardless of what they subscribed to, and it
/// carries no `io.modelcontextprotocol/subscriptionId` — so a conforming
/// `2026-07-28` client, which correlates stream notifications by that
/// id, never sees it on the stream it opened. Only
/// [`rmcp::service::SubscriptionSink`] filters against the accepted
/// filter and stamps the id.
#[derive(Clone, Default)]
pub(super) struct Subscriptions(Arc<tokio::sync::RwLock<Option<rmcp::service::SubscriptionSink>>>);

impl Subscriptions {
    /// Hold the stream's sink for as long as the stream lives.
    ///
    /// rmcp has already acknowledged the subscription by the time this
    /// runs, so the only job here is to keep the sink reachable and to
    /// let go when the client cancels — returning `Ok(())` is what marks
    /// a graceful teardown.
    pub(super) async fn run(&self, context: rmcp::service::SubscriptionContext) {
        self.0.write().await.replace(context.sink().clone());
        context.cancelled().await;
        self.0.write().await.take();
    }

    /// Deliver `notifications/resources/updated`, preferring the stream.
    pub(super) async fn resource_updated(&self, peer: &rmcp::service::Peer<RoleServer>, uri: String) {
        if let Some(sink) = self.0.read().await.as_ref() {
            match sink.notify_resource_updated(uri.clone()).await {
                Ok(()) => return,
                // Not an error: the client subscribed to other URIs, so
                // this one is genuinely none of its business. Falling
                // through to a broadcast would defeat the filter.
                Err(rmcp::service::SubscriptionSendError::NotificationNotAccepted(_)) => return,
                Err(err) => {
                    tracing::debug!(%err, %uri, "mcp::server: subscription send failed — falling back to broadcast");
                }
            }
        }
        // No stream, or the stream broke. This is the only channel a
        // client on an older revision has, and the one they have always
        // had.
        let param = rmcp::model::ResourceUpdatedNotificationParam::new(uri.clone());
        if let Err(err) = peer.notify_resource_updated(param).await {
            tracing::debug!(%err, %uri, "mcp::server: resource-updated notification failed");
        }
    }

    /// Deliver `notifications/resources/list_changed`, preferring the
    /// stream. Same channel choice as [`Self::resource_updated`].
    pub(super) async fn resource_list_changed(&self, peer: &rmcp::service::Peer<RoleServer>) {
        if let Some(sink) = self.0.read().await.as_ref() {
            match sink.notify_resource_list_changed().await {
                Ok(()) => return,
                Err(rmcp::service::SubscriptionSendError::NotificationNotAccepted(_)) => return,
                Err(err) => {
                    tracing::debug!(%err, "mcp::server: subscription send failed — falling back to broadcast");
                }
            }
        }
        if let Err(err) = peer.notify_resource_list_changed().await {
            tracing::debug!(%err, "mcp::server: resource list-changed notification failed");
        }
    }
}

/// Accept a `subscriptions/listen` opt-in for the two categories the
/// in-tree servers actually emit.
///
/// `resource_subscriptions` is filtered to URIs this server can notify:
/// the acknowledgment is the client's contract for what it will receive,
/// so accepting a scheme we never fire leaves it waiting forever. The
/// SDK then intersects the result with the request and the advertised
/// capabilities, which is what correctly refuses `toolsListChanged` —
/// the tool set is compiled in and cannot change.
pub(super) fn accept_resource_subscriptions(
    requested: &rmcp::model::SubscriptionFilter,
    known_uri: impl Fn(&str) -> bool,
) -> Option<rmcp::model::SubscriptionFilter> {
    let mut accepted = rmcp::model::SubscriptionFilter::new();
    accepted.resources_list_changed = requested.resources_list_changed;
    accepted.resource_subscriptions = requested
        .resource_subscriptions
        .as_ref()
        .map(|uris| uris.iter().filter(|uri| known_uri(uri)).cloned().collect());

    Some(accepted)
}

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
    /// all.
    ///
    /// The ttl is deliberately longer than any sidecar lives, which is
    /// only honest because every mutable surface fires an invalidation
    /// notification. Lowering it back toward zero would mean the
    /// invalidation stopped being trustworthy — a real change, not a
    /// tuning tweak, so it breaks here first.
    #[test]
    fn cacheable_results_cache_until_invalidated() {
        // Asserted as an exact value rather than a bound: the point is
        // that changing it is a deliberate act with consequences for
        // every cached surface, not that it clears some threshold.
        assert_eq!(
            RESULT_TTL_MS,
            24 * 60 * 60 * 1000,
            "the ttl must outlive a sidecar, or clients re-fetch on a timer we told them not to need"
        );
        assert_eq!(
            RESULT_CACHE_SCOPE,
            CacheScope::Private,
            "these results are scoped to one captain's config — no shared intermediary may serve them on"
        );
    }

    /// The acknowledgment is the client's CONTRACT for what it will
    /// receive, so accepting a URI this server never fires for leaves
    /// the client waiting on it forever.
    #[test]
    fn the_filter_accepts_only_uris_this_server_can_notify() {
        let mut requested = rmcp::model::SubscriptionFilter::new();
        requested.resources_list_changed = Some(true);
        requested.resource_subscriptions = Some(vec![
            "hyprpilot://sessions/abc".into(),
            "file:///etc/passwd".into(),
            "hyprpilot://skills/git-commit".into(),
        ]);

        let accepted = accept_resource_subscriptions(&requested, |uri| uri.starts_with("hyprpilot://sessions/"))
            .expect("subscriptions are supported");

        assert_eq!(
            accepted.resource_subscriptions,
            Some(vec!["hyprpilot://sessions/abc".to_string()]),
            "a foreign scheme must be dropped, not acknowledged"
        );
        assert_eq!(accepted.resources_list_changed, Some(true));
    }

    /// `toolsListChanged` is never ours to give — the tool set is
    /// compiled in. We simply never set it; the SDK's own intersection
    /// against the advertised capabilities does the rest.
    #[test]
    fn the_filter_never_claims_a_category_we_cannot_emit() {
        let mut requested = rmcp::model::SubscriptionFilter::new();
        requested.tools_list_changed = Some(true);
        requested.prompts_list_changed = Some(true);

        let accepted = accept_resource_subscriptions(&requested, |_| true).expect("subscriptions are supported");

        assert_eq!(accepted.tools_list_changed, None);
        assert_eq!(accepted.prompts_list_changed, None);
    }

    fn supported_contains(set: &[ProtocolVersion], want: &ProtocolVersion) -> bool {
        set.iter().any(|v| v == want)
    }
}

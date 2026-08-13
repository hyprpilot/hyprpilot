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
/// fires `resources/list_changed` for ANY change — not only membership,
/// because a client that cannot subscribe has no other signal — plus
/// `resources/updated` per changed URI as the precision for
/// subscribers. A harness turn starting or ending fires the same pair
/// for its session. `tools/list` needs no signal because the tool set is
/// compiled in and cannot change while the process lives.
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

/// Anything held by id in a [`Registry`]. Exists so the add/remove
/// bookkeeping — the part that had the bug — is testable without a live
/// MCP service to mint a real sink from.
pub(super) trait Keyed {
    fn key(&self) -> &rmcp::model::RequestId;
}

impl Keyed for rmcp::service::SubscriptionSink {
    fn key(&self) -> &rmcp::model::RequestId {
        self.id()
    }
}

/// An id-keyed set of live entries.
///
/// `remove` matches on the key rather than clearing, which is the whole
/// point: `listen(A)`, `listen(B)`, `cancel(A)` is legal and is how a
/// client changes its filter, so A's teardown must not take B with it.
#[derive(Debug)]
pub(super) struct Registry<T>(Arc<tokio::sync::RwLock<Vec<T>>>);

impl<T> Clone for Registry<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T> Default for Registry<T> {
    fn default() -> Self {
        Self(Arc::new(tokio::sync::RwLock::new(Vec::new())))
    }
}

impl<T: Keyed + Clone> Registry<T> {
    async fn add(&self, entry: T) {
        self.0.write().await.push(entry);
    }

    async fn remove(&self, key: &rmcp::model::RequestId) {
        self.0.write().await.retain(|open| open.key() != key);
    }

    /// Snapshot, so sends happen without the lock held — a send awaits
    /// the transport, and holding a read guard across that would stall
    /// every teardown behind it.
    async fn snapshot(&self) -> Vec<T> {
        self.0.read().await.clone()
    }
}

/// What one stream did with a notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StreamOutcome {
    /// Sent on that stream, filtered and tagged.
    Delivered,
    /// Refused by the accepted filter. The client declared what it
    /// wants; not receiving the rest is the declaration working.
    Declined,
    /// Cancelled or transport-dead. Says nothing about what the client
    /// wants, so it must not stand in for an answer.
    Broken,
}

/// Whether the raw broadcast should still run.
///
/// The rule the wire-visible bug lived in: a broadcast is the fallback
/// for a client with NO usable stream, never a second delivery on top of
/// one. So it runs when there are no streams at all, or when every
/// stream is broken — and it must NOT run merely because a stream
/// declined, or the filter would be pointless.
pub(super) fn needs_broadcast(outcomes: &[StreamOutcome]) -> bool {
    outcomes.iter().all(|outcome| *outcome == StreamOutcome::Broken)
}

/// Every open `subscriptions/listen` stream.
///
/// A LIST, not a slot. rmcp runs each request in its own task, so two
/// `listen` calls are legal and the natural filter-change sequence is
/// listen(new) then cancel(old) — with a single slot, the cancelling
/// stream tears down the surviving stream's sink and every later
/// notification silently degrades to an untagged broadcast.
///
/// This exists because `Peer::notify_*` is an unconditional pipe send.
/// It ignores the accepted filter and carries no
/// `io.modelcontextprotocol/subscriptionId` — so a conforming
/// `2026-07-28` client, which correlates stream notifications by that
/// id, never sees it on the stream it opened. Only
/// [`rmcp::service::SubscriptionSink`] filters and stamps.
#[derive(Clone, Default)]
pub(super) struct Subscriptions(Registry<rmcp::service::SubscriptionSink>);

impl Subscriptions {
    /// Hold this stream's sink for as long as the stream lives.
    ///
    /// rmcp has already acknowledged the subscription by the time this
    /// runs, so the job is to keep the sink reachable and to remove
    /// exactly THIS one on teardown — keyed by request id, because a
    /// sibling stream may have registered in between.
    pub(super) async fn run(&self, context: rmcp::service::SubscriptionContext) {
        let sink = context.sink().clone();
        let id = sink.id().clone();
        self.0.add(sink).await;
        context.cancelled().await;
        self.0.remove(&id).await;
    }

    /// Snapshot of the open sinks.
    ///
    /// Cloned so the sends happen without the lock held — a sink send
    /// awaits the transport, and holding a read guard across that would
    /// stall every `run()` teardown behind it.
    async fn streams(&self) -> Vec<rmcp::service::SubscriptionSink> {
        self.0.snapshot().await
    }

    /// Deliver `notifications/resources/updated`.
    ///
    /// Offered to EVERY open stream: a refusal by one must not silence
    /// another. A refusal is not a failure — the client declared what it
    /// wants, and broadcasting past its filter would defeat the
    /// declaration — so it still counts as handled. The broadcast is the
    /// fallback only when no stream exists, which is the case for every
    /// client on an older revision.
    pub(super) async fn resource_updated(&self, peer: &rmcp::service::Peer<RoleServer>, uri: String) {
        let mut outcomes = Vec::new();
        for sink in &self.streams().await {
            outcomes.push(match sink.notify_resource_updated(uri.clone()).await {
                Ok(()) => StreamOutcome::Delivered,
                Err(rmcp::service::SubscriptionSendError::NotificationNotAccepted(_)) => StreamOutcome::Declined,
                Err(err) => {
                    tracing::debug!(%err, %uri, "mcp::server: subscription send failed");
                    StreamOutcome::Broken
                }
            });
        }
        if !needs_broadcast(&outcomes) {
            return;
        }
        let param = rmcp::model::ResourceUpdatedNotificationParam::new(uri.clone());
        if let Err(err) = peer.notify_resource_updated(param).await {
            tracing::debug!(%err, %uri, "mcp::server: resource-updated notification failed");
        }
    }

    /// Deliver `resources/updated` for several URIs at once.
    ///
    /// One event can invalidate more than one view of the same thing —
    /// a turn ending changes a session's status, its answer and its
    /// transcript — and a subscriber may hold any subset of them.
    pub(super) async fn resources_updated(&self, peer: &rmcp::service::Peer<RoleServer>, uris: Vec<String>) {
        for uri in uris {
            self.resource_updated(peer, uri).await;
        }
    }

    /// Deliver `notifications/resources/list_changed`. Same channel
    /// choice as [`Self::resource_updated`].
    pub(super) async fn resource_list_changed(&self, peer: &rmcp::service::Peer<RoleServer>) {
        let mut outcomes = Vec::new();
        for sink in &self.streams().await {
            outcomes.push(match sink.notify_resource_list_changed().await {
                Ok(()) => StreamOutcome::Delivered,
                Err(rmcp::service::SubscriptionSendError::NotificationNotAccepted(_)) => StreamOutcome::Declined,
                Err(err) => {
                    tracing::debug!(%err, "mcp::server: subscription send failed");
                    StreamOutcome::Broken
                }
            });
        }
        if !needs_broadcast(&outcomes) {
            return;
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

/// Answer `initialize`, recording the NEGOTIATED protocol version as the
/// peer's — not the one it asked for.
///
/// rmcp's in-loop default records the REQUESTED version and never
/// revisits it, while the pre-loop handshake we no longer use overwrote
/// it with the negotiated one. Left alone, a client asking for a
/// revision outside [`supported_protocol_versions`] is told we
/// negotiated down and then served that revision's result shapes
/// anyway — `resultType` on a session that agreed `2025-11-25`. That is
/// the same failure the `ttlMs` stamp exists for: a client validating
/// the revision it was handed rejects the payload, and the whole
/// listing goes with it.
///
/// The negotiation rule mirrors rmcp's `negotiate_protocol_version`
/// (`pub(crate)`, so it cannot be called): echo the requested version
/// when we support it, else fall back to our own.
pub(super) fn initialize_negotiated<H: ServerHandler>(
    handler: &H,
    request: rmcp::model::InitializeRequestParams,
    context: &rmcp::service::RequestContext<RoleServer>,
) -> rmcp::model::InitializeResult {
    let mut info = handler.get_info();
    if handler
        .supported_protocol_versions()
        .contains(&request.protocol_version)
    {
        info.protocol_version = request.protocol_version.clone();
    }

    let mut peer_info = request;
    peer_info.protocol_version = info.protocol_version.clone();
    context.peer.set_peer_info(peer_info);
    info
}

/// Start serving from the connection's FIRST byte, with no handshake
/// phase of our own.
///
/// `ServiceExt::serve` runs rmcp's pre-loop handshake, which handles a
/// non-`initialize` opener INLINE — `service.handle_request(..).await`
/// completes before the serve loop is spawned. A long-lived opener
/// therefore deadlocks the process: `subscriptions/listen` acknowledges
/// through `Peer::send_notification`, which awaits a oneshot only the
/// loop can fire, so the ack waits on a loop that is waiting on the ack.
/// Nothing is read or written again, ever.
///
/// That is not a hypothetical ordering. Claude Code's v2 MCP runtime
/// probes `server/discover` on a DISPOSABLE second process, then opens
/// the real transport with `subscriptions/listen` as its first request —
/// so the deadlock is the normal path for a server that implements
/// subscriptions, and only for those. It reports `connected` (the throwaway
/// probe succeeded) and then times out fetching tools.
///
/// `serve_directly` spawns the loop from byte zero, so every request —
/// opener included — runs in its own task. `initialize` still negotiates
/// against `supported_protocol_versions` and still records `peer_info`,
/// but the in-loop default records the version the client ASKED for
/// rather than the negotiated one — see `initialize_negotiated`, which
/// every server overrides `initialize` to use.
pub(super) fn serve_from_first_byte<H, T, E, A>(
    handler: H,
    transport: T,
) -> rmcp::service::RunningService<RoleServer, H>
where
    H: ServerHandler,
    T: rmcp::transport::IntoTransport<RoleServer, E, A>,
    E: std::error::Error + Send + Sync + 'static,
{
    rmcp::service::serve_directly(handler, transport, None)
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
            reason = &mut transport => {
            // A transport ERROR and a clean EOF are the same silence to a
            // supervisor otherwise, and the sidecar's exit code cannot
            // tell them apart — `serve_directly` is infallible, so this
            // is the only place either is observable.
            tracing::debug!(?reason, "mcp: transport closed");
        }
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

    /// The rule the FIRST rejection was about, pinned.
    ///
    /// Reverting to an unconditional `Peer::notify_*` broadcast — the
    /// original bug — makes this the wrong answer for a client with a
    /// working stream, so a revert fails here instead of only failing a
    /// manual transcript.
    #[test]
    fn broadcast_is_the_fallback_for_no_usable_stream_and_nothing_else() {
        use StreamOutcome::{Broken, Declined, Delivered};

        assert!(needs_broadcast(&[]), "no stream at all is the legacy client");
        assert!(needs_broadcast(&[Broken]), "a dead stream must not swallow the event");
        assert!(
            needs_broadcast(&[Broken, Broken]),
            "every stream dead is still no usable stream"
        );

        assert!(
            !needs_broadcast(&[Delivered]),
            "broadcasting on top of a delivery double-sends it"
        );
        assert!(
            !needs_broadcast(&[Declined]),
            "a decline is the filter working — broadcasting past it makes the filter pointless"
        );
        assert!(
            !needs_broadcast(&[Broken, Delivered]),
            "one live stream is enough; the broken sibling does not re-open the broadcast"
        );
        assert!(!needs_broadcast(&[Declined, Broken]));
    }

    /// A fake entry so the registry's bookkeeping — the part that had
    /// the bug — is testable without a live service to mint a sink from.
    #[derive(Clone, Debug, PartialEq)]
    struct Entry(rmcp::model::RequestId);

    impl Keyed for Entry {
        fn key(&self) -> &rmcp::model::RequestId {
            &self.0
        }
    }

    fn entry(id: i64) -> Entry {
        Entry(rmcp::model::RequestId::Number(id))
    }

    /// The bug this type was rebuilt for.
    ///
    /// rmcp runs each request in its own task, so `listen(A)`,
    /// `listen(B)`, `cancel(A)` is legal — and is how a client changes
    /// its filter. The previous single-slot version had A's teardown
    /// clear B's sink, after which every notification silently degraded
    /// to an untagged broadcast. Reverting `remove` to a clear fails
    /// here.
    #[tokio::test]
    async fn cancelling_one_stream_leaves_its_sibling_registered() {
        let registry = Registry::default();
        registry.add(entry(10)).await;
        registry.add(entry(11)).await;

        registry.remove(&rmcp::model::RequestId::Number(10)).await;

        assert_eq!(
            registry.snapshot().await,
            vec![entry(11)],
            "the surviving stream must still be reachable, or its client goes silent"
        );
    }

    /// Teardown must be idempotent and must not touch strangers.
    ///
    /// A `run` future dropped without cancellation produces ZERO
    /// removals — the entry leaks, harmlessly, because the sink's
    /// `DropGuard` closes it and a closed sink falls through to the
    /// broadcast. What idempotency actually guards is a double
    /// cancellation racing the same id.
    #[tokio::test]
    async fn removing_an_unknown_stream_is_a_no_op() {
        let registry = Registry::default();
        registry.add(entry(1)).await;

        registry.remove(&rmcp::model::RequestId::Number(99)).await;
        registry.remove(&rmcp::model::RequestId::Number(1)).await;
        registry.remove(&rmcp::model::RequestId::Number(1)).await;

        assert!(registry.snapshot().await.is_empty());
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

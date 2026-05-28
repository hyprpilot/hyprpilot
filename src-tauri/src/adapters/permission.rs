//! Generic permission-prompt vocabulary + `PermissionController` trait.
//!
//! The decision pipeline is intentionally minimal — hyprpilot stays
//! transparent to the agent's permission model and only intercepts what
//! captains explicitly opt into via static MCP-side config:
//!
//! 1. **Per-server hyprpilot extension globs** — looked up via the
//!    tool→server attribution map populated at `session/new` time.
//!    Reject beats accept.
//! 2. **Default**: `AskUser` — bounces to the UI.
//!
//! There is no daemon-side runtime trust store. The captain's "always
//! allow / always deny" pick rides the wire as-is to ACP; the agent
//! itself owns whatever persistence it offers (claude-agent-acp writes
//! to `~/.claude/settings.json`, etc.). Hyprpilot does not shadow that.
//!
//! `register_pending` + `resolve` own the oneshot waiter map that
//! bridges the Tauri `permission_reply` command back to the awaiting
//! ACP handler.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, oneshot, Mutex};

use crate::adapters::instance::InstanceEvent;
use crate::mcp::MCPsRegistry;

/// Sentinel `option_id` used on the `PermissionResolved` event when
/// the resolution path is the 10-min `WAITER_TIMEOUT` rather than a
/// captain-supplied answer. See [`InstanceEvent::PermissionResolved`].
pub const PERMISSION_EXPIRED_OPTION_ID: &str = "__expired__";

/// How long an `AskUser` waiter stays live before the caller
/// abandons it and treats the outcome as `Cancelled`. Matches the
/// issue's 10-min target; a prompt left unanswered across a
/// compositor lock or a user's lunch break should not wedge the
/// ACP session forever. Enforced by `tokio::time::timeout` at the
/// `AcpClient::request_permission` call site; the controller itself
/// does not spawn a timer — that let a detached `sleep(WAITER_TIMEOUT)`
/// accumulate one future per resolved prompt.
pub const WAITER_TIMEOUT: Duration = Duration::from_secs(10 * 60);

const ALLOW_ONCE_KIND: &str = "allow_once";
const ALLOW_ALWAYS_KIND: &str = "allow_always";
const REJECT_ONCE_KIND: &str = "reject_once";

/// UI-facing projection of a permission option. Wire-normalised so
/// the webview doesn't need to speak any specific vendor's shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PermissionOptionView {
    pub option_id: String,
    pub name: String,
    /// Wire-normalised snake-case string from the agent (`"allow_once"`,
    /// `"allow_always"`, `"reject_once"`, `"reject_always"` today;
    /// vendors are free to introduce new variants). Do not infer
    /// semantics from `name` / `option_id`: all supported ACP adapters
    /// mark the one-shot accept / reject actions with canonical
    /// `allow_once` / `reject_once` kinds, and unknown kinds must stay
    /// unbound rather than guessing.
    pub kind: String,
}

/// `true` for any kind whose wire string begins with `reject`
/// (today: `reject_once` / `reject_always`; future variants like
/// `reject_session` classify as reject). Used by external
/// classification consumers — `permissions/respond` decides whether
/// the captain's pick triggers the reject-feedback follow-up; the
/// picker functions below are stricter and match once-kind keybind
/// targets exactly.
#[must_use]
pub fn is_reject_kind(kind: &str) -> bool {
    kind.starts_with("reject")
}

/// Canonical wire ordering for permission options: `allow_always`
/// first, `allow_once` second (the default highlight + Ctrl+G
/// target), `reject_once` third (Ctrl+R target), then everything
/// else in its original order. Vendors ship options in different
/// orders (codex puts `Approve Once` first, claude puts
/// `Always Allow` first); daemon-side normalisation means every
/// frontend (Vue overlay, nvim plugin, ws remote) renders the same
/// button arrangement without each re-implementing the sort.
///
/// Classification is kind-only. Do not fall back to labels or ids:
/// a vendor option with an unknown kind is not safe to bind to the
/// default / keybind paths, even when its display label looks close.
#[must_use]
pub fn reorder_options(options: Vec<PermissionOptionView>) -> Vec<PermissionOptionView> {
    let mut allow_always: Option<PermissionOptionView> = None;
    let mut allow_once: Option<PermissionOptionView> = None;
    let mut reject_once: Option<PermissionOptionView> = None;
    let mut rest: Vec<PermissionOptionView> = Vec::with_capacity(options.len());

    for o in options {
        if allow_once.is_none() && o.kind == ALLOW_ONCE_KIND {
            allow_once = Some(o);
        } else if allow_always.is_none() && o.kind == ALLOW_ALWAYS_KIND {
            allow_always = Some(o);
        } else if reject_once.is_none() && o.kind == REJECT_ONCE_KIND {
            reject_once = Some(o);
        } else {
            rest.push(o);
        }
    }

    let mut out: Vec<PermissionOptionView> = Vec::with_capacity(
        usize::from(allow_always.is_some())
            + usize::from(allow_once.is_some())
            + usize::from(reject_once.is_some())
            + rest.len(),
    );

    if let Some(o) = allow_always {
        out.push(o);
    }

    if let Some(o) = allow_once {
        out.push(o);
    }

    if let Some(o) = reject_once {
        out.push(o);
    }
    out.extend(rest);
    out
}

/// Tool reference behind a permission request. `name` / `title` /
/// `raw_args` drive display; `tool_kind` is the single typed
/// classification used for styling and MCP allowlist matching.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallRef {
    /// Display / adapter key. Adapters populate with the most stable
    /// identifier their wire exposes (for ACP: the tool's title,
    /// falling back to the ToolKind wire name).
    pub name: String,
    pub tool_kind: crate::tools::ToolKind,
    pub title: Option<String>,
    /// Short human-readable summary of args the UI displays below
    /// the tool name (e.g. the `command` for a Bash call). Opaque to
    /// the allowlist matcher.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_args: Option<String>,
    /// Full structured `tool_call.rawInput` JSON object — pass-through
    /// of the ACP wire field. Carries fields like `plan` for the
    /// claude-code `ExitPlanMode` permission flow so the UI can render
    /// a markdown-bodied plan modal instead of the collapsed string in
    /// `raw_args`. Opaque to the allowlist matcher.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_input: Option<serde_json::Value>,
    /// Raw `tool_call.content[]` array — pass-through of the ACP wire
    /// shape (`{ type: 'content' | 'diff' | 'terminal', ... }`).
    /// Populated for permissions whose markdown body lives on the
    /// tool-call's content array (claude-code's `Switch mode` flow
    /// ships the plan body here, not on `raw_input.plan`). UI walks
    /// the array directly — no server-side joining.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<serde_json::Value>,
}

/// Everything the controller needs to make a decision and route a
/// later reply. `request_id` is the correlation key the reply
/// command sends back; `instance_id` tags the snapshot returned by
/// `permissions/pending` so callers can address a specific live
/// instance.
#[derive(Debug, Clone)]
pub struct PermissionRequest {
    pub instance_id: Option<String>,
    pub request_id: String,
    pub tool_call: ToolCallRef,
    pub options: Vec<PermissionOptionView>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PermissionOptionTargets {
    pub default_option_id: Option<String>,
    pub allow_option_id: Option<String>,
    pub reject_option_id: Option<String>,
}

/// Decision chain outcome. `Allow` / `Deny` map directly to ACP at
/// the call site; `AskUser` means the caller must emit a
/// `acp:permission-request` event + await the controller-managed
/// oneshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny,
    AskUser,
}

/// What the UI (or the timeout) eventually decides. Mirrors the
/// ACP `RequestPermissionOutcome` wire shape one-for-one:
/// `Selected(option_id)` or `Cancelled`. `Cancelled` is only
/// constructed inside tests today (via `controller.resolve(_, ...)`);
/// production paths build `Selected(option_id)` from the captain's
/// pick. Narrow allow keeps the variant available for the test-only
/// path without forcing a `#[cfg(test)]` split.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionOutcome {
    Selected(String),
    Cancelled,
}

/// Snapshot of a pending permission request returned by
/// `permissions/pending`. `args` carries `tool_call.raw_args` (or
/// `tool_call.title` when no raw args were available) verbatim — the
/// UI decides how to render / truncate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRequestSnapshot {
    pub request_id: String,
    pub instance_id: Option<String>,
    pub tool: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<String>,
    pub options: Vec<PermissionOptionView>,
    /// Pre-selected option id — captains hitting `Enter` on the
    /// modal commit this. Mirror of `allow_option_id` today.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_option_id: Option<String>,
    /// Which option the captain's `allow` keybind (Ctrl+G by
    /// default) commits to. Same as the matching field on
    /// `InstanceEvent::PermissionRequest` — frontends read this
    /// directly so the keybind handler doesn't duplicate the
    /// allow-once matcher client-side.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_option_id: Option<String>,
    /// Which option the captain's `deny` keybind (Ctrl+R by
    /// default) commits to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reject_option_id: Option<String>,
}

/// Everything the decision pipeline needs at call time. `mcps`
/// provides the per-server hyprpilot-extension globs; when `None`
/// every decision falls through to `AskUser`.
///
/// Tool→server attribution is structured on `ToolCallRef::tool_kind`.
/// Adapters that receive legacy `mcp__<server>__<tool>` strings may
/// parse those at the adapter boundary, but the controller never
/// matches against the string form. Vendor-side native tools (Bash,
/// Read, …) carry `None` and skip the lookup entirely.
pub struct DecisionContext<'a> {
    pub mcps: Option<&'a MCPsRegistry>,
}

/// The decision + waiter surface. `decide` is synchronous (pure
/// lookups against the per-server MCP glob config). `register_pending`
/// + `resolve` own the oneshot map that bridges the Tauri
/// `permission_reply` command back to the awaiting ACP handler.
///
/// `resolve` and `options_for` are exercised by tests but no
/// production path calls them via `dyn PermissionController` today —
/// the adapter goes through `respond_with` (atomic). Narrow allow
/// keeps them visible as the documented test-touch surface.
#[allow(dead_code)]
#[async_trait]
pub trait PermissionController: Send + Sync + 'static {
    /// Run the per-server MCP glob lookup; `AskUser` for everything
    /// else. Reject beats accept inside the glob lane.
    fn decide(&self, req: &PermissionRequest, ctx: &DecisionContext<'_>) -> Decision;

    /// Register a pending prompt. Returns the receiver the caller
    /// awaits; wrap the receive in `tokio::time::timeout(WAITER_TIMEOUT,
    /// rx)` and call `forget` on elapsed so stale waiters don't pin
    /// the map.
    async fn register_pending(&self, req: PermissionRequest) -> oneshot::Receiver<PermissionOutcome>;

    /// Resolve a pending request by id. No-op when no waiter
    /// exists for `request_id` — the command handler never needs
    /// to know whether the waiter already timed out.
    async fn resolve(&self, request_id: &str, outcome: PermissionOutcome);

    /// Drop a pending request from the waiter map without signalling.
    /// Used by the call-site timeout path: once the caller has decided
    /// to abandon an `rx.await`, the map entry needs to go so a late
    /// `permission_reply` doesn't land on a zombie waiter.
    async fn forget(&self, request_id: &str);

    /// Lookup the preserved options vector for a pending request.
    /// The Tauri `permission_reply` command uses this to translate
    /// the UI's simple `allow` / `deny` strings into real ACP option
    /// ids. Returns `None` when the waiter has already been resolved
    /// or never existed.
    async fn options_for(&self, request_id: &str) -> Option<Vec<PermissionOptionView>>;

    /// Full snapshot for a single pending request — the same shape
    /// `list_pending` produces, but addressed by id. Used by
    /// `permissions/respond` (and the Tauri `permission_reply`
    /// command) to read the instance id + the picked option's
    /// `kind` BEFORE resolving the waiter, so the post-resolve
    /// feedback dispatch can route a synthetic follow-up turn to
    /// the right instance. `None` when the waiter has already
    /// resolved or never existed.
    async fn snapshot_for(&self, request_id: &str) -> Option<PermissionRequestSnapshot>;

    /// Atomic membership-check + option-validation + resolve under a
    /// single lock. Used by `permissions/respond` so the lookup ≠
    /// resolve race window collapses to zero.
    ///
    /// - `None` — no waiter for `request_id` (already resolved or
    ///   never registered).
    /// - `Some(false)` — waiter exists but `option_id` is not in its
    ///   stored options list; nothing fired.
    /// - `Some(true)` — waiter resolved with `Selected(option_id)`.
    async fn resolve_if_pending(&self, request_id: &str, option_id: &str) -> Option<bool>;

    /// Snapshot every currently-pending request as a
    /// `PermissionRequestSnapshot` vector. Powers `permissions/pending`.
    async fn list_pending(&self) -> Vec<PermissionRequestSnapshot>;
}

/// Default impl: in-memory waiter map only. Glob sets are compiled
/// per `decide` call (the per-server lists are tiny — a handful of
/// patterns each — so caching is premature; if it surfaces in a
/// profile, swap to a precompiled cache keyed by server name +
/// content-hash).
///
/// `events_tx` is wire-attached post-construction via
/// [`Self::attach_events_tx`]: the registry that owns the broadcast
/// channel is built by the adapter, but the controller is constructed
/// in the daemon BEFORE the adapter exists (so the adapter can be
/// handed an `Arc<dyn PermissionController>` at construction time).
/// `OnceLock` carries the post-construction wire-up without forcing
/// every test site through a sender-aware constructor.
#[derive(Debug, Default)]
pub struct DefaultPermissionController {
    waiters: Arc<Mutex<HashMap<String, PendingWaiter>>>,
    /// `None` in tests / standalone units; `Some(tx)` in production
    /// after the daemon's adapter is wired up. When `None`, resolve /
    /// forget paths skip the broadcast — no `PermissionResolved` event
    /// surfaces, which matches the test setup that never subscribes.
    events_tx: OnceLock<broadcast::Sender<InstanceEvent>>,
}

#[derive(Debug)]
struct PendingWaiter {
    tx: oneshot::Sender<PermissionOutcome>,
    /// Original options list — preserved so the Tauri
    /// `permission_reply` command can resolve synthetic `"allow"` /
    /// `"deny"` shortcuts against real ACP option ids.
    options: Vec<PermissionOptionView>,
    /// Snapshot of the tool + instance identity at registration time.
    /// `permissions/pending` reads from this so the wire shape is
    /// fully derivable without reaching back into the originating
    /// ACP request.
    snapshot: PermissionRequestSnapshot,
}

impl DefaultPermissionController {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Wire the registry's event broadcast in. Idempotent — calling
    /// twice is a no-op (the `OnceLock` rejects the second set).
    /// Production wiring lives in the daemon: build the controller →
    /// build the adapter (which owns the registry) → call this with
    /// `adapter.events_tx()`. Tests skip this and the resolve / forget
    /// paths silently no-op the broadcast.
    pub fn attach_events_tx(&self, tx: broadcast::Sender<InstanceEvent>) {
        let _ = self.events_tx.set(tx);
    }

    /// Emit a `PermissionResolved` event when the broadcast is wired
    /// up. Best-effort — slow subscribers drop messages on Lagged just
    /// like every other registry event.
    fn emit_resolved(&self, instance_id: Option<&str>, request_id: &str, option_id: &str) {
        let Some(tx) = self.events_tx.get() else {
            return;
        };
        let event = InstanceEvent::PermissionResolved {
            instance_id: instance_id.unwrap_or_default().to_string(),
            request_id: request_id.to_string(),
            option_id: option_id.to_string(),
        };
        let _ = tx.send(event);
    }
}

#[async_trait]
impl PermissionController for DefaultPermissionController {
    fn decide(&self, req: &PermissionRequest, ctx: &DecisionContext<'_>) -> Decision {
        let tool = req.tool_call.name.as_str();

        // Per-server hyprpilot extension globs. The adapter boundary
        // attributes MCP calls to `{ server, leaf }`; the controller
        // matches the SERVER-RELATIVE leaf against that server's
        // accept / reject globs. Captains write `read_*` / `delete_*`
        // under the server block; the server namespace is implicit.
        // Reject beats accept. Vendor-native tools skip this lane
        // entirely.
        if let Some(registry) = ctx.mcps {
            if let crate::tools::ToolKind::Mcp { server, tool: leaf } = &req.tool_call.tool_kind {
                // Cached globs — built once at MCPsRegistry construction.
                // Reject hits short-circuit before the accept set is even
                // examined; both are precompiled so neither path allocates.
                if let Some((reject_set, accept_set)) = registry.globs_for(server) {
                    if reject_set.as_ref().is_some_and(|gs| gs.is_match(leaf)) {
                        tracing::debug!(
                            request_id = %req.request_id,
                            tool,
                            server = %server,
                            leaf,
                            "permission::decide: per-server reject glob hit"
                        );
                        return Decision::Deny;
                    }
                    if accept_set.as_ref().is_some_and(|gs| gs.is_match(leaf)) {
                        tracing::debug!(
                            request_id = %req.request_id,
                            tool,
                            server = %server,
                            leaf,
                            "permission::decide: per-server accept glob hit"
                        );
                        return Decision::Allow;
                    }
                }
            }
        }

        tracing::debug!(
            request_id = %req.request_id,
            tool,
            "permission::decide: no rule, AskUser"
        );
        Decision::AskUser
    }

    async fn register_pending(&self, req: PermissionRequest) -> oneshot::Receiver<PermissionOutcome> {
        let (tx, rx) = oneshot::channel();
        let reordered = reorder_options(req.options.clone());
        let targets = permission_option_targets(&reordered);
        let snapshot = PermissionRequestSnapshot {
            request_id: req.request_id.clone(),
            instance_id: req.instance_id.clone(),
            tool: req.tool_call.name.clone(),
            args: req.tool_call.raw_args.clone().or_else(|| req.tool_call.title.clone()),
            default_option_id: targets.default_option_id,
            allow_option_id: targets.allow_option_id,
            reject_option_id: targets.reject_option_id,
            options: reordered,
        };
        let mut waiters = self.waiters.lock().await;
        waiters.insert(
            req.request_id.clone(),
            PendingWaiter {
                tx,
                options: req.options.clone(),
                snapshot,
            },
        );
        tracing::debug!(
            request_id = %req.request_id,
            waiter_count = waiters.len(),
            "permission::register_pending: waiter registered"
        );
        rx
    }

    async fn resolve(&self, request_id: &str, outcome: PermissionOutcome) {
        let removed = {
            let mut waiters = self.waiters.lock().await;
            waiters.remove(request_id)
        };
        if let Some(w) = removed {
            tracing::debug!(
                request_id,
                outcome = ?outcome,
                "permission::resolve: firing waiter"
            );
            let _ = w.tx.send(outcome);
        } else {
            tracing::debug!(
                request_id,
                "permission::resolve: no waiter (already resolved or never registered)"
            );
        }
    }

    async fn forget(&self, request_id: &str) {
        let removed = {
            let mut waiters = self.waiters.lock().await;
            waiters.remove(request_id)
        };
        if let Some(w) = removed {
            tracing::debug!(request_id, "permission::forget: waiter dropped without firing");
            // Broadcast the drop so mirrors / remote subscribers prune
            // their `pending_permissions` row even when no captain
            // answer landed (typically the 10-min `WAITER_TIMEOUT`
            // path in `AcpClient::request_permission`). Sentinel
            // `option_id` keeps a uniform wire shape across the
            // captain-answered + expired roundtrips.
            self.emit_resolved(
                w.snapshot.instance_id.as_deref(),
                request_id,
                PERMISSION_EXPIRED_OPTION_ID,
            );
        }
    }

    async fn options_for(&self, request_id: &str) -> Option<Vec<PermissionOptionView>> {
        let waiters = self.waiters.lock().await;
        waiters.get(request_id).map(|w| w.options.clone())
    }

    async fn snapshot_for(&self, request_id: &str) -> Option<PermissionRequestSnapshot> {
        let waiters = self.waiters.lock().await;
        waiters.get(request_id).map(|w| w.snapshot.clone())
    }

    async fn resolve_if_pending(&self, request_id: &str, option_id: &str) -> Option<bool> {
        // Lock-scoped block so the broadcast emit (which can block on
        // a slow subscriber's drop, in theory) doesn't hold the
        // waiters Mutex.
        let removed = {
            let mut waiters = self.waiters.lock().await;
            let entry = waiters.get(request_id)?;
            if !entry.options.iter().any(|o| o.option_id == option_id) {
                tracing::debug!(
                    request_id,
                    option_id,
                    "permission::resolve_if_pending: option not in stored options"
                );
                return Some(false);
            }
            waiters.remove(request_id).expect("entry checked above")
        };
        let _ = removed.tx.send(PermissionOutcome::Selected(option_id.to_string()));
        tracing::debug!(request_id, option_id, "permission::resolve_if_pending: waiter fired");
        // Single emit site for the captain-answered roundtrip:
        // `resolve_if_pending` is the atomic membership-check + resolve
        // both production paths funnel through (the
        // `permissions/respond` RPC handler AND the Tauri
        // `permission_reply` command), so the desktop ↔ remote sync
        // event lands here exactly once per real answer.
        self.emit_resolved(removed.snapshot.instance_id.as_deref(), request_id, option_id);
        Some(true)
    }

    async fn list_pending(&self) -> Vec<PermissionRequestSnapshot> {
        let waiters = self.waiters.lock().await;
        waiters.values().map(|w| w.snapshot.clone()).collect()
    }
}

/// Strict `allow_once` picker — returns `Some(option_id)` only when
/// the agent offered an option whose `kind` is exactly `allow_once`.
/// Returns `None` for everything else (including `allow_always`,
/// vendor-specific allow flavours, and plausible-looking labels).
///
/// Used by the default-highlight path so the captain pressing
/// `Enter` on a permission prompt commits ONLY to "allow this once",
/// never to "allow forever". Agents that don't offer a single-shot
/// allow ship no default; the captain picks explicitly. Distinct
/// from [`pick_allow_option_id`] so call sites document whether they
/// are selecting a UI target or an auto-accept response.
#[must_use]
pub fn pick_allow_once_id(options: &[PermissionOptionView]) -> Option<String> {
    let picked = pick_option_by_kind(options, ALLOW_ONCE_KIND);
    if let Some(opt) = picked {
        tracing::debug!(
            option_id = %opt.option_id,
            kind = %opt.kind,
            offered = options.len(),
            "permission::pick_allow_once: option selected"
        );
    } else {
        tracing::debug!(
            offered = options.len(),
            "permission::pick_allow_once: no allow_once option offered — frontends render no default highlight"
        );
    }
    picked.map(|o| o.option_id.clone())
}

/// Pick an `allow`-shaped option id. Used on `Decision::Allow` when
/// the controller has to translate an auto-accept decision back into
/// an ACP `Selected(option_id)` response.
///
/// Strategy: exact `kind` match on `allow_once` only. No label/id/first
/// option fallback is allowed: auto-accept is a one-shot decision and
/// must not accidentally select a persistent allow option.
#[must_use]
pub fn pick_allow_option_id(options: &[PermissionOptionView]) -> Option<String> {
    let picked = pick_option_by_kind(options, ALLOW_ONCE_KIND);
    if let Some(opt) = picked {
        tracing::debug!(
            option_id = %opt.option_id,
            kind = %opt.kind,
            offered = options.len(),
            "permission::pick_allow: option selected"
        );
    }
    picked.map(|o| o.option_id.clone())
}

/// Pick a `reject`-shaped option id. Same strategy as allow but for
/// the reject half. Returns `None` when no `reject_once` option exists
/// — the caller falls back to `Cancelled`.
#[must_use]
pub fn pick_reject_option_id(options: &[PermissionOptionView]) -> Option<String> {
    let picked = pick_option_by_kind(options, REJECT_ONCE_KIND);
    if let Some(opt) = picked {
        tracing::debug!(
            option_id = %opt.option_id,
            kind = %opt.kind,
            offered = options.len(),
            "permission::pick_reject: option selected"
        );
    }
    picked.map(|o| o.option_id.clone())
}

#[must_use]
pub fn permission_option_targets(options: &[PermissionOptionView]) -> PermissionOptionTargets {
    let allow_option_id = pick_allow_once_id(options);

    PermissionOptionTargets {
        default_option_id: allow_option_id.clone(),
        allow_option_id,
        reject_option_id: pick_reject_option_id(options),
    }
}

fn pick_option_by_kind<'a>(options: &'a [PermissionOptionView], kind: &str) -> Option<&'a PermissionOptionView> {
    options.iter().find(|o| o.kind == kind)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::*;
    use crate::mcp::{HyprpilotExtension, MCPDefinition, MCPsRegistry};

    fn request(id: &str, tool: &str) -> PermissionRequest {
        PermissionRequest {
            instance_id: Some("instance-1".into()),
            request_id: id.into(),
            tool_call: ToolCallRef {
                name: tool.into(),
                tool_kind: crate::tools::ToolKind::from_mcp_name(tool).unwrap_or_default(),
                title: Some(tool.into()),
                raw_args: None,
                raw_input: None,
                content: Vec::new(),
            },
            options: vec![
                PermissionOptionView {
                    option_id: "allow-once".into(),
                    name: "Allow".into(),
                    kind: "allow_once".into(),
                },
                PermissionOptionView {
                    option_id: "reject-once".into(),
                    name: "Reject".into(),
                    kind: "reject_once".into(),
                },
            ],
        }
    }

    fn registry_with(name: &str, accept: &[&str], reject: &[&str]) -> MCPsRegistry {
        MCPsRegistry::new(vec![MCPDefinition {
            name: name.into(),
            raw: json!({ "command": "echo" }),
            hyprpilot: HyprpilotExtension {
                auto_accept_tools: accept.iter().map(|s| (*s).to_string()).collect(),
                auto_reject_tools: reject.iter().map(|s| (*s).to_string()).collect(),
            },
            source: PathBuf::from("test.json"),
        }])
    }

    #[test]
    fn decide_empty_context_asks_user() {
        let controller = DefaultPermissionController::new();
        let d = controller.decide(&request("r1", "Read"), &DecisionContext { mcps: None });
        assert_eq!(d, Decision::AskUser);
    }

    #[test]
    fn decide_per_server_reject_beats_accept() {
        let controller = DefaultPermissionController::new();
        // Globs are server-relative — `delete_*` matches the leaf
        // `delete_file` on the wire-side `mcp__filesystem__delete_file`.
        let registry = registry_with("filesystem", &["delete_*"], &["delete_*"]);
        let d = controller.decide(
            &request("r1", "mcp__filesystem__delete_file"),
            &DecisionContext { mcps: Some(&registry) },
        );
        assert_eq!(d, Decision::Deny);
    }

    #[test]
    fn decide_per_server_accept_glob_matches_leaf() {
        // Captain writes `read_*` inside the server block; the
        // `mcp__filesystem__` prefix is implicit because globs are
        // server-relative.
        let controller = DefaultPermissionController::new();
        let registry = registry_with("filesystem", &["read_*"], &[]);
        let d = controller.decide(
            &request("r1", "mcp__filesystem__read_file"),
            &DecisionContext { mcps: Some(&registry) },
        );
        assert_eq!(d, Decision::Allow);
    }

    #[test]
    fn decide_per_server_full_prefix_glob_does_not_match_leaf() {
        // Defensive: a captain who repeats the `mcp__<server>__` prefix
        // (copy-pasted from another tool) gets a no-match — the glob
        // is matched against the leaf, not the full wire name.
        let controller = DefaultPermissionController::new();
        let registry = registry_with("filesystem", &["mcp__filesystem__read_*"], &[]);
        let d = controller.decide(
            &request("r1", "mcp__filesystem__read_file"),
            &DecisionContext { mcps: Some(&registry) },
        );
        assert_eq!(d, Decision::AskUser);
    }

    #[test]
    fn decide_native_tool_skips_per_server_lane() {
        // `Bash` is a vendor-native tool with no `mcp__` prefix —
        // server attribution returns None, the lane short-circuits, and
        // we fall through to AskUser regardless of MCP config.
        let controller = DefaultPermissionController::new();
        let registry = registry_with("filesystem", &["Bash"], &[]);
        let d = controller.decide(&request("r1", "Bash"), &DecisionContext { mcps: Some(&registry) });
        assert_eq!(d, Decision::AskUser);
    }

    #[test]
    fn decide_per_server_unknown_server_asks_user() {
        // Tool prefix says it came from "ghost"; registry doesn't
        // carry that server → falls through to AskUser. Defensive
        // against a server being removed from the catalog mid-session.
        let controller = DefaultPermissionController::new();
        let registry = registry_with("filesystem", &["*"], &[]);
        let d = controller.decide(
            &request("r1", "mcp__ghost__some_tool"),
            &DecisionContext { mcps: Some(&registry) },
        );
        assert_eq!(d, Decision::AskUser);
    }

    #[tokio::test]
    async fn resolve_routes_reply_to_right_waiter() {
        let controller = DefaultPermissionController::new();
        let mut rx1 = controller.register_pending(request("one", "A")).await;
        let mut rx2 = controller.register_pending(request("two", "B")).await;

        controller
            .resolve("one", PermissionOutcome::Selected("allow".into()))
            .await;

        let first = tokio::time::timeout(Duration::from_millis(50), &mut rx1)
            .await
            .expect("rx1 resolves")
            .expect("receiver ok");
        assert_eq!(first, PermissionOutcome::Selected("allow".into()));

        // The second waiter must still be pending.
        match tokio::time::timeout(Duration::from_millis(50), &mut rx2).await {
            Err(_) => {}
            Ok(Err(_)) => panic!("rx2 closed unexpectedly"),
            Ok(Ok(v)) => panic!("rx2 resolved to {v:?} — it should still be pending"),
        }

        controller.resolve("two", PermissionOutcome::Cancelled).await;
        let second = tokio::time::timeout(Duration::from_millis(50), rx2)
            .await
            .expect("rx2 resolves")
            .expect("receiver ok");
        assert_eq!(second, PermissionOutcome::Cancelled);
    }

    #[tokio::test]
    async fn resolve_unknown_id_is_noop() {
        let controller = DefaultPermissionController::new();
        // No registration — resolve with a random id.
        controller
            .resolve("never-registered", PermissionOutcome::Selected("x".into()))
            .await;
        // No panic = pass. Re-resolving a real id after it fired also stays quiet.
        let _rx = controller.register_pending(request("once", "A")).await;
        controller.resolve("once", PermissionOutcome::Cancelled).await;
        controller.resolve("once", PermissionOutcome::Cancelled).await;
    }

    #[tokio::test]
    async fn resolve_if_pending_unknown_request_returns_none() {
        let controller = DefaultPermissionController::new();
        assert_eq!(controller.resolve_if_pending("ghost", "allow-once").await, None);
    }

    #[tokio::test]
    async fn resolve_if_pending_invalid_option_returns_some_false_and_keeps_waiter() {
        let controller = DefaultPermissionController::new();
        let mut rx = controller.register_pending(request("r1", "Bash")).await;
        let res = controller.resolve_if_pending("r1", "ghost-option").await;
        assert_eq!(res, Some(false));
        match tokio::time::timeout(Duration::from_millis(50), &mut rx).await {
            Err(_) => {}
            Ok(_) => panic!("waiter must not fire on invalid option"),
        }
        // Waiter still registered — options_for returns the original list.
        assert!(controller.options_for("r1").await.is_some());
    }

    #[tokio::test]
    async fn resolve_if_pending_valid_option_returns_some_true_and_fires_waiter() {
        let controller = DefaultPermissionController::new();
        let rx = controller.register_pending(request("r1", "Bash")).await;
        let res = controller.resolve_if_pending("r1", "allow-once").await;
        assert_eq!(res, Some(true));
        let outcome = tokio::time::timeout(Duration::from_millis(50), rx)
            .await
            .expect("waiter fires")
            .expect("receiver ok");
        assert_eq!(outcome, PermissionOutcome::Selected("allow-once".into()));
        // Waiter dropped from the map.
        assert!(controller.options_for("r1").await.is_none());
    }

    /// Timeout enforcement lives at the call site — `AcpClient::request_permission`
    /// wraps `rx.await` in `tokio::time::timeout(WAITER_TIMEOUT, rx)`
    /// and calls `forget(request_id)` on elapsed. This test pins the
    /// `forget` half: after the caller gives up, the waiter is gone
    /// from the map and a late `resolve` is a no-op.
    #[tokio::test]
    async fn forget_drops_waiter_without_firing_sender() {
        let controller = DefaultPermissionController::new();
        let _rx = controller.register_pending(request("slow", "Bash")).await;
        controller.forget("slow").await;
        assert!(controller.options_for("slow").await.is_none());
        // Second forget on the same id is a no-op (same invariant as resolve).
        controller.forget("slow").await;
    }

    #[tokio::test]
    async fn resolve_if_pending_emits_permission_resolved_event() {
        // Phase A4: captain answers a permission → controller broadcasts
        // a `PermissionResolved` so the mirror (and remote subscribers)
        // can prune their `pending_permissions` row even when the
        // answer landed on the other transport.
        let controller = DefaultPermissionController::new();
        let (tx, mut rx) = broadcast::channel::<InstanceEvent>(8);
        controller.attach_events_tx(tx);

        let _waiter_rx = controller.register_pending(request("r1", "Bash")).await;
        let res = controller.resolve_if_pending("r1", "allow-once").await;
        assert_eq!(res, Some(true));

        let evt = tokio::time::timeout(Duration::from_millis(50), rx.recv())
            .await
            .expect("event received within 50ms")
            .expect("broadcast not closed");
        match evt {
            InstanceEvent::PermissionResolved {
                instance_id,
                request_id,
                option_id,
            } => {
                assert_eq!(request_id, "r1");
                assert_eq!(option_id, "allow-once");
                // `request()` carries `instance_id: Some("instance-1")`.
                assert_eq!(instance_id, "instance-1");
            }
            other => panic!("expected PermissionResolved, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn resolve_if_pending_no_event_when_request_unknown_or_option_invalid() {
        // The Some(false) and None paths must NOT emit. Mirrors / remote
        // subscribers shouldn't see a `PermissionResolved` for a row
        // that was never resolved.
        let controller = DefaultPermissionController::new();
        let (tx, mut rx) = broadcast::channel::<InstanceEvent>(8);
        controller.attach_events_tx(tx);

        // Unknown request_id (None path).
        assert_eq!(controller.resolve_if_pending("ghost", "allow-once").await, None);
        match rx.try_recv() {
            Err(broadcast::error::TryRecvError::Empty) => {}
            other => panic!("expected no event for unknown request_id, got {other:?}"),
        }

        // Registered but invalid option (Some(false) path).
        let _waiter_rx = controller.register_pending(request("r1", "Bash")).await;
        assert_eq!(controller.resolve_if_pending("r1", "nope").await, Some(false));
        match rx.try_recv() {
            Err(broadcast::error::TryRecvError::Empty) => {}
            other => panic!("expected no event for invalid option, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn forget_emits_expired_permission_resolved_event() {
        // Timeout path: `AcpClient::request_permission`'s
        // `WAITER_TIMEOUT` elapses → `controller.forget()` runs →
        // mirror needs to drop the row even though no captain answer
        // ever came. Sentinel `option_id` keeps the wire shape uniform.
        let controller = DefaultPermissionController::new();
        let (tx, mut rx) = broadcast::channel::<InstanceEvent>(8);
        controller.attach_events_tx(tx);

        let _waiter_rx = controller.register_pending(request("slow", "Bash")).await;
        controller.forget("slow").await;

        let evt = tokio::time::timeout(Duration::from_millis(50), rx.recv())
            .await
            .expect("event received within 50ms")
            .expect("broadcast not closed");
        match evt {
            InstanceEvent::PermissionResolved {
                request_id, option_id, ..
            } => {
                assert_eq!(request_id, "slow");
                assert_eq!(option_id, PERMISSION_EXPIRED_OPTION_ID);
            }
            other => panic!("expected PermissionResolved, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn two_identical_asks_back_to_back_both_prompt() {
        let controller = DefaultPermissionController::new();
        let d1 = controller.decide(&request("r1", "Bash"), &DecisionContext { mcps: None });
        let d2 = controller.decide(&request("r2", "Bash"), &DecisionContext { mcps: None });
        assert_eq!(d1, Decision::AskUser);
        assert_eq!(d2, Decision::AskUser);
    }

    #[test]
    fn pick_allow_option_prefers_allow_once() {
        let opts = vec![
            PermissionOptionView {
                option_id: "o1".into(),
                name: "Allow Always".into(),
                kind: "allow_always".into(),
            },
            PermissionOptionView {
                option_id: "o2".into(),
                name: "Allow Once".into(),
                kind: "allow_once".into(),
            },
        ];
        assert_eq!(pick_allow_option_id(&opts).as_deref(), Some("o2"));
    }

    #[test]
    fn pick_allow_option_never_picks_allow_always_kind() {
        let opts = vec![PermissionOptionView {
            option_id: "o1".into(),
            name: "Allow Always".into(),
            kind: "allow_always".into(),
        }];
        assert_eq!(pick_allow_option_id(&opts), None);
    }

    #[test]
    fn pick_allow_option_ignores_unknown_kind_even_when_label_looks_allow_once() {
        let opts = vec![PermissionOptionView {
            option_id: "once".into(),
            name: "Allow once".into(),
            kind: "unknown".into(),
        }];
        assert_eq!(pick_allow_option_id(&opts), None);
    }

    #[test]
    fn pick_allow_once_returns_only_exact_allow_once() {
        let opts = vec![
            PermissionOptionView {
                option_id: "o1".into(),
                name: "Allow Always".into(),
                kind: "allow_always".into(),
            },
            PermissionOptionView {
                option_id: "o2".into(),
                name: "Allow Once".into(),
                kind: "allow_once".into(),
            },
        ];
        assert_eq!(pick_allow_once_id(&opts).as_deref(), Some("o2"));
    }

    /// Strict: without an exact `allow_once`, the picker returns
    /// `None` — `allow_always` is NOT a fallback for the default
    /// highlight. Captains pressing `Enter` should never commit to
    /// "allow forever" without an explicit pick.
    #[test]
    fn pick_allow_once_returns_none_when_only_allow_always_offered() {
        let opts = vec![PermissionOptionView {
            option_id: "o1".into(),
            name: "Allow Always".into(),
            kind: "allow_always".into(),
        }];
        assert_eq!(pick_allow_once_id(&opts), None);
    }

    #[test]
    fn pick_allow_once_ignores_adapter_labels_when_kind_is_unknown() {
        let opts = vec![
            PermissionOptionView {
                option_id: "allow".into(),
                name: "Allow".into(),
                kind: "unknown".into(),
            },
            PermissionOptionView {
                option_id: "approved".into(),
                name: "Yes, proceed".into(),
                kind: "unknown".into(),
            },
            PermissionOptionView {
                option_id: "once".into(),
                name: "Allow once".into(),
                kind: "unknown".into(),
            },
        ];
        assert_eq!(pick_allow_once_id(&opts), None);
    }

    #[test]
    fn pick_allow_once_never_picks_allow_always_kind() {
        let opts = vec![PermissionOptionView {
            option_id: "o1".into(),
            name: "Allow Always".into(),
            kind: "allow_always".into(),
        }];
        assert_eq!(pick_allow_once_id(&opts), None);
    }

    #[test]
    fn pick_allow_once_returns_none_on_empty_options() {
        assert_eq!(pick_allow_once_id(&[]), None);
    }

    #[test]
    fn pick_reject_option_prefers_reject_once() {
        let opts = vec![
            PermissionOptionView {
                option_id: "r1".into(),
                name: "Reject Always".into(),
                kind: "reject_always".into(),
            },
            PermissionOptionView {
                option_id: "r2".into(),
                name: "Reject Once".into(),
                kind: "reject_once".into(),
            },
        ];
        assert_eq!(pick_reject_option_id(&opts).as_deref(), Some("r2"));
    }

    #[test]
    fn pick_reject_option_returns_none_when_no_reject_once() {
        let opts = vec![PermissionOptionView {
            option_id: "allow-once".into(),
            name: "Allow".into(),
            kind: "allow_once".into(),
        }];
        assert!(pick_reject_option_id(&opts).is_none());
    }

    #[test]
    fn pick_reject_option_ignores_unknown_kind_even_when_label_looks_reject() {
        let opts = vec![PermissionOptionView {
            option_id: "reject".into(),
            name: "Reject".into(),
            kind: "unknown".into(),
        }];
        assert_eq!(pick_reject_option_id(&opts), None);
    }

    #[test]
    fn pick_reject_option_never_picks_reject_always_kind() {
        let opts = vec![PermissionOptionView {
            option_id: "rej-always".into(),
            name: "Reject Always".into(),
            kind: "reject_always".into(),
        }];
        assert_eq!(pick_reject_option_id(&opts), None);
    }

    #[test]
    fn permission_targets_cover_supported_adapter_shapes_by_kind() {
        let cases = [
            (
                "claude-code-tool",
                vec![
                    PermissionOptionView {
                        option_id: "allow_always".into(),
                        name: "Always Allow all Bash".into(),
                        kind: "allow_always".into(),
                    },
                    PermissionOptionView {
                        option_id: "allow".into(),
                        name: "Allow".into(),
                        kind: "allow_once".into(),
                    },
                    PermissionOptionView {
                        option_id: "reject".into(),
                        name: "Reject".into(),
                        kind: "reject_once".into(),
                    },
                ],
                "allow",
                "reject",
            ),
            (
                "claude-code-plan-exit",
                vec![
                    PermissionOptionView {
                        option_id: "auto".into(),
                        name: r#"Yes, and use "auto" mode"#.into(),
                        kind: "allow_always".into(),
                    },
                    PermissionOptionView {
                        option_id: "acceptEdits".into(),
                        name: "Yes, and auto-accept edits".into(),
                        kind: "allow_always".into(),
                    },
                    PermissionOptionView {
                        option_id: "default".into(),
                        name: "Yes, and manually approve edits".into(),
                        kind: "allow_once".into(),
                    },
                    PermissionOptionView {
                        option_id: "plan".into(),
                        name: "No, keep planning".into(),
                        kind: "reject_once".into(),
                    },
                ],
                "default",
                "plan",
            ),
            (
                "codex-exec",
                vec![
                    PermissionOptionView {
                        option_id: "approved".into(),
                        name: "Yes, proceed".into(),
                        kind: "allow_once".into(),
                    },
                    PermissionOptionView {
                        option_id: "approved-for-session".into(),
                        name: "Yes, and don't ask again for this command in this session".into(),
                        kind: "allow_always".into(),
                    },
                    PermissionOptionView {
                        option_id: "denied".into(),
                        name: "No, continue without running it".into(),
                        kind: "reject_once".into(),
                    },
                ],
                "approved",
                "denied",
            ),
            (
                "codex-mcp-approval",
                vec![
                    PermissionOptionView {
                        option_id: "approved".into(),
                        name: "Allow".into(),
                        kind: "allow_once".into(),
                    },
                    PermissionOptionView {
                        option_id: "approved-for-session".into(),
                        name: "Allow for this session".into(),
                        kind: "allow_always".into(),
                    },
                    PermissionOptionView {
                        option_id: "approved-always".into(),
                        name: "Allow and don't ask again".into(),
                        kind: "allow_always".into(),
                    },
                    PermissionOptionView {
                        option_id: "cancel".into(),
                        name: "Cancel".into(),
                        kind: "reject_once".into(),
                    },
                ],
                "approved",
                "cancel",
            ),
            (
                "codex-patch-approval",
                vec![
                    PermissionOptionView {
                        option_id: "approved".into(),
                        name: "Yes".into(),
                        kind: "allow_once".into(),
                    },
                    PermissionOptionView {
                        option_id: "abort".into(),
                        name: "No, provide feedback".into(),
                        kind: "reject_once".into(),
                    },
                ],
                "approved",
                "abort",
            ),
            (
                "opencode",
                vec![
                    PermissionOptionView {
                        option_id: "once".into(),
                        name: "Allow once".into(),
                        kind: "allow_once".into(),
                    },
                    PermissionOptionView {
                        option_id: "always".into(),
                        name: "Always allow".into(),
                        kind: "allow_always".into(),
                    },
                    PermissionOptionView {
                        option_id: "reject".into(),
                        name: "Reject".into(),
                        kind: "reject_once".into(),
                    },
                ],
                "once",
                "reject",
            ),
        ];

        for (adapter, options, allow, reject) in cases {
            let targets = permission_option_targets(&options);

            assert_eq!(targets.default_option_id.as_deref(), Some(allow), "{adapter}");
            assert_eq!(targets.allow_option_id.as_deref(), Some(allow), "{adapter}");
            assert_eq!(targets.reject_option_id.as_deref(), Some(reject), "{adapter}");
        }
    }

    #[test]
    fn permission_targets_do_not_guess_when_once_kinds_are_absent() {
        let opts = vec![
            PermissionOptionView {
                option_id: "allow_always".into(),
                name: "Allow always".into(),
                kind: "allow_always".into(),
            },
            PermissionOptionView {
                option_id: "reject_always".into(),
                name: "Reject always".into(),
                kind: "reject_always".into(),
            },
            PermissionOptionView {
                option_id: "maybe".into(),
                name: "Allow".into(),
                kind: "unknown".into(),
            },
        ];
        let targets = permission_option_targets(&opts);

        assert_eq!(targets.default_option_id, None);
        assert_eq!(targets.allow_option_id, None);
        assert_eq!(targets.reject_option_id, None);
    }

    /// Strict: wire kinds are protocol strings, so a mixed-case value
    /// is treated as unknown instead of being guessed into a keybind
    /// target.
    #[test]
    fn pick_allow_once_does_not_case_fold_kind() {
        let opts = vec![PermissionOptionView {
            option_id: "ok".into(),
            name: "Allow Once".into(),
            kind: "Allow_Once".into(),
        }];
        assert_eq!(pick_allow_once_id(&opts), None);
    }

    /// Daemon enforces a canonical wire order: `allow_always` first,
    /// `allow_once` second, `reject_once` third, then anything else.
    /// codex ships `[Approve Once, Approve This Session, Reject]`
    /// — after reordering, the captain's frontends see the same
    /// `[Always, Once, Reject]` arrangement as claude.
    #[test]
    fn reorder_options_canonicalises_codex_order() {
        let opts = vec![
            PermissionOptionView {
                option_id: "approve-once".into(),
                name: "Approve Once".into(),
                kind: "allow_once".into(),
            },
            PermissionOptionView {
                option_id: "approve-session".into(),
                name: "Approve This Session".into(),
                kind: "allow_always".into(),
            },
            PermissionOptionView {
                option_id: "reject".into(),
                name: "Reject".into(),
                kind: "reject_once".into(),
            },
        ];
        let out = reorder_options(opts);
        let ids: Vec<String> = out.iter().map(|o| o.option_id.clone()).collect();

        assert_eq!(ids, vec!["approve-session", "approve-once", "reject"]);
    }

    /// Vendor without an `allow_always` (codex's three-option set
    /// uses an `allow_always`; this hypothetical case covers a
    /// future vendor that only ships once + reject). Order stays
    /// stable: `allow_once` first (no always to lead it), reject
    /// second.
    #[test]
    fn reorder_options_handles_missing_buckets() {
        let opts = vec![
            PermissionOptionView {
                option_id: "reject".into(),
                name: "Reject".into(),
                kind: "reject_once".into(),
            },
            PermissionOptionView {
                option_id: "once".into(),
                name: "Once".into(),
                kind: "allow_once".into(),
            },
        ];
        let out = reorder_options(opts);
        let ids: Vec<String> = out.iter().map(|o| o.option_id.clone()).collect();

        assert_eq!(ids, vec!["once", "reject"]);
    }

    /// Unclassified options trail at the end in original order.
    #[test]
    fn reorder_options_preserves_unclassified_tail_order() {
        let opts = vec![
            PermissionOptionView {
                option_id: "mystery-1".into(),
                name: "Mystery 1".into(),
                kind: "unknown".into(),
            },
            PermissionOptionView {
                option_id: "once".into(),
                name: "Once".into(),
                kind: "allow_once".into(),
            },
            PermissionOptionView {
                option_id: "mystery-2".into(),
                name: "Mystery 2".into(),
                kind: "unknown".into(),
            },
        ];
        let out = reorder_options(opts);
        let ids: Vec<String> = out.iter().map(|o| o.option_id.clone()).collect();

        assert_eq!(ids, vec!["once", "mystery-1", "mystery-2"]);
    }
}

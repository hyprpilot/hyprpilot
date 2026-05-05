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
//! itself owns whatever persistence it offers (claude-code-acp writes
//! to `~/.claude/settings.json`, etc.). Hyprpilot does not shadow that.
//!
//! `register_pending` + `resolve` own the oneshot waiter map that
//! bridges the Tauri `permission_reply` command back to the awaiting
//! ACP handler.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use tokio::sync::{oneshot, Mutex};

use crate::mcp::MCPsRegistry;

/// How long an `AskUser` waiter stays live before the caller
/// abandons it and treats the outcome as `Cancelled`. Matches the
/// issue's 10-min target; a prompt left unanswered across a
/// compositor lock or a user's lunch break should not wedge the
/// ACP session forever. Enforced by `tokio::time::timeout` at the
/// `AcpClient::request_permission` call site; the controller itself
/// does not spawn a timer — that let a detached `sleep(WAITER_TIMEOUT)`
/// accumulate one future per resolved prompt.
pub const WAITER_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// UI-facing projection of a permission option. Wire-normalised so
/// the webview doesn't need to speak any specific vendor's shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PermissionOptionView {
    pub option_id: String,
    pub name: String,
    /// Normalised wire name: `"allow_once" | "allow_always" |
    /// "reject_once" | "reject_always"` today. Closed set once the
    /// crate's upstream enum stabilises; `String` keeps the UI
    /// tolerant to new-variant drift today.
    pub kind: String,
}

/// Identity projection of the tool behind a permission request. The
/// glob chain matches on `name` only; `title` / `raw_args` /
/// `kind_wire` are carried for the UI and (future) argument-scoped /
/// kind-scoped rules — they are opaque to the allowlist decision
/// today.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallRef {
    /// Canonical tool name for glob matching. Adapters populate with
    /// the most stable identifier their wire exposes (for ACP: the
    /// ToolKind wire name, falling back to the tool's `title`).
    pub name: String,
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
    /// Closed-set tool kind wire string when `name` was resolved from
    /// a typed enum (ACP `ToolKind`); `None` when name fell back to
    /// the human-readable title. The UI uses this to colour the
    /// permission prompt off the closed-set theme map; the matcher
    /// ignores it today (future kind-scoped rules will read it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind_wire: Option<String>,
    /// Raw `tool_call.content[]` array — pass-through of the ACP wire
    /// shape (`{ type: 'content' | 'diff' | 'terminal', ... }`).
    /// Populated for permissions whose markdown body lives on the
    /// tool-call's content array (claude-code's `Switch mode` flow
    /// ships the plan body here, not on `raw_input.plan`). UI walks
    /// the array directly — no server-side joining.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<serde_json::Value>,
}

impl ToolCallRef {
    /// Wire `kind` string for the permission-prompt UI. Reads
    /// `kind_wire` (lowercased) when set; falls back to the neutral
    /// `"acp"` sentinel so free-form English (title fallbacks) never
    /// bleeds into the UI's closed-set theme map.
    #[must_use]
    pub fn permission_kind_wire(&self) -> String {
        self.kind_wire
            .as_deref()
            .map(str::to_ascii_lowercase)
            .unwrap_or_else(|| "acp".to_string())
    }
}

/// Everything the controller needs to make a decision and route a
/// later reply. `request_id` is the correlation key the reply
/// command sends back; `instance_id` tags the snapshot returned by
/// `permissions/pending` so callers can address a specific live
/// instance.
#[derive(Debug, Clone)]
pub struct PermissionRequest {
    pub session_id: String,
    pub instance_id: Option<String>,
    pub request_id: String,
    pub tool_call: ToolCallRef,
    pub options: Vec<PermissionOptionView>,
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
/// `Selected(option_id)` or `Cancelled`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionOutcome {
    Selected(String),
    Cancelled,
}

/// A request the adapter fans out to the webview via
/// `acp:permission-request`. Carries the options + the identity bits
/// needed to route the reply back to the awaiting actor.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionPrompt {
    pub session_id: String,
    pub request_id: String,
    pub options: Vec<PermissionOptionView>,
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
}

/// The UI's answer back. `PermissionController` threads these
/// through the adapter so the awaiting actor resumes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionReply {
    pub session_id: String,
    pub request_id: String,
    pub option_id: String,
}

/// Everything the decision pipeline needs at call time. `mcps`
/// provides the per-server hyprpilot-extension globs; when `None`
/// every decision falls through to `AskUser`.
///
/// Tool→server attribution is by prefix convention — `mcp__<server>__<tool>`,
/// the shared shape across claude-code-acp / codex-acp / opencode-acp
/// (all three namespace MCP tools the same way). Vendor-side native
/// tools (Bash, Read, …) carry no `mcp__` prefix and skip the lookup
/// entirely.
pub struct DecisionContext<'a> {
    pub mcps: Option<&'a MCPsRegistry>,
}

impl<'a> DecisionContext<'a> {
    /// All-misses context — every decision falls through to `AskUser`.
    /// Used by tests and by call sites that haven't yet wired the
    /// MCP registry.
    #[must_use]
    pub fn empty() -> Self {
        Self { mcps: None }
    }
}

/// Parse `mcp__<server>__<leaf>` → `(<server>, <leaf>)`. Returns `None`
/// for vendor-native tool names (Bash, Read, …) that don't carry the
/// MCP prefix. The leaf is what per-server auto-accept / auto-reject
/// globs match against — captains write `read_*` / `delete_*` inside
/// the server block; repeating the `mcp__<server>__` prefix would be
/// redundant.
#[must_use]
pub fn parse_mcp_tool_name(tool: &str) -> Option<(&str, &str)> {
    let after_prefix = tool.strip_prefix("mcp__")?;
    let (server, leaf) = after_prefix.split_once("__")?;
    if server.is_empty() || leaf.is_empty() {
        return None;
    }
    Some((server, leaf))
}

/// The decision + waiter surface. `decide` is synchronous (pure
/// lookups against the per-server MCP glob config). `register_pending`
/// + `resolve` own the oneshot map that bridges the Tauri
/// `permission_reply` command back to the awaiting ACP handler.
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
#[derive(Debug, Default)]
pub struct DefaultPermissionController {
    waiters: Arc<Mutex<HashMap<String, PendingWaiter>>>,
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
}

/// Compile a list of glob patterns into a `GlobSet`. Returns `None`
/// when the input is empty or every pattern fails to compile (logged).
/// Used inside `decide` for per-server hyprpilot extension match.
fn compile_globs(patterns: &[String]) -> Option<GlobSet> {
    if patterns.is_empty() {
        return None;
    }
    let mut builder = GlobSetBuilder::new();
    let mut added = 0_usize;
    for p in patterns {
        match Glob::new(p) {
            Ok(g) => {
                builder.add(g);
                added += 1;
            }
            Err(err) => {
                tracing::warn!(pattern = %p, %err, "permission::decide: skipping invalid glob");
            }
        }
    }
    if added == 0 {
        return None;
    }
    builder.build().ok()
}

#[async_trait]
impl PermissionController for DefaultPermissionController {
    fn decide(&self, req: &PermissionRequest, ctx: &DecisionContext<'_>) -> Decision {
        let tool = req.tool_call.name.as_str();

        // Per-server hyprpilot extension globs. Attribute the tool to
        // its server via the `mcp__<server>__<leaf>` prefix convention,
        // then match the SERVER-RELATIVE leaf against that server's
        // accept / reject globs. Captains write `read_*` / `delete_*`
        // under the server block; the `mcp__<server>__` prefix is
        // implicit. Reject beats accept. Vendor-native tools (no
        // prefix) skip this lane entirely.
        if let Some(registry) = ctx.mcps {
            if let Some((server_name, leaf)) = parse_mcp_tool_name(tool) {
                if let Some(def) = registry.get(server_name) {
                    let reject_set = compile_globs(&def.hyprpilot.auto_reject_tools);
                    if reject_set.as_ref().is_some_and(|gs| gs.is_match(leaf)) {
                        tracing::debug!(
                            request_id = %req.request_id,
                            tool,
                            server = %server_name,
                            leaf,
                            "permission::decide: per-server reject glob hit"
                        );
                        return Decision::Deny;
                    }
                    let accept_set = compile_globs(&def.hyprpilot.auto_accept_tools);
                    if accept_set.as_ref().is_some_and(|gs| gs.is_match(leaf)) {
                        tracing::debug!(
                            request_id = %req.request_id,
                            tool,
                            server = %server_name,
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
        let snapshot = PermissionRequestSnapshot {
            request_id: req.request_id.clone(),
            instance_id: req.instance_id.clone(),
            tool: req.tool_call.name.clone(),
            args: req.tool_call.raw_args.clone().or_else(|| req.tool_call.title.clone()),
            options: req.options.clone(),
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
        let mut waiters = self.waiters.lock().await;
        if waiters.remove(request_id).is_some() {
            tracing::debug!(request_id, "permission::forget: waiter dropped without firing");
        }
    }

    async fn options_for(&self, request_id: &str) -> Option<Vec<PermissionOptionView>> {
        let waiters = self.waiters.lock().await;
        waiters.get(request_id).map(|w| w.options.clone())
    }

    async fn resolve_if_pending(&self, request_id: &str, option_id: &str) -> Option<bool> {
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
        let removed = waiters.remove(request_id).expect("entry checked above");
        let _ = removed.tx.send(PermissionOutcome::Selected(option_id.to_string()));
        tracing::debug!(request_id, option_id, "permission::resolve_if_pending: waiter fired");
        Some(true)
    }

    async fn list_pending(&self) -> Vec<PermissionRequestSnapshot> {
        let waiters = self.waiters.lock().await;
        waiters.values().map(|w| w.snapshot.clone()).collect()
    }
}

/// Pick an `allow`-shaped option id. Used on `Decision::Allow` when
/// the controller has to translate back to ACP's
/// `Selected(option_id)` wire. Strategy: first an exact `kind`
/// match on `allow_once` / `allow_always`, else anything whose
/// `option_id` or `name` contains "allow" case-insensitively, else
/// the first option overall.
#[must_use]
pub fn pick_allow_option_id(options: &[PermissionOptionView]) -> Option<String> {
    let picked = options
        .iter()
        .find(|o| o.kind == "allow_once")
        .or_else(|| options.iter().find(|o| o.kind == "allow_always"))
        .or_else(|| {
            options.iter().find(|o| {
                o.option_id.to_ascii_lowercase().contains("allow") || o.name.to_ascii_lowercase().contains("allow")
            })
        })
        .or_else(|| options.first());
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
/// the reject half. Returns `None` when no reject-coloured option
/// exists — the caller falls back to `Cancelled`.
#[must_use]
pub fn pick_reject_option_id(options: &[PermissionOptionView]) -> Option<String> {
    let picked = options
        .iter()
        .find(|o| o.kind == "reject_once")
        .or_else(|| options.iter().find(|o| o.kind == "reject_always"))
        .or_else(|| {
            options.iter().find(|o| {
                o.option_id.to_ascii_lowercase().contains("reject")
                    || o.option_id.to_ascii_lowercase().contains("deny")
                    || o.name.to_ascii_lowercase().contains("reject")
                    || o.name.to_ascii_lowercase().contains("deny")
            })
        });
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::*;
    use crate::mcp::{HyprpilotExtension, MCPDefinition, MCPsRegistry};

    fn request(id: &str, tool: &str) -> PermissionRequest {
        PermissionRequest {
            session_id: "sess-1".into(),
            instance_id: Some("instance-1".into()),
            request_id: id.into(),
            tool_call: ToolCallRef {
                name: tool.into(),
                title: Some(tool.into()),
                raw_args: None,
                raw_input: None,
                kind_wire: None,
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
        let d = controller.decide(&request("r1", "Read"), &DecisionContext::empty());
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

    #[test]
    fn parse_mcp_tool_name_strips_prefix_and_returns_leaf() {
        assert_eq!(
            parse_mcp_tool_name("mcp__filesystem__read_file"),
            Some(("filesystem", "read_file"))
        );
        assert_eq!(
            parse_mcp_tool_name("mcp__github__create_pr"),
            Some(("github", "create_pr"))
        );
    }

    #[test]
    fn parse_mcp_tool_name_rejects_non_mcp_or_empty_components() {
        assert!(parse_mcp_tool_name("Bash").is_none());
        assert!(parse_mcp_tool_name("Read").is_none());
        assert!(parse_mcp_tool_name("mcp__no_separator").is_none());
        assert!(parse_mcp_tool_name("mcp____empty_server").is_none());
        assert!(parse_mcp_tool_name("mcp__server__").is_none(), "empty leaf rejected");
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

    /// Timeout enforcement moved out of the controller in the K-245
    /// review pass — `AcpClient::request_permission` wraps `rx.await`
    /// in `tokio::time::timeout(WAITER_TIMEOUT, rx)` and calls
    /// `forget(request_id)` on elapsed. This test pins the `forget`
    /// half: after the caller gives up, the waiter is gone from the
    /// map and a late `resolve` is a no-op.
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
    async fn two_identical_asks_back_to_back_both_prompt() {
        let controller = DefaultPermissionController::new();
        let d1 = controller.decide(&request("r1", "Bash"), &DecisionContext::empty());
        let d2 = controller.decide(&request("r2", "Bash"), &DecisionContext::empty());
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
    fn pick_allow_option_falls_back_to_name_match() {
        let opts = vec![
            PermissionOptionView {
                option_id: "yes".into(),
                name: "Allow This".into(),
                kind: "unknown".into(),
            },
            PermissionOptionView {
                option_id: "other".into(),
                name: "Skip".into(),
                kind: "unknown".into(),
            },
        ];
        assert_eq!(pick_allow_option_id(&opts).as_deref(), Some("yes"));
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
    fn pick_reject_option_returns_none_when_no_reject_shape() {
        let opts = vec![PermissionOptionView {
            option_id: "allow-once".into(),
            name: "Allow".into(),
            kind: "allow_once".into(),
        }];
        assert!(pick_reject_option_id(&opts).is_none());
    }
}

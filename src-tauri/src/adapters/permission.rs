//! Generic permission-prompt vocabulary + `PermissionController` trait.
//!
//! The adapter emits a `PermissionPrompt` via
//! `InstanceEvent::PermissionRequest` when the decision chain bounces
//! to the UI; the webview replies with a `PermissionReply {
//! option_id }`. K-245 replaces the auto-`Cancelled` stub with a real
//! chain: profile reject-globs → profile accept-globs → ask the user
//! and block the ACP response until the reply lands (or the 10-minute
//! waiter timeout fires, whichever first).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::{oneshot, Mutex};

use crate::config::ProfileConfig;

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
    /// Concatenated text from every `content[]` block whose type is
    /// `content` / `text`. Populated for permissions whose markdown
    /// body lives on the tool-call's content array rather than its
    /// `raw_input` (claude-code's `Switch mode` flow ships the plan
    /// body here, not on `raw_input.plan`). The UI reads this as a
    /// fallback markdown body for the modal when the rawInput
    /// shape-detector misses.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_text: Option<String>,
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

/// The decision + waiter surface. `decide` is synchronous (pure
/// glob lookup against a profile); `register_pending` + `resolve`
/// own the oneshot map that bridges the Tauri `permission_reply`
/// command back to the awaiting ACP handler.
#[async_trait]
pub trait PermissionController: Send + Sync + 'static {
    /// Apply the profile allowlist chain. Reject beats accept; a
    /// miss on both lists returns `AskUser`.
    fn decide(&self, req: &PermissionRequest, profile: Option<&ProfileConfig>) -> Decision;

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

/// Default impl: an in-memory waiter map, profile globs compiled on
/// every `decide` call (the glob set is tiny — a handful of patterns
/// per profile — so caching is premature).
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

#[async_trait]
impl PermissionController for DefaultPermissionController {
    fn decide(&self, req: &PermissionRequest, profile: Option<&ProfileConfig>) -> Decision {
        let Some(profile) = profile else {
            tracing::debug!(
                request_id = %req.request_id,
                tool = %req.tool_call.name,
                "permission::decide: no profile attached, AskUser"
            );
            return Decision::AskUser;
        };
        // Glob compile failures never reach here — TOML validation
        // rejects bad patterns at load time. If a hand-constructed
        // profile slips past validation, we fall through to AskUser
        // rather than panic.
        let Ok((accept, reject)) = profile.compile_tool_globs() else {
            tracing::warn!(
                profile = %profile.id,
                "permission::decide: glob compile failed, defaulting to AskUser"
            );
            return Decision::AskUser;
        };
        let name = req.tool_call.name.as_str();
        if reject.is_match(name) {
            tracing::debug!(
                request_id = %req.request_id,
                profile = %profile.id,
                tool = %name,
                "permission::decide: matched auto_reject_tools glob"
            );
            return Decision::Deny;
        }
        if accept.is_match(name) {
            tracing::debug!(
                request_id = %req.request_id,
                profile = %profile.id,
                tool = %name,
                "permission::decide: matched auto_accept_tools glob"
            );
            return Decision::Allow;
        }
        tracing::debug!(
            request_id = %req.request_id,
            profile = %profile.id,
            tool = %name,
            "permission::decide: no glob matched, AskUser"
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
    use super::*;

    fn profile_with_globs(id: &str, accept: &[&str], reject: &[&str]) -> ProfileConfig {
        ProfileConfig {
            id: id.into(),
            agent: "claude-code".into(),
            model: None,
            system_prompt: None,
            system_prompt_file: None,
            auto_accept_tools: accept.iter().map(|s| s.to_string()).collect(),
            auto_reject_tools: reject.iter().map(|s| s.to_string()).collect(),
            mcps: None,
            skills: None,
            mode: None,
            cwd: None,
            env: Default::default(),
        }
    }

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
                content_text: None,
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

    #[test]
    fn decide_profile_reject_beats_accept() {
        let controller = DefaultPermissionController::new();
        let profile = profile_with_globs("p", &["Bash"], &["Bash"]);
        let d = controller.decide(&request("r1", "Bash"), Some(&profile));
        assert_eq!(d, Decision::Deny);
    }

    #[test]
    fn decide_profile_accept_skips_user() {
        let controller = DefaultPermissionController::new();
        let profile = profile_with_globs("p", &["Read*"], &[]);
        let d = controller.decide(&request("r1", "ReadFile"), Some(&profile));
        assert_eq!(d, Decision::Allow);
    }

    #[test]
    fn decide_no_match_asks_user() {
        let controller = DefaultPermissionController::new();
        let profile = profile_with_globs("p", &["Read"], &["Delete"]);
        let d = controller.decide(&request("r1", "Edit"), Some(&profile));
        assert_eq!(d, Decision::AskUser);
    }

    #[test]
    fn decide_without_profile_asks_user() {
        let controller = DefaultPermissionController::new();
        let d = controller.decide(&request("r1", "Read"), None);
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
        let profile = profile_with_globs("p", &[], &[]);

        let d1 = controller.decide(&request("r1", "Bash"), Some(&profile));
        let d2 = controller.decide(&request("r2", "Bash"), Some(&profile));
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

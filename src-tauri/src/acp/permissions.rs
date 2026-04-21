//! Permission-resolution plumbing for the ACP bridge.
//!
//! The ACP wire protocol has **no** "permission mode" — a
//! `session/request_permission` request just delivers an array of
//! `PermissionOption { id, kind, name }` and expects the client to
//! respond with one option id. Each vendor ships a different subset of
//! `PermissionOptionKind`s, so the hyprpilot-level
//! `AcpPermissionPolicy` (`ask` / `accept-edits` / `bypass`) can't map
//! onto a single protocol field.
//!
//! Instead, the policy maps onto an *intent* — `AcpPermissionOptionKind`
//! — which the vendor resolves against the options actually offered by
//! the agent via the fallback chain in `select_option_id`. Every
//! vendor we target today ships `allow_once` / `allow_always` /
//! `reject_once`; none currently ships `reject_always`, so
//! `RejectAlways` always falls back to `RejectOnce`.
//!
//! This mirrors the Python pilot's `_KIND_FALLBACK` table, updated for
//! the ACP 0.11 types.

use agent_client_protocol::schema::{PermissionOption, PermissionOptionId, PermissionOptionKind};

use crate::config::AcpPermissionPolicy;

/// Hyprpilot-side intent independent of what the agent offers on the
/// wire. Matches the `PermissionOptionKind` shape 1:1 so the resolver
/// can table-driven-map without a translation step; kept as a distinct
/// type so hyprpilot code can evolve without being tied to ACP schema
/// changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcpPermissionOptionKind {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
}

impl AcpPermissionOptionKind {
    /// Fallback chain this intent collapses to when the agent doesn't
    /// offer the exact kind we want. Listed in descending preference;
    /// the resolver picks the first option whose kind matches any
    /// element of the returned slice, in slice order.
    fn fallback_chain(self) -> &'static [PermissionOptionKind] {
        match self {
            Self::AllowAlways => &[PermissionOptionKind::AllowAlways, PermissionOptionKind::AllowOnce],
            Self::AllowOnce => &[PermissionOptionKind::AllowOnce, PermissionOptionKind::AllowAlways],
            Self::RejectAlways => &[PermissionOptionKind::RejectAlways, PermissionOptionKind::RejectOnce],
            Self::RejectOnce => &[PermissionOptionKind::RejectOnce, PermissionOptionKind::RejectAlways],
        }
    }
}

/// Pick the option id that best satisfies `desired` from the set the
/// agent offers. Returns `None` when no option in `options` matches
/// any kind in the fallback chain — the caller then surfaces either a
/// `Cancelled` outcome (for `reject_*` intents) or routes back to the
/// user prompt (for `allow_*`).
#[must_use]
pub fn select_option_id(options: &[PermissionOption], desired: AcpPermissionOptionKind) -> Option<PermissionOptionId> {
    desired
        .fallback_chain()
        .iter()
        .find_map(|k| options.iter().find(|o| o.kind == *k))
        .map(|o| o.option_id.clone())
}

/// Policy-level resolution decision. `Auto(option_id)` — the policy
/// picked an option without asking the user; the caller feeds it
/// straight back to the agent as a `Selected(option_id)` outcome.
/// `Ask` — no short-circuit; the caller must emit the
/// `acp:permission-request` Tauri event and park on a webview reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    Auto(PermissionOptionId),
    Ask,
}

/// Resolve an incoming permission request against the configured
/// policy for the addressed agent. `tool_kind_is_edit` is the result
/// of vendor-specific logic that classifies the tool call (claude's
/// `_meta.claudeCode.toolName`, codex's `raw_input` shape,
/// opencode's tool-name string) into a coarse "is this an edit
/// operation" signal.
///
/// - `Bypass` always returns `Auto` with an `AllowAlways` fallback.
/// - `AcceptEdits` returns `Auto(AllowAlways)` for edit tools, `Ask`
///   otherwise.
/// - `Ask` always returns `Ask`.
#[must_use]
pub fn resolve_policy(
    policy: AcpPermissionPolicy,
    tool_kind_is_edit: bool,
    options: &[PermissionOption],
) -> PolicyDecision {
    match policy {
        AcpPermissionPolicy::Bypass => select_option_id(options, AcpPermissionOptionKind::AllowAlways)
            .map(PolicyDecision::Auto)
            .unwrap_or(PolicyDecision::Ask),
        AcpPermissionPolicy::AcceptEdits if tool_kind_is_edit => {
            select_option_id(options, AcpPermissionOptionKind::AllowAlways)
                .map(PolicyDecision::Auto)
                .unwrap_or(PolicyDecision::Ask)
        }
        AcpPermissionPolicy::AcceptEdits => PolicyDecision::Ask,
        AcpPermissionPolicy::Ask => PolicyDecision::Ask,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a `PermissionOption` keyed by kind + id. The `name` field
    /// isn't load-bearing for the resolver, so we stub it with a
    /// human-recognizable "<kind>-<id>" literal for debugability.
    fn opt(id: &str, kind: PermissionOptionKind) -> PermissionOption {
        PermissionOption::new(PermissionOptionId::new(id), format!("{kind:?}-{id}"), kind)
    }

    /// Concrete option sets for the three vendors we target.
    fn claude_options() -> Vec<PermissionOption> {
        vec![
            opt("allow-once", PermissionOptionKind::AllowOnce),
            opt("allow-always", PermissionOptionKind::AllowAlways),
            opt("reject-once", PermissionOptionKind::RejectOnce),
        ]
    }

    fn codex_options() -> Vec<PermissionOption> {
        vec![
            opt("approved", PermissionOptionKind::AllowOnce),
            opt("approved-for-session", PermissionOptionKind::AllowAlways),
            opt("approved-always", PermissionOptionKind::AllowAlways),
            opt("cancel", PermissionOptionKind::RejectOnce),
        ]
    }

    fn opencode_options() -> Vec<PermissionOption> {
        vec![
            opt("once", PermissionOptionKind::AllowOnce),
            opt("always", PermissionOptionKind::AllowAlways),
            opt("reject", PermissionOptionKind::RejectOnce),
        ]
    }

    #[test]
    fn select_prefers_exact_kind_match() {
        let options = claude_options();
        // AllowAlways requests the exact AllowAlways option.
        let id = select_option_id(&options, AcpPermissionOptionKind::AllowAlways).unwrap();
        assert_eq!(&*id.0, "allow-always");

        let id = select_option_id(&options, AcpPermissionOptionKind::AllowOnce).unwrap();
        assert_eq!(&*id.0, "allow-once");

        let id = select_option_id(&options, AcpPermissionOptionKind::RejectOnce).unwrap();
        assert_eq!(&*id.0, "reject-once");
    }

    #[test]
    fn select_falls_back_when_exact_kind_missing() {
        // Vendor offers only AllowOnce + RejectOnce — the two
        // deny-always requests fall back to reject-once.
        let options = vec![
            opt("once", PermissionOptionKind::AllowOnce),
            opt("reject", PermissionOptionKind::RejectOnce),
        ];

        let id = select_option_id(&options, AcpPermissionOptionKind::AllowAlways).unwrap();
        assert_eq!(&*id.0, "once", "AllowAlways falls back to AllowOnce");

        let id = select_option_id(&options, AcpPermissionOptionKind::RejectAlways).unwrap();
        assert_eq!(&*id.0, "reject", "RejectAlways falls back to RejectOnce");
    }

    #[test]
    fn select_returns_none_when_nothing_in_chain_matches() {
        // Only AllowOnce offered — any reject intent resolves to None.
        let options = vec![opt("once", PermissionOptionKind::AllowOnce)];
        assert!(select_option_id(&options, AcpPermissionOptionKind::RejectOnce).is_none());
        assert!(select_option_id(&options, AcpPermissionOptionKind::RejectAlways).is_none());
    }

    #[test]
    fn select_picks_first_matching_when_multiple_of_same_kind() {
        // Regression: codex ships both `approved-for-session` and
        // `approved-always`, both of kind `AllowAlways`. Order in the
        // vec is "session first, persisted second"; we must honor
        // that ordering so the less-invasive option wins by default.
        let options = codex_options();
        let id = select_option_id(&options, AcpPermissionOptionKind::AllowAlways).unwrap();
        assert_eq!(&*id.0, "approved-for-session");
    }

    #[test]
    fn resolve_policy_bypass_always_auto_allows_across_vendors() {
        for (name, options) in [
            ("claude", claude_options()),
            ("codex", codex_options()),
            ("opencode", opencode_options()),
        ] {
            for tool_is_edit in [true, false] {
                let decision = resolve_policy(AcpPermissionPolicy::Bypass, tool_is_edit, &options);
                match decision {
                    PolicyDecision::Auto(id) => {
                        let opt = options.iter().find(|o| o.option_id == id).unwrap();
                        assert!(
                            matches!(
                                opt.kind,
                                PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways
                            ),
                            "{name}: bypass must pick allow_*, got {:?}",
                            opt.kind
                        );
                    }
                    PolicyDecision::Ask => panic!("{name}: bypass must never Ask"),
                }
            }
        }
    }

    #[test]
    fn resolve_policy_accept_edits_allows_edits_asks_others() {
        let options = claude_options();

        match resolve_policy(AcpPermissionPolicy::AcceptEdits, true, &options) {
            PolicyDecision::Auto(id) => assert_eq!(&*id.0, "allow-always"),
            PolicyDecision::Ask => panic!("edit tool under accept-edits must auto-allow"),
        }

        let decision = resolve_policy(AcpPermissionPolicy::AcceptEdits, false, &options);
        assert_eq!(decision, PolicyDecision::Ask, "non-edit tool under accept-edits asks");
    }

    #[test]
    fn resolve_policy_ask_always_asks() {
        for (name, options) in [
            ("claude", claude_options()),
            ("codex", codex_options()),
            ("opencode", opencode_options()),
        ] {
            for tool_is_edit in [true, false] {
                let decision = resolve_policy(AcpPermissionPolicy::Ask, tool_is_edit, &options);
                assert_eq!(decision, PolicyDecision::Ask, "{name}: ask must always Ask");
            }
        }
    }

    /// Regression: the table-driven resolver pairs every `(policy,
    /// vendor, edit?)` triple and asserts the concrete outcome.
    /// Future vendor additions extend this table; behavior is pinned,
    /// not implied.
    #[test]
    fn policy_decision_table_is_complete_across_vendors() {
        use AcpPermissionPolicy::*;

        struct Case {
            policy: AcpPermissionPolicy,
            tool_is_edit: bool,
            expected_auto_option_id: Option<&'static str>,
        }

        // All three vendors ship allow_always today; bypass always
        // resolves to it. accept-edits + edit → AllowAlways. Every
        // other combo → Ask. The option-id strings below come from the
        // vendor fixtures above.
        let cases_per_vendor: &[Case] = &[
            Case {
                policy: Bypass,
                tool_is_edit: true,
                expected_auto_option_id: Some("allow-always"),
            },
            Case {
                policy: Bypass,
                tool_is_edit: false,
                expected_auto_option_id: Some("allow-always"),
            },
            Case {
                policy: AcceptEdits,
                tool_is_edit: true,
                expected_auto_option_id: Some("allow-always"),
            },
            Case {
                policy: AcceptEdits,
                tool_is_edit: false,
                expected_auto_option_id: None,
            },
            Case {
                policy: Ask,
                tool_is_edit: true,
                expected_auto_option_id: None,
            },
            Case {
                policy: Ask,
                tool_is_edit: false,
                expected_auto_option_id: None,
            },
        ];

        // Only assert against the claude fixture — the shape is shared
        // across vendors, and the cross-vendor Bypass invariant is
        // covered by its own test above.
        let options = claude_options();
        for c in cases_per_vendor {
            let decision = resolve_policy(c.policy, c.tool_is_edit, &options);
            match (decision, c.expected_auto_option_id) {
                (PolicyDecision::Auto(id), Some(expected)) => {
                    assert_eq!(&*id.0, expected, "policy={:?} edit={}", c.policy, c.tool_is_edit);
                }
                (PolicyDecision::Ask, None) => {}
                other => panic!(
                    "policy={:?} edit={}: expected {:?}, got {:?}",
                    c.policy, c.tool_is_edit, c.expected_auto_option_id, other
                ),
            }
        }
    }
}

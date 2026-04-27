//! Closed set of topic strings the `events/subscribe` filter accepts.
//!
//! Two-axis split (per CLAUDE.md "Topic naming — two axes"):
//! - **Tauri event names** use `:` separators (`acp:transcript`).
//! - **Wire topic strings** use `.` separators (`instance.transcript`).
//!
//! `WireTopic` covers the second axis only — the dot-separated names a
//! `ctl events tail` consumer filters against. Instance-bound variants
//! map 1:1 onto `InstanceEvent::topic()`; non-instance variants
//! (`toast.emitted`, `session.loaded`, `skills.changed`, `mcps.changed`,
//! `daemon.reloaded`) are accepted by the validator today but no
//! producer wires events to them yet — the subscribe handler logs a
//! single warn per accepted-but-unwired topic so the gap is loud
//! without rejecting future-dated configs.

use serde::{Deserialize, Serialize};

use crate::adapters::InstanceEvent;

/// Closed set of subscribable topics. Hand-renamed via `#[serde(rename
/// = "...")]` because the wire literals contain dots, which
/// `rename_all` cannot produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WireTopic {
    #[serde(rename = "instances.changed")]
    InstancesChanged,
    #[serde(rename = "instances.focused")]
    InstancesFocused,
    #[serde(rename = "instance.state")]
    InstanceState,
    #[serde(rename = "instance.transcript")]
    InstanceTranscript,
    #[serde(rename = "instance.permission_request")]
    InstancePermissionRequest,
    #[serde(rename = "instance.turn_started")]
    InstanceTurnStarted,
    #[serde(rename = "instance.turn_ended")]
    InstanceTurnEnded,
    /// Issue alias for `instance.transcript` chunks. Maps onto the same
    /// underlying event today; kept distinct on the wire so future
    /// chunk-level filtering can split them.
    #[serde(rename = "transcript.chunk")]
    TranscriptChunk,
    /// Issue alias for `instance.state`.
    #[serde(rename = "state.changed")]
    StateChanged,
    /// Issue alias for `instance.permission_request`.
    #[serde(rename = "permission.requested")]
    PermissionRequested,
    /// Reserved — no producer wires events to these yet. Accepted by
    /// the subscriber API so future toast / session-loaded / skill
    /// reload / MCP reload / daemon reload producers can land without
    /// a wire-protocol bump.
    #[serde(rename = "toast.emitted")]
    ToastEmitted,
    #[serde(rename = "session.loaded")]
    SessionLoaded,
    #[serde(rename = "skills.changed")]
    SkillsChanged,
    #[serde(rename = "mcps.changed")]
    McpsChanged,
    #[serde(rename = "daemon.reloaded")]
    DaemonReloaded,
    /// Per-instance terminal stdout/stderr / exit chunks. Wired to
    /// `InstanceEvent::Terminal` produced by the ACP runtime as the
    /// agent's child processes emit data.
    #[serde(rename = "terminal.output")]
    TerminalOutput,
}

impl WireTopic {
    /// Topics with no producer attached yet — used by the subscribe
    /// handler to emit one `warn!` per accepted-but-unwired request so
    /// gaps stay visible without rejecting forward-dated configs.
    #[must_use]
    pub fn is_unwired(self) -> bool {
        matches!(
            self,
            Self::ToastEmitted | Self::SessionLoaded | Self::SkillsChanged | Self::McpsChanged | Self::DaemonReloaded
        )
    }

    /// True when `event` belongs to this topic (or its issue alias).
    #[must_use]
    pub fn matches(self, event: &InstanceEvent) -> bool {
        matches!(
            (self, event),
            (Self::InstancesChanged, InstanceEvent::InstancesChanged { .. })
                | (Self::InstancesFocused, InstanceEvent::InstancesFocused { .. })
                | (Self::InstanceState | Self::StateChanged, InstanceEvent::State { .. })
                | (
                    Self::InstanceTranscript | Self::TranscriptChunk,
                    InstanceEvent::Transcript { .. }
                )
                | (
                    Self::InstancePermissionRequest | Self::PermissionRequested,
                    InstanceEvent::PermissionRequest { .. }
                )
                | (Self::InstanceTurnStarted, InstanceEvent::TurnStarted { .. })
                | (Self::InstanceTurnEnded, InstanceEvent::TurnEnded { .. })
                | (Self::TerminalOutput, InstanceEvent::Terminal { .. })
        )
    }
}

/// Pull the instance id off the event variants that carry one. Used by
/// the per-subscription filter to honour the `instanceId` scope.
#[must_use]
pub fn event_instance_id(event: &InstanceEvent) -> Option<&str> {
    match event {
        InstanceEvent::State { instance_id, .. }
        | InstanceEvent::Transcript { instance_id, .. }
        | InstanceEvent::PermissionRequest { instance_id, .. }
        | InstanceEvent::TurnStarted { instance_id, .. }
        | InstanceEvent::TurnEnded { instance_id, .. }
        | InstanceEvent::Terminal { instance_id, .. } => Some(instance_id),
        InstanceEvent::InstancesChanged { .. }
        | InstanceEvent::InstancesFocused { .. }
        | InstanceEvent::DaemonReloaded { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    /// Every wire literal round-trips through serde without
    /// re-stating the strings inline.
    #[test]
    fn wire_topic_round_trips_through_serde() {
        let pairs = [
            (WireTopic::InstancesChanged, "instances.changed"),
            (WireTopic::InstancesFocused, "instances.focused"),
            (WireTopic::InstanceState, "instance.state"),
            (WireTopic::InstanceTranscript, "instance.transcript"),
            (WireTopic::InstancePermissionRequest, "instance.permission_request"),
            (WireTopic::InstanceTurnStarted, "instance.turn_started"),
            (WireTopic::InstanceTurnEnded, "instance.turn_ended"),
            (WireTopic::TranscriptChunk, "transcript.chunk"),
            (WireTopic::StateChanged, "state.changed"),
            (WireTopic::PermissionRequested, "permission.requested"),
            (WireTopic::ToastEmitted, "toast.emitted"),
            (WireTopic::SessionLoaded, "session.loaded"),
            (WireTopic::SkillsChanged, "skills.changed"),
            (WireTopic::McpsChanged, "mcps.changed"),
            (WireTopic::DaemonReloaded, "daemon.reloaded"),
            (WireTopic::TerminalOutput, "terminal.output"),
        ];
        for (topic, literal) in pairs {
            let v: Value = serde_json::to_value(topic).expect("serialize");
            assert_eq!(v, json!(literal), "topic literal mismatch for {topic:?}");
            let back: WireTopic = serde_json::from_value(v).expect("deserialize");
            assert_eq!(back, topic);
        }
    }

    #[test]
    fn wire_topic_rejects_unknown_strings() {
        let err = serde_json::from_str::<WireTopic>(r#""bogus.topic""#).expect_err("must reject");
        assert!(err.to_string().to_lowercase().contains("unknown"));
    }

    fn evt_state() -> InstanceEvent {
        InstanceEvent::State {
            agent_id: "a".into(),
            instance_id: "id-1".into(),
            session_id: None,
            state: crate::adapters::InstanceState::Running,
        }
    }

    fn evt_transcript() -> InstanceEvent {
        InstanceEvent::Transcript {
            agent_id: "a".into(),
            instance_id: "id-1".into(),
            session_id: "s".into(),
            turn_id: None,
            item: crate::adapters::TranscriptItem::AgentText { text: "hi".into() },
        }
    }

    fn evt_permission() -> InstanceEvent {
        InstanceEvent::PermissionRequest {
            agent_id: "a".into(),
            instance_id: "id-1".into(),
            session_id: "s".into(),
            turn_id: None,
            request_id: "r-1".into(),
            tool: "fs.read".into(),
            kind: "execute".into(),
            args: "{}".into(),
            options: vec![],
        }
    }

    fn evt_turn_started() -> InstanceEvent {
        InstanceEvent::TurnStarted {
            agent_id: "a".into(),
            instance_id: "id-1".into(),
            session_id: "s".into(),
            turn_id: "t-1".into(),
        }
    }

    fn evt_turn_ended() -> InstanceEvent {
        InstanceEvent::TurnEnded {
            agent_id: "a".into(),
            instance_id: "id-1".into(),
            session_id: "s".into(),
            turn_id: "t-1".into(),
            stop_reason: None,
        }
    }

    fn evt_terminal() -> InstanceEvent {
        InstanceEvent::Terminal {
            agent_id: "a".into(),
            instance_id: "id-1".into(),
            session_id: "s".into(),
            turn_id: None,
            terminal_id: "term-1".into(),
            chunk: crate::adapters::TerminalChunk::Output {
                stream: crate::adapters::TerminalStream::Stdout,
                data: "hello".into(),
            },
        }
    }

    fn evt_instances_changed() -> InstanceEvent {
        InstanceEvent::InstancesChanged {
            instance_ids: vec![],
            focused_id: None,
        }
    }

    fn evt_instances_focused() -> InstanceEvent {
        InstanceEvent::InstancesFocused { instance_id: None }
    }

    /// Each topic + alias pair matches exactly the right
    /// `InstanceEvent` variant; cross-variant matches are rejected.
    #[test]
    #[allow(clippy::type_complexity)]
    fn wire_topic_matches_correct_instance_event_variant() {
        let cases: &[(WireTopic, fn() -> InstanceEvent)] = &[
            (WireTopic::InstanceState, evt_state),
            (WireTopic::StateChanged, evt_state),
            (WireTopic::InstanceTranscript, evt_transcript),
            (WireTopic::TranscriptChunk, evt_transcript),
            (WireTopic::InstancePermissionRequest, evt_permission),
            (WireTopic::PermissionRequested, evt_permission),
            (WireTopic::InstanceTurnStarted, evt_turn_started),
            (WireTopic::InstanceTurnEnded, evt_turn_ended),
            (WireTopic::TerminalOutput, evt_terminal),
            (WireTopic::InstancesChanged, evt_instances_changed),
            (WireTopic::InstancesFocused, evt_instances_focused),
        ];
        for (topic, mk) in cases {
            assert!(topic.matches(&mk()), "topic {topic:?} must match its variant");
        }

        assert!(!WireTopic::InstanceState.matches(&evt_transcript()));
        assert!(!WireTopic::InstanceTranscript.matches(&evt_state()));
        assert!(!WireTopic::TranscriptChunk.matches(&evt_state()));
        assert!(!WireTopic::InstancesChanged.matches(&evt_state()));
        assert!(!WireTopic::ToastEmitted.matches(&evt_state()));
    }

    #[test]
    fn unwired_topics_advertise_themselves() {
        for topic in [
            WireTopic::ToastEmitted,
            WireTopic::SessionLoaded,
            WireTopic::SkillsChanged,
            WireTopic::McpsChanged,
            WireTopic::DaemonReloaded,
        ] {
            assert!(topic.is_unwired(), "{topic:?} must report unwired");
        }
        for topic in [
            WireTopic::InstanceState,
            WireTopic::InstanceTranscript,
            WireTopic::TerminalOutput,
        ] {
            assert!(!topic.is_unwired());
        }
    }

    #[test]
    fn event_instance_id_extracts_per_variant_id() {
        assert_eq!(event_instance_id(&evt_state()), Some("id-1"));
        assert_eq!(event_instance_id(&evt_transcript()), Some("id-1"));
        assert_eq!(event_instance_id(&evt_permission()), Some("id-1"));
        assert_eq!(event_instance_id(&evt_turn_started()), Some("id-1"));
        assert_eq!(event_instance_id(&evt_turn_ended()), Some("id-1"));
        assert_eq!(event_instance_id(&evt_terminal()), Some("id-1"));
        assert!(event_instance_id(&evt_instances_changed()).is_none());
        assert!(event_instance_id(&evt_instances_focused()).is_none());
    }
}

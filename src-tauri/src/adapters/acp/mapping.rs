//! `From` / `TryFrom` bridges between ACP wire DTOs and the generic
//! `adapters::*` vocabulary. Every conversion point where an
//! ACP-shaped value escapes `adapters::acp` goes through this file —
//! the rest of the crate never touches an ACP DTO directly.
//!
//! The bridge projects the ACP-side event + state + bootstrap enums
//! onto the generic `adapters::instance` vocabulary, plus
//! `AcpPermissionOptionView → PermissionOptionView`. Runtime emits raw
//! `SessionUpdate` JSON onto `InstanceEvent::Transcript.update`
//! rather than projecting into `TranscriptItem` variants. Future
//! typed-transcript mappings slot in here when we need typed
//! projection on the Rust side (today the webview does the variant
//! dispatch).

use crate::adapters::instance::{InstanceEvent as GenericInstanceEvent, InstanceState as GenericInstanceState};
use crate::adapters::permission::PermissionOptionView;
use crate::adapters::Bootstrap as GenericBootstrap;

use super::client::PermissionOptionView as AcpPermissionOptionView;
use super::runtime::{Bootstrap as AcpBootstrap, InstanceEvent as AcpInstanceEvent, InstanceState as AcpInstanceState};

impl From<AcpPermissionOptionView> for PermissionOptionView {
    fn from(v: AcpPermissionOptionView) -> Self {
        Self {
            option_id: v.option_id,
            name: v.name,
            kind: v.kind,
        }
    }
}

impl From<AcpInstanceState> for GenericInstanceState {
    fn from(s: AcpInstanceState) -> Self {
        match s {
            AcpInstanceState::Starting => GenericInstanceState::Starting,
            AcpInstanceState::Running => GenericInstanceState::Running,
            AcpInstanceState::Ended => GenericInstanceState::Ended,
            AcpInstanceState::Error => GenericInstanceState::Error,
        }
    }
}

impl From<AcpInstanceEvent> for GenericInstanceEvent {
    fn from(e: AcpInstanceEvent) -> Self {
        match e {
            AcpInstanceEvent::State {
                agent_id,
                instance_id,
                session_id,
                state,
            } => GenericInstanceEvent::State {
                agent_id,
                instance_id,
                session_id,
                state: state.into(),
            },
            AcpInstanceEvent::Transcript {
                agent_id,
                instance_id,
                session_id,
                update,
            } => GenericInstanceEvent::Transcript {
                agent_id,
                instance_id,
                session_id,
                update,
            },
            AcpInstanceEvent::PermissionRequest {
                agent_id,
                instance_id,
                session_id,
                request_id,
                tool,
                kind,
                args,
                options,
            } => GenericInstanceEvent::PermissionRequest {
                agent_id,
                instance_id,
                session_id,
                request_id,
                tool,
                kind,
                args,
                options: options.into_iter().map(Into::into).collect(),
            },
            AcpInstanceEvent::InstancesChanged {
                instance_ids,
                focused_id,
            } => GenericInstanceEvent::InstancesChanged {
                instance_ids,
                focused_id,
            },
            AcpInstanceEvent::InstancesFocused { instance_id } => {
                GenericInstanceEvent::InstancesFocused { instance_id }
            }
        }
    }
}

/// Generic → ACP bootstrap. The generic layer owns the public enum;
/// `AcpAdapter::start_instance` accepts the generic variant and maps
/// here before handing to the runtime.
impl From<GenericBootstrap> for AcpBootstrap {
    fn from(b: GenericBootstrap) -> Self {
        match b {
            GenericBootstrap::Fresh => AcpBootstrap::Fresh,
            GenericBootstrap::Resume(id) => AcpBootstrap::Resume(agent_client_protocol::schema::SessionId::new(id)),
            GenericBootstrap::ListOnly => AcpBootstrap::ListOnly,
        }
    }
}

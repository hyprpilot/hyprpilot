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

use agent_client_protocol::schema::{
    ContentBlock, EmbeddedResource, EmbeddedResourceResource, TextContent, TextResourceContents,
};

use super::runtime::{Bootstrap as AcpBootstrap, InstanceEvent as AcpInstanceEvent, InstanceState as AcpInstanceState};
use crate::adapters::instance::{InstanceEvent as GenericInstanceEvent, InstanceState as GenericInstanceState};
use crate::adapters::transcript::Attachment;
use crate::adapters::Bootstrap as GenericBootstrap;

/// Build the ACP `Vec<ContentBlock>` payload for one user turn.
/// Attachments project as `EmbeddedResource` (text/markdown today)
/// and prepend the prose text block — order matters: the agent
/// reads context before instructions.
pub(crate) fn build_prompt_blocks(text: &str, attachments: &[Attachment]) -> Vec<ContentBlock> {
    let mut blocks: Vec<ContentBlock> = Vec::with_capacity(attachments.len() + 1);
    for att in attachments {
        let mut tr = TextResourceContents::new(att.body.clone(), format!("file://{}", att.path.display()));
        tr.mime_type = Some("text/markdown".into());
        blocks.push(ContentBlock::Resource(EmbeddedResource::new(
            EmbeddedResourceResource::TextResourceContents(tr),
        )));
    }
    blocks.push(ContentBlock::Text(TextContent::new(text.to_owned())));
    blocks
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
                turn_id,
                update,
            } => GenericInstanceEvent::Transcript {
                agent_id,
                instance_id,
                session_id,
                turn_id,
                update,
            },
            AcpInstanceEvent::PermissionRequest {
                agent_id,
                instance_id,
                session_id,
                turn_id,
                request_id,
                tool,
                kind,
                args,
                options,
            } => GenericInstanceEvent::PermissionRequest {
                agent_id,
                instance_id,
                session_id,
                turn_id,
                request_id,
                tool,
                kind,
                args,
                options,
            },
            AcpInstanceEvent::TurnStarted {
                agent_id,
                instance_id,
                session_id,
                turn_id,
            } => GenericInstanceEvent::TurnStarted {
                agent_id,
                instance_id,
                session_id,
                turn_id,
            },
            AcpInstanceEvent::TurnEnded {
                agent_id,
                instance_id,
                session_id,
                turn_id,
                stop_reason,
            } => GenericInstanceEvent::TurnEnded {
                agent_id,
                instance_id,
                session_id,
                turn_id,
                stop_reason,
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn build_prompt_blocks_emits_only_text_when_no_attachments() {
        let blocks = build_prompt_blocks("hello", &[]);
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            ContentBlock::Text(t) => assert_eq!(t.text, "hello"),
            other => panic!("expected text block, got {other:?}"),
        }
    }

    #[test]
    fn build_prompt_blocks_prepends_resources_before_text() {
        let att = Attachment {
            slug: "git-commit".into(),
            path: PathBuf::from("/tmp/skills/git-commit/SKILL.md"),
            body: "stage and commit".into(),
            title: Some("Git commit".into()),
        };
        let blocks = build_prompt_blocks("please commit", std::slice::from_ref(&att));
        assert_eq!(blocks.len(), 2, "one resource + one text");
        let ContentBlock::Resource(res) = &blocks[0] else {
            panic!("first block must be Resource");
        };
        let EmbeddedResourceResource::TextResourceContents(tr) = &res.resource else {
            panic!("resource must carry text contents");
        };
        assert_eq!(tr.uri, "file:///tmp/skills/git-commit/SKILL.md");
        assert_eq!(tr.mime_type.as_deref(), Some("text/markdown"));
        assert_eq!(tr.text, "stage and commit");
        match &blocks[1] {
            ContentBlock::Text(t) => assert_eq!(t.text, "please commit"),
            other => panic!("second block must be text, got {other:?}"),
        }
    }

    #[test]
    fn build_prompt_blocks_preserves_attachment_order() {
        let a = Attachment {
            slug: "a".into(),
            path: PathBuf::from("/tmp/a/SKILL.md"),
            body: "A".into(),
            title: None,
        };
        let b = Attachment {
            slug: "b".into(),
            path: PathBuf::from("/tmp/b/SKILL.md"),
            body: "B".into(),
            title: None,
        };
        let blocks = build_prompt_blocks("text", &[a, b]);
        assert_eq!(blocks.len(), 3);
        let ContentBlock::Resource(first) = &blocks[0] else {
            panic!()
        };
        let EmbeddedResourceResource::TextResourceContents(tr0) = &first.resource else {
            panic!()
        };
        assert_eq!(tr0.text, "A");
        let ContentBlock::Resource(second) = &blocks[1] else {
            panic!()
        };
        let EmbeddedResourceResource::TextResourceContents(tr1) = &second.resource else {
            panic!()
        };
        assert_eq!(tr1.text, "B");
    }
}

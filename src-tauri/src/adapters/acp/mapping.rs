//! `From` / `TryFrom` bridges between ACP wire DTOs and the generic
//! `adapters::*` vocabulary. Every conversion point where an
//! ACP-shaped value escapes `adapters::acp` goes through this file —
//! the rest of the crate never touches an ACP DTO directly.
//!
//! Today the bridge is minimal — `AcpPermissionOption → PermissionOptionView`
//! is the only mapping that ships, since the runtime emits raw
//! `SessionUpdate` JSON onto `InstanceEvent::Transcript.update`
//! rather than projecting into `TranscriptItem` variants. Future
//! mappings slot in here when we need typed transcript projection on
//! the Rust side (today the webview does the variant dispatch).

use crate::adapters::permission::PermissionOptionView;

use super::client::PermissionOptionView as AcpPermissionOptionView;

impl From<AcpPermissionOptionView> for PermissionOptionView {
    fn from(v: AcpPermissionOptionView) -> Self {
        Self {
            option_id: v.option_id,
            name: v.name,
            kind: v.kind,
        }
    }
}

//! Generic permission-prompt vocabulary. The adapter emits a
//! `PermissionPrompt` via `InstanceEvent::PermissionRequest`; the UI
//! replies with a `PermissionReply { option_id }`. Today the ACP
//! runtime auto-`Cancelled`s every request (see CLAUDE.md §Permissions
//! are the vendor's concern); the K-245 `PermissionController` lands
//! a real trust-store behind this trait.

use serde::{Deserialize, Serialize};

/// UI-facing projection of a permission option. Wire-normalised so
/// the webview doesn't need to speak any specific vendor's shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionOptionView {
    pub option_id: String,
    pub name: String,
    /// Normalised wire name: `"allow_once" | "allow_always" |
    /// "reject_once" | "reject_always"` today. Closed set once the
    /// crate's upstream enum stabilises; `String` keeps the UI
    /// tolerant to new-variant drift today.
    pub kind: String,
}

/// A request the adapter fans out to the webview via
/// `acp:permission-request`. Carries the options + the identity bits
/// needed to route the reply back to the awaiting actor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionPrompt {
    pub session_id: String,
    pub request_id: String,
    pub options: Vec<PermissionOptionView>,
}

/// The UI's answer back. `PermissionController` (K-245) threads these
/// through the adapter so the awaiting actor resumes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionReply {
    pub session_id: String,
    pub request_id: String,
    pub option_id: String,
}

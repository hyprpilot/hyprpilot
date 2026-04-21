//! Agent Client Protocol bridge.
//!
//! Module layout (per the K-239 spec):
//!
//! - `permissions` — policy-to-option-id resolver + fallback chain.
//!   The one piece that's purely data-driven and doesn't need a live
//!   ACP connection; fully covered by unit tests.
//! - `agents` — per-vendor adapter structs (claude-code-acp,
//!   codex-acp, opencode), each encoding the vendor's launch command,
//!   permission option subset, and tool-content quirks. Only the type
//!   shapes land today; the vendor-specific `render_update` bodies
//!   come with the live session plumbing.
//! - `session`, `client`, `spawn`, `commands` — the ACP runtime.
//!   Trait signatures are sketched; the wire-level wiring against
//!   `agent-client-protocol = "0.11"`'s `Client.builder()` +
//!   `ConnectionTo<Agent>` pattern lands with the live-session
//!   follow-up.
//!
//! Today `AcpSessions::submit` / `cancel` / `info` are stubs that
//! return the same shapes the pre-K-239 `CoreHandler` did. The
//! `SessionHandler` in `rpc::handlers` dispatches into them so the
//! wire surface is stable across the build-out.
//!
//! The `#[allow(dead_code)]` on the re-exports below is intentional:
//! the types are exercised by unit tests today and by the live
//! session plumbing in the next commit. Removing the gate would
//! force us to either half-wire each piece prematurely or silence
//! warnings per-item, both worse trade-offs than the crate-wide
//! opt-out on this scaffold module.

#[allow(dead_code)]
pub mod agents;
#[allow(dead_code)]
pub mod permissions;
pub mod session;

#[allow(unused_imports)]
pub use agents::AcpAgent;
#[allow(unused_imports)]
pub use permissions::{resolve_policy, select_option_id, AcpPermissionOptionKind, PolicyDecision};
#[allow(unused_imports)]
pub use session::{AcpSessionState, AcpSessions};

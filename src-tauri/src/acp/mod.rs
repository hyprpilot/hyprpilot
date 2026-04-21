//! Agent Client Protocol bridge.
//!
//! Module layout:
//!
//! - `agents` — per-vendor adapter structs (claude-code-acp,
//!   codex-acp, opencode), each encoding the vendor's launch command
//!   and tool-content quirks. Only the type shapes land today; the
//!   vendor-specific `render_update` bodies come with the live
//!   session plumbing.
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
//! Permission handling is deliberately *not* a concern of this
//! module. The ACP protocol has no policy layer, and each vendor now
//! ships its own plan/build modes + granular permission controls
//! (claude-code-acp's plan mode, codex-acp's approval modes,
//! opencode's tool filters). Hyprpilot forwards every
//! `session/request_permission` straight to the webview as an
//! `acp:permission-request` Tauri event; the user picks an option.
//! Client-side auto-accept / auto-reject rules (per-tool allowlists,
//! trust store) are the scope of a future `PermissionController`
//! issue, not this scaffold.
//!
//! The `#[allow(dead_code)]` on the re-exports below is intentional:
//! the types are exercised by unit tests today and by the live
//! session plumbing in the next commit. Removing the gate would
//! force us to either half-wire each piece prematurely or silence
//! warnings per-item, both worse trade-offs than the crate-wide
//! opt-out on this scaffold module.

#[allow(dead_code)]
pub mod agents;
pub mod session;

#[allow(unused_imports)]
pub use agents::AcpAgent;
#[allow(unused_imports)]
pub use session::{AcpSessionState, AcpSessions};

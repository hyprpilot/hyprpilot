//! Agent Client Protocol bridge.
//!
//! Live runtime active. One `tokio::spawn`-ed actor per session
//! (`runtime::run_session`) owns the child + `ConnectionTo<Agent>`;
//! `AcpSessions` is the Tauri-managed registry of those actors.
//! `session/submit` spawns the vendor on first hit then reuses the
//! live session for follow-up prompts.
//!
//! `ClientSideConnection` is not a thing in `agent-client-protocol` 0.11
//! — the crate uses `Client.builder().connect_with(transport, main_fn)`.
//! `Send`-safe throughout; no `LocalSet` required.
//!
//! Permission handling is auto-`Cancelled` today; the
//! `PermissionController` issue replaces the auto-deny with a real
//! trust-store. Every `session/request_permission` still reaches the
//! webview as `acp:permission-request` for observability.

pub mod agents;
pub mod client;
pub mod commands;
pub mod resolve;
pub mod runtime;
pub mod session;
pub mod spawn;

pub use session::AcpSessions;

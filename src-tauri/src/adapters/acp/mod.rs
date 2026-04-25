//! Agent Client Protocol adapter.
//!
//! Live runtime active. One `tokio::spawn`-ed actor per instance
//! (`runtime::run_instance`) owns the child + `ConnectionTo<Agent>`;
//! `AcpAdapter` holds the `AdapterRegistry<AcpInstance>` + the
//! ACP-specific glue (resolve / spawn / submit / cancel / list).
//! `session/submit` spawns the vendor on first hit then reuses the
//! live session for follow-up prompts.
//!
//! `ClientSideConnection` is not a thing in `agent-client-protocol` 0.11
//! — the crate uses `Client.builder().connect_with(transport, main_fn)`.
//! `Send`-safe throughout; no `LocalSet` required.
//!
//! Permission handling routes through `DefaultPermissionController`:
//! profile reject/accept globs resolve without UI traffic, and only
//! the `AskUser` path reaches the webview as `acp:permission-request`.
//! The call-site wraps the waiter's `rx.await` in
//! `tokio::time::timeout(WAITER_TIMEOUT, rx)` so a prompt left
//! unanswered for 10 minutes falls through to `Cancelled` without
//! wedging the ACP session.
//!
//! Layering: nothing outside `src-tauri/src/adapters/` may
//! `use crate::adapters::acp::*`. That's a layering violation — the
//! rest of the crate talks to `dyn Adapter` or to the concrete
//! `AcpAdapter` (re-exported at the adapter root).

pub mod agents;
pub mod client;
pub mod instance;
pub mod instances;
pub mod mapping;
pub mod runtime;
pub mod spawn;

pub use instances::AcpAdapter;

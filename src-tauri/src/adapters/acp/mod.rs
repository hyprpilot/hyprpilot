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

pub use instances::AcpAdapter;

use once_cell::sync::Lazy;

use crate::tools::formatter::registry::FormatterRegistry;

/// Process-wide formatter registry — shared by every ACP actor's
/// notification task. Built once at first access; the registry is
/// stateless after construction so a single shared `&FormatterRegistry`
/// is safe across actors.
static FORMATTERS: Lazy<FormatterRegistry> = Lazy::new(crate::tools::formatter::build_default_registry);

pub(crate) fn formatter_registry() -> &'static FormatterRegistry {
    &FORMATTERS
}

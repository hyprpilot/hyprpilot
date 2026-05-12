//! Tauri `#[command]` surface for the composer-autocomplete dropdown.
//! Mirrors the JSON-RPC `completion/{query,resolve,cancel,rank}`
//! methods so the in-process webview can `invoke()` directly without
//! going through the socket. Both call paths share the same
//! `CompletionRegistry` + `CompletionCancellations` from managed
//! state, and both route through `completion::dispatch::run_*` so
//! detection / cancellation / ranking semantics can't drift between
//! the in-process and JSON-RPC transports.

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use serde_json::{json, Value};
use tauri::State;

use crate::completion::dispatch;
use crate::completion::{CompletionCancellations, CompletionRegistry};
use crate::config::Config;

type RegistryState<'a> = State<'a, Arc<CompletionRegistry>>;
type CancellationsState<'a> = State<'a, Arc<CompletionCancellations>>;

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn completion_query(
    registry: RegistryState<'_>,
    cancellations: CancellationsState<'_>,
    text: String,
    cursor: usize,
    cwd: Option<PathBuf>,
    manual: Option<bool>,
    // `instance_id` on the wire — currently unused on the daemon side
    // (the registry's `detect` is instance-agnostic). Kept on the
    // wire so the UI doesn't need to gate on it; future instance-
    // scoped sources land here.
    #[allow(unused_variables)] instance_id: Option<String>,
    // Whitelist of source ids (`["path"]`) to consider. When `Some`,
    // sources whose id isn't in the list are skipped during detect.
    // Drives palette modes wanting a single source — cwd palette
    // passes `["path"]`.
    sources: Option<Vec<String>>,
) -> Result<Value, String> {
    let manual = manual.unwrap_or(false);
    tracing::trace!(
        text_len = text.len(),
        cursor,
        manual,
        sources = ?sources,
        "cmd::completion_query"
    );
    dispatch::run_query(
        registry.inner(),
        cancellations.inner(),
        &text,
        cursor,
        cwd.as_deref(),
        manual,
        sources.as_deref(),
    )
    .await
    .map_err(|e| e.message)
}

#[tauri::command]
pub async fn completion_resolve(
    registry: RegistryState<'_>,
    resolve_id: String,
    source_id: String,
) -> Result<Value, String> {
    dispatch::run_resolve(registry.inner(), &resolve_id, &source_id)
        .await
        .map_err(|e| e.message)
}

#[tauri::command]
pub async fn completion_cancel(cancellations: CancellationsState<'_>, request_id: String) -> Result<Value, String> {
    Ok(dispatch::run_cancel(cancellations.inner(), &request_id))
}

/// Rank `candidates` against `query` via the candidates source.
/// Distinct from `completion/query`: discovery sources walk the
/// world to find candidates; this one ranks a caller-supplied
/// list. Same `CompletionItem[]` output shape so the popover
/// state machine doesn't branch.
#[tauri::command]
pub async fn completion_rank(
    query: String,
    candidates: Vec<crate::completion::source::candidates::CandidateItem>,
) -> Result<Value, String> {
    Ok(dispatch::run_rank(&query, &candidates))
}

/// Snapshot of the captain's `[completion]` config block. UI reads
/// this at boot to apply the ripgrep auto-trigger debounce — the
/// daemon-side source already honours `auto` / `min_prefix`, but
/// debounce lives client-side because that's where keystrokes
/// happen.
#[tauri::command]
pub async fn get_completion_config(config: State<'_, Arc<RwLock<Config>>>) -> Result<Value, String> {
    let cfg = config.read().map_err(|e| format!("config rwlock poisoned: {e}"))?;
    let rg = &cfg.completion.ripgrep;
    Ok(json!({
        "ripgrep": {
            "auto": rg.auto.unwrap_or(true),
            "debounceMs": rg.debounce_ms.unwrap_or(250),
            "minPrefix": rg.min_prefix.unwrap_or(3),
        }
    }))
}

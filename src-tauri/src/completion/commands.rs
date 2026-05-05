//! Tauri `#[command]` surface for the composer-autocomplete dropdown.
//! Mirrors the JSON-RPC `completion/{query,resolve,cancel}` methods
//! so the webview can `invoke()` directly without going through the
//! socket. Both call paths share the same `CompletionRegistry` +
//! `CompletionCancellations` from managed state.

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use serde_json::{json, Value};
use tauri::State;

use crate::completion::{CompletionCancellations, CompletionRegistry, ReplacementRange};
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
    // `_instance_id` on the wire — Tauri infers `instanceId` (camelCase)
    // for the JS invoke shape. Currently unused on the daemon side
    // (the registry's `detect` is instance-agnostic) but kept on the
    // wire for forward-compat.
    #[allow(unused_variables)] instance_id: Option<String>,
    // Whitelist of source ids (`["path"]`) to consider. When `Some`,
    // sources whose id isn't in the list are skipped during detect.
    // Drives palette modes wanting a single source — cwd palette
    // passes `["path"]`.
    sources: Option<Vec<String>>,
) -> Result<Value, String> {
    let _ = instance_id;
    let request_id = uuid::Uuid::new_v4().to_string();
    let manual = manual.unwrap_or(false);
    tracing::trace!(
        request_id,
        text_len = text.len(),
        cursor,
        manual,
        sources = ?sources,
        "cmd::completion_query"
    );

    let detected = registry.detect_filtered(&text, cursor, manual, sources.as_deref());
    let (source, ctx) = match detected {
        Some(d) => d,
        None => {
            return Ok(json!({
                "requestId": request_id,
                "sourceId": null,
                "replacementRange": null,
                "items": [],
            }));
        }
    };

    let cancel = cancellations.new_token(&request_id);
    let range = ReplacementRange {
        start: ctx.trigger_offset,
        end: ctx.cursor,
    };
    let source_id = source.id();
    // Forget unconditionally — leaving the token in the table on error
    // would make a follow-up `completion/cancel` look like a stale hit.
    let result = source.fetch(ctx, cwd.as_deref(), cancel).await;
    cancellations.forget(&request_id);
    let items = result.map_err(|e| format!("completion/query: {e}"))?;

    Ok(json!({
        "requestId": request_id,
        "sourceId": source_id,
        "replacementRange": range,
        "items": items,
    }))
}

#[tauri::command]
pub async fn completion_resolve(
    registry: RegistryState<'_>,
    resolve_id: String,
    source_id: String,
) -> Result<Value, String> {
    let source = registry
        .source_by_id(&source_id)
        .ok_or_else(|| format!("unknown source_id: {source_id}"))?;
    let documentation = source
        .resolve(&resolve_id)
        .await
        .map_err(|e| format!("completion/resolve: {e}"))?;
    Ok(json!({ "documentation": documentation }))
}

#[tauri::command]
pub async fn completion_cancel(cancellations: CancellationsState<'_>, request_id: String) -> Result<Value, String> {
    let cancelled = cancellations.cancel(&request_id);
    Ok(json!({ "cancelled": cancelled }))
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
    let request_id = uuid::Uuid::new_v4().to_string();
    let items = crate::completion::source::candidates::rank_candidates(&query, &candidates);
    Ok(json!({
        "requestId": request_id,
        "sourceId": "candidates",
        "replacementRange": null,
        "items": items,
    }))
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

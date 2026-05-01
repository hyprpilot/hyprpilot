//! Tauri `#[command]` surface for the composer-autocomplete dropdown.
//! Mirrors the JSON-RPC `completion/{query,resolve,cancel}` methods
//! so the webview can `invoke()` directly without going through the
//! socket. Both call paths share the same `CompletionRegistry` +
//! `CompletionCancellations` from managed state.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};
use tauri::State;

use crate::completion::{CompletionCancellations, CompletionRegistry, ReplacementRange};

type RegistryState<'a> = State<'a, Arc<CompletionRegistry>>;
type CancellationsState<'a> = State<'a, Arc<CompletionCancellations>>;

#[tauri::command]
pub async fn completion_query(
    registry: RegistryState<'_>,
    cancellations: CancellationsState<'_>,
    text: String,
    cursor: usize,
    cwd: Option<PathBuf>,
    manual: Option<bool>,
    #[allow(non_snake_case)] _instanceId: Option<String>,
) -> Result<Value, String> {
    let request_id = uuid::Uuid::new_v4().to_string();
    let manual = manual.unwrap_or(false);

    let detected = registry.detect(&text, cursor, manual);
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
    let items = source
        .fetch(ctx, cwd.as_deref(), cancel)
        .await
        .map_err(|e| format!("completion/query: {e}"))?;
    cancellations.forget(&request_id);

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
    #[allow(non_snake_case)] resolveId: String,
    #[allow(non_snake_case)] sourceId: String,
) -> Result<Value, String> {
    let source = registry
        .source_by_id(&sourceId)
        .ok_or_else(|| format!("unknown source_id: {sourceId}"))?;
    let documentation = source
        .resolve(&resolveId)
        .await
        .map_err(|e| format!("completion/resolve: {e}"))?;
    Ok(json!({ "documentation": documentation }))
}

#[tauri::command]
pub async fn completion_cancel(
    cancellations: CancellationsState<'_>,
    #[allow(non_snake_case)] requestId: String,
) -> Result<Value, String> {
    let cancelled = cancellations.cancel(&requestId);
    Ok(json!({ "cancelled": cancelled }))
}

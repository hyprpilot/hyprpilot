//! Transport-agnostic completion dispatch helpers. The composer
//! autocomplete surface lives on three call paths:
//!
//! 1. Tauri `#[command]`s in `completion::commands` — direct
//!    invocation from the in-process webview (desktop SPA).
//! 2. JSON-RPC `tauri/completion_*` arms in `rpc::handlers::tauri_proxy` —
//!    same surface routed over the WS bridge for the remote SPA.
//! 3. JSON-RPC `completion/*` arms in `rpc::handlers::completion` —
//!    public namespace consumed by non-SPA clients (nvim, ctl, future
//!    remotes).
//!
//! All three call into the same `run_*` async fns here so a fix to
//! detection / cancellation / ranking semantics can't silently drift
//! between transports. Return type is `Result<Value, RpcError>` —
//! the Tauri command path adapts to `Result<Value, String>` at its
//! own edge.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::completion::source::candidates::{rank_candidates, CandidateItem};
use crate::completion::{CompletionCancellations, CompletionRegistry, ReplacementRange};
use crate::rpc::protocol::RpcError;

/// `completion/query` — walk the registry for a source that claims
/// the cursor position, then run its `fetch`. Returns the standard
/// `{ requestId, sourceId, replacementRange, items }` envelope.
/// `sourceId: null` + `items: []` means no source matched (the
/// typed query didn't hit any trigger).
pub async fn run_query(
    registry: &Arc<CompletionRegistry>,
    cancellations: &Arc<CompletionCancellations>,
    text: &str,
    cursor: usize,
    cwd: Option<&std::path::Path>,
    manual: bool,
    sources: Option<&[String]>,
) -> Result<Value, RpcError> {
    let request_id = uuid::Uuid::new_v4().to_string();

    let detected = registry.detect_filtered(text, cursor, manual, sources);
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
    let result = source.fetch(ctx, cwd, cancel).await;
    // Forget unconditionally — leaving the token in the table on
    // error makes a follow-up cancel look like a stale hit.
    cancellations.forget(&request_id);
    let items = result.map_err(|e| RpcError::internal_error(format!("completion/query: {e}")))?;

    Ok(json!({
        "requestId": request_id,
        "sourceId": source_id,
        "replacementRange": range,
        "items": items,
    }))
}

/// `completion/resolve` — lazy documentation fetch for a previously
/// returned item. `documentation: null` (or empty string) means the
/// source had nothing to add.
pub async fn run_resolve(
    registry: &Arc<CompletionRegistry>,
    resolve_id: &str,
    source_id: &str,
) -> Result<Value, RpcError> {
    let source = registry
        .source_by_id(source_id)
        .ok_or_else(|| RpcError::invalid_params(format!("unknown source_id: {source_id}")))?;
    let documentation = source
        .resolve(resolve_id)
        .await
        .map_err(|e| RpcError::internal_error(format!("completion/resolve: {e}")))?;
    Ok(json!({ "documentation": documentation }))
}

/// `completion/cancel` — best-effort. Sources check the token
/// cooperatively; the bool reports whether the token was found, not
/// whether the underlying work actually halted.
pub fn run_cancel(cancellations: &Arc<CompletionCancellations>, request_id: &str) -> Value {
    let cancelled = cancellations.cancel(request_id);
    json!({ "cancelled": cancelled })
}

/// `completion/rank` — rank a caller-supplied candidate list via the
/// candidates source. Distinct from `completion/query` (which
/// discovers candidates by walking the world); this one only ranks
/// what the caller already has. Same `CompletionItem[]` output
/// shape so the popover state machine doesn't branch.
pub fn run_rank(query: &str, candidates: &[CandidateItem]) -> Value {
    let request_id = uuid::Uuid::new_v4().to_string();
    let items = rank_candidates(query, candidates);
    json!({
        "requestId": request_id,
        "sourceId": "candidates",
        "replacementRange": null,
        "items": items,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::completion::source::candidates::CandidateItem;

    #[test]
    fn rank_with_empty_query_preserves_order() {
        let cands = vec![
            CandidateItem {
                id: "alpha".into(),
                label: "alpha".into(),
                description: None,
            },
            CandidateItem {
                id: "beta".into(),
                label: "beta".into(),
                description: None,
            },
        ];
        let out = run_rank("", &cands);
        let items = out["items"].as_array().expect("items array");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["label"], "alpha");
        assert_eq!(items[1]["label"], "beta");
        assert_eq!(out["sourceId"], "candidates");
        assert!(out["replacementRange"].is_null());
    }

    #[tokio::test]
    async fn cancel_unknown_request_returns_false() {
        let cancellations = Arc::new(CompletionCancellations::default());
        let v = run_cancel(&cancellations, "ghost-request");
        assert_eq!(v["cancelled"], false);
    }

    #[tokio::test]
    async fn cancel_known_request_returns_true() {
        let cancellations = Arc::new(CompletionCancellations::default());
        let _token = cancellations.new_token("req-1");
        let v = run_cancel(&cancellations, "req-1");
        assert_eq!(v["cancelled"], true);
    }
}

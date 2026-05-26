//! Desktop integration — signals the overlay reads from the user's
//! environment that aren't part of `[ui]` config. Used to ferry
//! `$HOME` across to the webview, but every cwd field on the wire
//! is now display-formatted server-side (`tools::path::display_cwd`)
//! so the frontend never needs to know `$HOME` to do its own
//! collapse — the dumb-display invariant that lets multi-frontend
//! designs (Vue overlay + future Neovim plugin + `ctl --json`)
//! render the same shape verbatim.
//!
//! Today: file-attachment hydrator + raw-text
//! captain-typed-path resolver. Future XDG paths, monitor scale
//! overrides, and other desktop-environment knobs land here when
//! they earn their slot.

/// Tauri shell over the file-attachment hydrator. Implementation
/// lives at `completion::hydration::file::read_file_for_attachment` —
/// it pairs with `completion::source::path::PathSource` (sources
/// detect the path pattern at compose time, this hydrator resolves
/// the picked path into the wire-side attachment body).
#[tauri::command]
pub(crate) async fn read_file_for_attachment(path: String) -> Result<serde_json::Value, String> {
    crate::completion::hydration::file::read_file_for_attachment(path).await
}

/// Captain-typed → absolute resolution. Returns `None` when the
/// input is empty or relative-with-no-cwd-base. The webview can't
/// `${VAR}`-expand without OS access, so the daemon owns the
/// resolution path; UI-side display niceties (home → `~`
/// substitution, CSS truncation) stay client-side.
///
/// Signature is the flat `(raw, cwdBase)` pair, NOT a wrapping
/// `PathsResolveArgs` struct — Tauri's command-arg binding
/// projects struct params as `{ <paramName>: { ... } }` on the
/// JSON-RPC wire, so a struct here would force the UI to send
/// `{ args: { raw, cwdBase } }`. Flat params bind directly to
/// `{ raw, cwdBase }`, matching every other Tauri command in the
/// codebase.
#[tauri::command]
pub(crate) fn paths_resolve(raw: String, cwd_base: Option<String>) -> Result<Option<String>, String> {
    let home = crate::paths::home_dir();
    let home_str = home.to_string_lossy();
    Ok(crate::tools::path::resolve_absolute(
        &raw,
        &home_str,
        cwd_base.as_deref(),
    ))
}

/// Generic daemon-RPC bridge for the command palette's daemon leaf.
/// Dispatches `method` + `params` through the same `RpcDispatcher`
/// the unix socket uses, so the palette and `ctl` reach exactly the
/// same handlers. Captain-driven only — the palette ships a
/// hardcoded list of methods (reload / shutdown / status / version
/// / diag-snapshot / window-toggle) so this isn't an arbitrary
/// dispatch surface for the webview.
#[tauri::command]
pub(crate) async fn daemon_rpc(
    app: tauri::AppHandle,
    dispatcher: tauri::State<'_, std::sync::Arc<crate::rpc::RpcDispatcher>>,
    status: tauri::State<'_, std::sync::Arc<crate::rpc::StatusBroadcast>>,
    adapter: tauri::State<'_, std::sync::Arc<dyn crate::adapters::Adapter>>,
    config: tauri::State<'_, std::sync::Arc<std::sync::RwLock<crate::config::Config>>>,
    method: String,
    params: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    use crate::rpc::handler::{HandlerCtx, HandlerOutcome};

    let ctx = HandlerCtx {
        app: Some(&app),
        status: status.inner(),
        adapter: adapter.inner().clone(),
        config: Some(config.inner().clone()),
        mcps: None,
        already_subscribed: false,
        already_events_subscribed: false,
        started_at: None,
        socket_path: None,
    };
    match dispatcher
        .dispatch(&method, params.unwrap_or(serde_json::Value::Null), ctx)
        .await
    {
        Ok(HandlerOutcome::Reply(v)) => Ok(v),
        Ok(HandlerOutcome::StatusSubscribed(_, _)) => Err("status/subscribe not supported on the Tauri bridge".into()),
        Ok(HandlerOutcome::EventsSubscribed(_, _, _)) => {
            Err("events/subscribe not supported on the Tauri bridge".into())
        }
        Err(e) => Err(format!("{}: {}", e.code, e.message)),
    }
}

//! Desktop integration — signals the overlay reads from the user's
//! environment that aren't part of `[ui]` config.
//!
//! Today: just `$HOME` for webview-side cwd display. Earlier rounds
//! probed the user's GTK font; that path was deleted in favor of the
//! CSS `ui-sans-serif` / `system-ui` keywords (the webview already
//! resolves them to the platform default), so the daemon no longer
//! reaches for the desktop font setting.
//!
//! Future XDG paths, monitor scale overrides, and other
//! desktop-environment knobs land here when they earn their slot.

/// Resolved home directory for the running user. The webview cannot
/// read it itself (no `process` global, no `~` expansion in the
/// renderer), so the daemon hands it off via Tauri command.
///
/// Goes through `paths::home_dir` (cached `BaseDirs`) so the mapping
/// is correct on every platform we ship to.
#[tauri::command]
pub(crate) fn get_home_dir() -> String {
    crate::paths::home_dir().to_string_lossy().into_owned()
}

/// The daemon's own working directory at the time of the call. Drives
/// the idle-banner context line so the captain sees where new
/// instances will land before they spawn one. Reflects whatever
/// `hyprpilot daemon --cwd <DIR>` was passed (or the spawning shell's
/// cwd when the flag was omitted). Falls back to `/` on
/// `current_dir()` failure — the only realistic source is the daemon's
/// cwd having been deleted out from under it, in which case
/// "unknown" reads cleaner than a noisy error.
#[tauri::command]
pub(crate) fn get_daemon_cwd() -> String {
    std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "/".to_string())
}

/// Tauri shell over the file-attachment hydrator. Implementation
/// lives at `completion::hydration::file::read_file_for_attachment` —
/// it pairs with `completion::source::path::PathSource` (sources
/// detect the path pattern at compose time, this hydrator resolves
/// the picked path into the wire-side attachment body).
#[tauri::command]
pub(crate) async fn read_file_for_attachment(path: String) -> Result<serde_json::Value, String> {
    crate::completion::hydration::file::read_file_for_attachment(path).await
}

/// Git status snapshot for an arbitrary path — drives the header
/// `branch ↑N ↓M` pill. The webview calls this with the active
/// instance's cwd whenever it changes (and on a coarse refresh
/// cadence while visible). Returns `null` when the path doesn't sit
/// inside a git repo. Implementation lives at `tools::git`.
#[tauri::command]
pub(crate) fn get_git_status(path: String) -> Result<Option<crate::tools::git::GitStatus>, String> {
    crate::tools::git::snapshot(std::path::Path::new(&path)).map_err(|e| format!("git status failed: {e:#}"))
}

/// Captain-typed → absolute resolution. Returns `None` when the
/// input is empty or relative-with-no-cwd-base. The webview can't
/// `${VAR}`-expand without OS access, so the daemon owns the
/// resolution path; UI-side display niceties (home → `~`
/// substitution, CSS truncation) stay client-side.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PathsResolveArgs {
    pub raw: String,
    #[serde(default)]
    pub cwd_base: Option<String>,
}

#[tauri::command]
pub(crate) fn paths_resolve(args: PathsResolveArgs) -> Result<Option<String>, String> {
    let home = crate::paths::home_dir();
    let home_str = home.to_string_lossy();
    Ok(crate::tools::path::resolve_absolute(
        &args.raw,
        &home_str,
        args.cwd_base.as_deref(),
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
        already_subscribed: false,
        started_at: None,
        socket_path: None,
    };
    match dispatcher
        .dispatch(&method, params.unwrap_or(serde_json::Value::Null), ctx)
        .await
    {
        Ok(HandlerOutcome::Reply(v)) => Ok(v),
        Ok(HandlerOutcome::StatusSubscribed(_, _)) => Err("status/subscribe not supported on the Tauri bridge".into()),
        Err(e) => Err(format!("{}: {}", e.code, e.message)),
    }
}

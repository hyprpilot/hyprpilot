mod autostart;
mod desktop;
mod renderer;
mod tray;
mod wm;
pub use renderer::WindowRenderer;

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Instant;

use anyhow::{bail, Context, Result};
use clap::Args;
use tauri::{Emitter, Manager, RunEvent, State};
use tokio::net::{UnixListener, UnixStream};
use tracing::{info, warn};

use crate::adapters::commands as adapter_commands;
use crate::adapters::permission::{DefaultPermissionController, PermissionController};
use crate::adapters::{AcpAdapter, Adapter};
use crate::config::{Config, Edge, KeymapsConfig, Theme, Window, WindowMode};
use crate::mcp::MCPsRegistry;
use crate::paths;
use crate::rpc::{RpcDispatcher, StatusBroadcast};

#[derive(Args, Debug, Default, Clone)]
pub struct DaemonArgs {
    /// Override the unix socket path (default: `$XDG_RUNTIME_DIR/hyprpilot.sock`).
    #[arg(long, env = "HYPRPILOT_SOCKET")]
    pub socket: Option<PathBuf>,
    /// Force hidden boot — daemon configures the layer-shell role
    /// without mapping the surface, regardless of `[daemon.window]
    /// visible`. Intended for systemd / autostart contexts where
    /// the captain doesn't want a window paint at login. Equivalent
    /// to `[daemon.window] visible = false` for this run; does not
    /// persist to config.
    #[arg(long)]
    pub hidden: bool,
    /// Working directory the daemon runs in. When set, the daemon
    /// `chdir`s here before Tauri builds — every spawned agent
    /// inherits it via the default cwd, every relative-path read
    /// resolves against it, and `std::env::current_dir()` returns
    /// it for the rest of the process. Without this flag the daemon
    /// inherits the spawning shell's cwd. Useful for hyprland binds
    /// / launcher contexts where the captain wants the daemon to
    /// land in their project root regardless of where the launcher
    /// was invoked from.
    #[arg(long, value_name = "DIR")]
    pub cwd: Option<PathBuf>,
}

#[tauri::command]
fn get_theme(theme: State<'_, Theme>) -> Theme {
    theme.inner().clone()
}

#[tauri::command]
fn get_keymaps(keymaps: State<'_, KeymapsConfig>) -> KeymapsConfig {
    keymaps.inner().clone()
}

/// Surface state the frontend needs to position chrome relative to the
/// anchored screen edge (e.g. draw the `[ui.theme.window] edge` accent on the
/// visible/inward side of the overlay). `anchor_edge` is `None` in center
/// mode — the frontend should render no screen-edge-relative chrome then.
///
/// Intentionally does **not** expose raw config (widths, heights, output
/// selectors) — those are daemon-internal concerns.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WindowState {
    pub(crate) mode: WindowMode,
    pub(crate) anchor_edge: Option<Edge>,
}

#[tauri::command]
fn get_window_state(state: State<'_, WindowState>) -> WindowState {
    state.inner().clone()
}

/// Webview-side surface for `window/toggle`. Drives the overlay's
/// show / hide off the same `WindowRenderer` path the RPC + tray use,
/// serialised through `lock_present` so two concurrent calls can't
/// straddle the `is_visible() → show/hide` window.
#[tauri::command]
async fn window_toggle(
    app: tauri::AppHandle,
    renderer: State<'_, crate::daemon::renderer::WindowRenderer>,
    status: State<'_, Arc<crate::rpc::StatusBroadcast>>,
) -> Result<bool, String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "main window not available".to_string())?;
    let _guard = renderer.lock_present().await;
    let visible = window.is_visible().map_err(|e| format!("is_visible failed: {e}"))?;
    if visible {
        renderer
            .hide_on_main(&app, &window)
            .await
            .map_err(|e| format!("hide failed: {e:#}"))?;
        status.set_visible(false);
        Ok(false)
    } else {
        renderer
            .show_on_main(&app, &window)
            .await
            .map_err(|e| format!("show failed: {e:#}"))?;
        status.set_visible(true);
        let _ = window.set_focus();
        Ok(true)
    }
}

/// Aggregated boot payload — every field the UI needs before the
/// fullscreen Loading overlay can drop. Replaces six sequential
/// `invoke()` round-trips (`get_theme` / `get_keymaps` /
/// `get_window_state` / `get_daemon_cwd` /
/// `get_completion_config` + `agents_list` + `profiles_list` +
/// `instances_list`) with one. Particularly load-bearing on the
/// remote bridge where each round-trip rides the same WS — six
/// awaits sequentially is a 6× RTT bill the captain pays staring
/// at "configuring window…".
///
/// Per-instance snapshot data (`MetaSnapshot`, `ChatSnapshot`,
/// `TerminalsSnapshot`) stays on its own RPCs; brim-sync calls
/// those after boot for whichever instance is focused, on demand.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BootSnapshot {
    pub(crate) theme: Theme,
    pub(crate) keymaps: KeymapsConfig,
    pub(crate) window_state: WindowState,
    /// Daemon working directory in display form (`$HOME` collapsed
    /// to `~`). The captain's mental model is the displayed path; we
    /// don't ship the absolute form because no UI consumer needs it
    /// and shipping it would force every frontend to re-collapse.
    pub(crate) daemon_cwd: String,
    pub(crate) completion_config: serde_json::Value,
    pub(crate) agents: serde_json::Value,
    pub(crate) profiles: serde_json::Value,
    /// Captain's currently-selected default profile id. Seeded from
    /// `[profile] default` at boot; runtime-mutable via `profile/set`.
    /// `null` when no profile has been configured AND no client has
    /// set one. Frontends drive their header pill / palette active
    /// marker off this value, subscribing to `acp:profile-changed`
    /// for live updates.
    pub(crate) selected_profile_id: Option<String>,
    pub(crate) instances: serde_json::Value,
    /// Per-instance queue snapshots keyed by instance id. Second-
    /// frontends connecting fresh use this to avoid an N+1 of
    /// `instance/snapshot/queue` reads. Empty queues are included
    /// (as `[]`) so the consumer can mirror the daemon's per-instance
    /// state set exactly.
    pub(crate) queues: serde_json::Value,
    /// Per-instance first chat-page snapshots keyed by instance id.
    /// Mirrors the shape `instance/snapshot/chat` returns (backward
    /// window anchored at the head, capped at
    /// [`BOOT_CHAT_PAGE_LIMIT`] items). Frontends seed their TanStack
    /// cache directly so the captain navigating into ANY visible
    /// instance sees full chat history immediately — no per-instance
    /// round-trip on focus, no "I only see the latest message"
    /// hydration gap when the daemon has no `focusedId` pointer. Empty
    /// `{ items: [], hasMore: false }` for instances whose mirror has
    /// no transcript yet so the consumer can rely on every listed
    /// instance having a chat key in the map.
    pub(crate) chats: serde_json::Value,
    /// Daemon-side "needs attention" snapshot — same shape the
    /// `notifications_list` Tauri command and `notifications/list`
    /// JSON-RPC method return. Empty `items: []` when no instance is
    /// flagged. Frontends seed their header-pill / palette state
    /// directly so a remote captain authenticating mid-session sees
    /// the pill immediately if anything was already pending.
    pub(crate) notifications: serde_json::Value,
}

/// First-page chat-snapshot size shipped in the boot payload — one
/// flat page per live instance. 100 lines up with the Vue overlay's
/// `BOOT_PAGE_SIZE` and the nvim plugin's `snapshot_limit`, so every
/// frontend has the same baseline of context on cold connect.
const BOOT_CHAT_PAGE_LIMIT: usize = 100;

/// Single source of truth for the boot-time payload — both the
/// `boot_snapshot` Tauri command and the `tauri/boot_snapshot` JSON-RPC
/// mirror call this. Keeps the wire shape lock-stepped between
/// transports without a typed-shim layer.
pub(crate) async fn build_boot_snapshot(
    theme: &Theme,
    keymaps: &KeymapsConfig,
    window_state: &WindowState,
    config: &Arc<RwLock<Config>>,
    adapter: &AcpAdapter,
) -> Result<BootSnapshot, String> {
    let completion_config = {
        let cfg = config.read().map_err(|e| format!("config rwlock poisoned: {e}"))?;
        let rg = &cfg.completion.ripgrep;
        serde_json::json!({
            "ripgrep": {
                "auto": rg.auto.unwrap_or(true),
                "debounceMs": rg.debounce_ms.unwrap_or(250),
                "minPrefix": rg.min_prefix.unwrap_or(3),
            }
        })
    };

    let agents = serde_json::json!({ "agents": adapter.list_agents() });
    let profiles = serde_json::json!({ "profiles": adapter.list_profiles() });

    let instances_list = adapter.list().await;
    let focused_id = adapter.focused_id().await.map(|k| k.as_string());
    let instance_entries: Vec<crate::adapters::InstanceListEntry> = instances_list
        .iter()
        .map(crate::adapters::InstanceListEntry::from)
        .collect();
    let mut instances_payload = serde_json::Map::with_capacity(2);
    instances_payload.insert(
        "instances".into(),
        serde_json::to_value(&instance_entries).map_err(|e| format!("serialize instances: {e}"))?,
    );
    if let Some(id) = focused_id {
        instances_payload.insert("focusedId".into(), serde_json::Value::String(id));
    }

    let daemon_cwd_abs = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "/".to_string());

    // Per-instance queue snapshots. Reads directly off each mirror
    // (no actor round-trip) so a fresh boot snapshot stays cheap even
    // with many live instances. Empty `items` arrays included so the
    // consumer treats absence as "no instance" rather than "queue
    // unknown".
    //
    // `entry.instance_id` came from `adapter.list()` which derives
    // it via `InstanceKey::as_string()`; the round-trip MUST succeed.
    // A parse failure here means a wire-shape mismatch — surface as
    // an error so the daemon log catches the regression instead of
    // silently shipping an instance with no queue field.
    let mut queues = serde_json::Map::with_capacity(instance_entries.len());
    // Per-instance first chat page. Same shape `instance/snapshot/chat`
    // returns (head window of size [`BOOT_CHAT_PAGE_LIMIT`]). Reads
    // straight off the per-instance mirror — no actor round-trip,
    // cheap even with many instances. An instance whose mirror was
    // torn down between `adapter.list()` and the lookup ships an
    // empty `{items: [], oldestSeq: null, latestSeq: null, hasMore:
    // false}` shape so the consumer can rely on every listed
    // instance having a chat key in the map.
    let mut chats = serde_json::Map::with_capacity(instance_entries.len());
    for entry in &instance_entries {
        let key = crate::adapters::InstanceKey::parse(&entry.instance_id).map_err(|e| {
            format!(
                "boot_snapshot: instance id from adapter.list() did not round-trip: {} ({e})",
                entry.instance_id
            )
        })?;
        // `instance_mirror` returning None means the actor was torn
        // down between `adapter.list()` and the mirror lookup — rare
        // race, ship an empty array so consumers don't see a missing
        // key for an instance that was listed.
        let (queue_items, chat_snap) = match adapter.instance_mirror(key).await {
            Some(mirror) => (
                mirror.queue_snapshot().await,
                mirror.chat_snapshot(None, None, BOOT_CHAT_PAGE_LIMIT).await,
            ),
            None => (
                Vec::new(),
                crate::adapters::mirror::ChatSnapshot {
                    items: Vec::new(),
                    oldest_seq: None,
                    latest_seq: None,
                    has_more: false,
                },
            ),
        };
        queues.insert(
            entry.instance_id.clone(),
            serde_json::to_value(&queue_items)
                .map_err(|e| format!("serialize queue for {}: {e}", entry.instance_id))?,
        );
        chats.insert(
            entry.instance_id.clone(),
            serde_json::to_value(&chat_snap).map_err(|e| format!("serialize chat for {}: {e}", entry.instance_id))?,
        );
    }

    let notifications_items = adapter.notifications().list_snapshot();
    let notifications = serde_json::json!({
        "items": serde_json::to_value(&notifications_items)
            .map_err(|e| format!("serialize notifications: {e}"))?,
    });

    Ok(BootSnapshot {
        theme: theme.clone(),
        keymaps: keymaps.clone(),
        window_state: window_state.clone(),
        daemon_cwd: crate::tools::path::display_cwd(&daemon_cwd_abs),
        completion_config,
        agents,
        profiles,
        selected_profile_id: adapter.selected_profile_id(),
        instances: serde_json::Value::Object(instances_payload),
        queues: serde_json::Value::Object(queues),
        chats: serde_json::Value::Object(chats),
        notifications,
    })
}

#[tauri::command]
async fn boot_snapshot(
    theme: State<'_, Theme>,
    keymaps: State<'_, KeymapsConfig>,
    window_state: State<'_, WindowState>,
    config: State<'_, Arc<RwLock<Config>>>,
    adapter: State<'_, Arc<AcpAdapter>>,
) -> Result<BootSnapshot, String> {
    build_boot_snapshot(
        theme.inner(),
        keymaps.inner(),
        window_state.inner(),
        config.inner(),
        adapter.inner(),
    )
    .await
}

/// Daemon entry point. Five phases, each its own helper:
///
/// 1. Resolve socket path from cli / config / `$XDG_RUNTIME_DIR` default.
/// 2. [`bind_socket`] — stale-detection + bind, before Tauri builds.
/// 3. [`RuntimeState::new`] — every Arc construction in dependency order.
/// 4. Tauri builder + plugin chain + `invoke_handler!` registration.
/// 5. [`setup_app`] — `app.manage` calls, GTK font / page zoom,
///    [`install_signal_handler`], [`spawn_accept_loop`].
pub fn run(cfg: Config, args: DaemonArgs) -> Result<()> {
    let started_at = Instant::now();

    // chdir before any further setup so spawned agents inherit the
    // captured cwd (Command::new picks up the parent's cwd on Linux),
    // relative-path config reads resolve against it, and
    // `std::env::current_dir()` returns it for the rest of the
    // process. Expand `~` / `$VAR` so a hyprland bind like
    // `--cwd ~/projects/foo` works without a wrapper script.
    //
    // Resolution: ONLY the `--cwd` flag chdir's the daemon. The old
    // root-level `Config.cwd` was deleted with the patches refactor —
    // captains who want a profile-wide cwd put it in a `[[patches]]`
    // entry's `cwd` field; per-instance agent processes pick that up
    // via the resolved profile, no daemon-level chdir needed.
    if let Some(raw) = args.cwd.as_deref() {
        let target = paths::resolve_user(&raw.to_string_lossy());

        std::env::set_current_dir(&target)
            .with_context(|| format!("daemon: cwd: failed to chdir to {}", target.display()))?;
        info!(cwd = %target.display(), "daemon: cwd applied");
    }

    let socket_path = args
        .socket
        .or_else(|| cfg.daemon.socket.clone())
        .unwrap_or_else(paths::socket_path);
    info!(socket = %socket_path.display(), "starting hyprpilot daemon");

    let listener = bind_socket(&socket_path)?;
    info!(socket = %socket_path.display(), "socket bound");

    // `--hidden` forces a hidden boot — daemon configures the
    // layer-shell role without mapping the surface. Default is
    // visible at boot (matches the pre-MR captain experience).
    // Autostart contexts (systemd unit, hyprland `exec-once`)
    // pass `--hidden` so the overlay doesn't paint over the
    // captain's workspace at login.
    let start_visible = !args.hidden;
    if args.hidden {
        info!("--hidden flag: forcing hidden boot");
    }

    let state = RuntimeState::new(cfg, start_visible);

    let builder = tauri::Builder::default()
        // Webview-side `log.*` wrapper fans into `log::Record`s here.
        // `.skip_logger()` is load-bearing: without it the plugin
        // installs its own fern logger and collides with the
        // `LogTracer` that `tracing-subscriber`'s `tracing-log` feature
        // auto-registers from `logging::init()`. With it, the plugin's
        // `log` command forwards to `log::logger()` — i.e. the
        // LogTracer — which routes into the backend tracing subscriber.
        // One file, both sides.
        .plugin(tauri_plugin_log::Builder::default().skip_logger().build())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_shell::init())
        // Native OS file / folder dialogs — used by the overlay
        // composer's attachments button and the cwd palette's
        // "Browse folder…" entry. Tauri-only; remote/browser
        // frontends fall back to `<input type="file">` for
        // attachments and the daemon-served cwd palette flow.
        .plugin(tauri_plugin_dialog::init())
        // tauri-plugin-autostart MUST register before tauri-plugin-
        // single-instance per the plugin's README — single-instance's
        // forward-and-exit path needs the autostart manager available
        // when a second invocation lands.
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            info!(?argv, ?cwd, "second instance attempted — forwarding to primary");
            if let Err(err) = app.emit(
                "single-instance",
                SingleInstancePayload {
                    argv: argv.clone(),
                    cwd,
                },
            ) {
                warn!(%err, "failed to emit single-instance event");
            }
            // Bare `hyprpilot` (no subcommand, or just `daemon`) from a
            // second invocation pops the overlay — captain's CLI escape
            // hatch when their hyprland keybind isn't bound yet. Same
            // path the tray "show" item + the overlay/show RPC use.
            if argv_is_bare(&argv) {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(err) = tray::present(&app).await {
                        warn!(%err, "second-instance present failed");
                    }
                });
            }
        }));

    #[cfg(feature = "e2e-testing")]
    let builder = builder.plugin(tauri_plugin_playwright::init());

    builder
        .invoke_handler(tauri::generate_handler![
            boot_snapshot,
            get_theme,
            get_keymaps,
            get_window_state,
            window_toggle,
            desktop::paths_resolve,
            desktop::daemon_rpc,
            desktop::read_file_for_attachment,
            adapter_commands::session_submit,
            adapter_commands::session_cancel,
            adapter_commands::agents_list,
            adapter_commands::profiles_list,
            adapter_commands::session_list,
            adapter_commands::session_load,
            adapter_commands::sessions_info,
            adapter_commands::permission_reply,
            adapter_commands::instances_list,
            adapter_commands::instances_focus,
            adapter_commands::instances_shutdown,
            adapter_commands::instances_rename,
            adapter_commands::instance_restart,
            adapter_commands::models_set,
            adapter_commands::modes_set,
            adapter_commands::config_option_set,
            adapter_commands::profile_get,
            adapter_commands::profile_set,
            adapter_commands::instance_meta,
            adapter_commands::instance_snapshot_meta,
            adapter_commands::instance_snapshot_chat,
            adapter_commands::instance_snapshot_terminals,
            adapter_commands::instance_snapshot_queue,
            adapter_commands::mcps_list,
            adapter_commands::queue_list,
            adapter_commands::queue_edit,
            adapter_commands::queue_remove,
            adapter_commands::queue_move,
            adapter_commands::queue_clear,
            adapter_commands::queue_dispatch,
            adapter_commands::notifications_list,
            adapter_commands::notifications_get,
            adapter_commands::notifications_clear,
            adapter_commands::notifications_clear_all,
            adapter_commands::resolve_spawn_cwd,
            crate::skills::commands::skills_list,
            crate::skills::commands::skills_get,
            crate::skills::commands::skills_reload,
            crate::completion::commands::completion_query,
            crate::completion::commands::completion_resolve,
            crate::completion::commands::completion_cancel,
            crate::completion::commands::completion_rank,
            crate::completion::commands::get_completion_config,
            crate::remote::commands::remote_confirm_pair,
            crate::remote::commands::remote_reject_pair,
            crate::remote::commands::remote_pending_pairs,
        ])
        .setup(move |app| {
            setup_app(app, state, listener, started_at, socket_path)?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .context("failed to build Tauri application")?
        .run(|_handle, event| match event {
            RunEvent::ExitRequested { .. } => info!("exit requested"),
            RunEvent::Exit => info!("application exiting"),
            _ => {}
        });

    Ok(())
}

/// Stale-socket-detection + `UnixListener::bind`. Lifted out of `run`
/// so a bind failure aborts with `Err` before the Tauri builder
/// runs — the daemon never opens a window with a broken control
/// surface. Probes with `connect()` and removes only on
/// `ECONNREFUSED`, refusing to clobber anything that's actively
/// listening (e.g. an errant `HYPRPILOT_SOCKET=/var/run/...`).
fn bind_socket(path: &Path) -> Result<UnixListener> {
    tauri::async_runtime::block_on(async {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }
        match UnixStream::connect(path).await {
            Ok(_) => bail!(
                "socket {} is already in use by another process — refusing to clobber it",
                path.display()
            ),
            Err(e) if e.kind() == ErrorKind::ConnectionRefused => {
                tokio::fs::remove_file(path)
                    .await
                    .with_context(|| format!("failed to remove stale socket at {}", path.display()))?;
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {
                // No prior socket file — nothing to clean up.
            }
            Err(e) => bail!("socket path {} is not accessible: {e}", path.display()),
        }
        UnixListener::bind(path).with_context(|| format!("failed to bind socket at {}", path.display()))
    })
}

/// Every shared handle the daemon constructs from `Config`. The dependency
/// order between Arcs (`shared_config → acp_adapter → adapter`,
/// `skills → dispatcher`, `mcps → dispatcher`, …) is captured once in
/// [`RuntimeState::new`]; downstream code (the Tauri `setup` closure +
/// `RpcState` construction) reads handles off this struct without needing
/// to know the construction order.
///
/// Owned-by-value so `setup_app` consumes it via `move` from the
/// `setup` closure — every field except `theme` / `keymaps` /
/// `window_state` is an `Arc` and clones cheaply when downstream code
/// keeps its own handle.
struct RuntimeState {
    theme: Theme,
    keymaps: KeymapsConfig,
    window_state: WindowState,
    renderer: WindowRenderer,
    status: Arc<StatusBroadcast>,
    permissions: Arc<dyn PermissionController>,
    acp_adapter: Arc<AcpAdapter>,
    adapter: Arc<dyn Adapter>,
    mcps: Arc<MCPsRegistry>,
    dispatcher: Arc<RpcDispatcher>,
    shared_config: Arc<RwLock<Config>>,
    /// Resolved boot visibility from `--hidden`. `true` → map the
    /// surface at setup; `false` → configure-only, wait for
    /// `overlay/show`.
    start_visible: bool,
}

impl RuntimeState {
    fn new(cfg: Config, start_visible: bool) -> Self {
        let theme = cfg.ui.theme.clone();
        let keymaps = cfg.keymaps.clone();
        let window_cfg: Window = cfg.daemon.window.clone();
        // Share one Arc<RwLock<Config>> between AcpAdapter and RpcState so
        // both reach the same instance — config is read-only at runtime,
        // the lock is just to thread one handle through cheaply.
        let shared_config = Arc::new(RwLock::new(cfg));

        // Snapshot the resolved window state up-front so the webview can fetch
        // it without re-reading the config at request time. `anchor_edge` is
        // `Some` in anchor mode so the frontend can paint the edge accent on
        // the inward side; `None` in center mode signals "no screen-edge chrome".
        let mode = window_cfg.mode.expect("[daemon.window] mode seeded by defaults.toml");
        let window_state = WindowState {
            mode,
            anchor_edge: match mode {
                WindowMode::Anchor => Some(
                    window_cfg
                        .anchor
                        .edge
                        .expect("[daemon.window.anchor] edge seeded by defaults.toml"),
                ),
                WindowMode::Center => None,
            },
        };

        // Initial visible bit tracks `--hidden` (false → visible at boot,
        // true → hidden). Waybar's `custom/hyprpilot` block reads this;
        // the bit flips on every overlay/show / overlay/hide
        // transition afterwards.
        let status = Arc::new(StatusBroadcast::new(start_visible));
        // Single PermissionController shared between AcpClient (one per
        // live instance, accessed through AcpAdapter's instance registry)
        // and the permission_reply Tauri command — both resolve against
        // the same waiter map so UI replies reach the awaiting ACP
        // handler regardless of which instance issued the prompt.
        // Hold the concrete type briefly so we can wire the registry
        // events broadcast into the controller (see
        // `attach_events_tx` below) before upcasting to
        // `Arc<dyn PermissionController>` for sharing.
        let default_permissions = Arc::new(DefaultPermissionController::new());
        let permissions: Arc<dyn PermissionController> = default_permissions.clone();
        // ACP adapter + generic `dyn Adapter` view. Tauri managed state
        // carries both — the concrete for config-adjacent commands
        // (`agents_list`, `session_load`, …) and the generic for the RPC
        // handlers which stay adapter-agnostic.
        let acp_adapter = Arc::new(AcpAdapter::with_shared_config(
            shared_config.clone(),
            status.clone(),
            permissions.clone(),
        ));
        // Now that the adapter (which owns the registry) exists, wire
        // its event broadcast into the controller so
        // `resolve_if_pending` / `forget` can emit
        // `PermissionResolved` events that mirrors and remote
        // subscribers consume — closing the desktop ↔ remote desync
        // where one transport answered a prompt the other was still
        // showing.
        default_permissions.attach_events_tx(acp_adapter.events_tx());
        let adapter: Arc<dyn Adapter> = acp_adapter.clone();

        // Skills are now per-instance — built at AcpInstance::start
        // from the active profile's `skills = [...]` (with global
        // fallback). The daemon-global registry is gone; the palette /
        // autocomplete / hydrator all read from the focused
        // instance's registry through `AcpAdapter::focused_skills`.

        // MCP registry — empty at daemon boot. Root-level `mcps`
        // was removed in the patches refactor; every effective MCP
        // set is per-instance now, built lazily from the resolved
        // profile (possibly via `[[patches]]`) in
        // `acp::instances::build_mcp_registry_with` at spawn time.
        // The daemon-scoped registry survives only as the
        // no-instance fallback some RPC handlers reach for; it stays
        // empty until / unless a future shape repopulates it.
        let mcps = Arc::new(MCPsRegistry::new(Vec::new()));
        let dispatcher = Arc::new(RpcDispatcher::with_defaults());

        let renderer = WindowRenderer::new(window_cfg, wm::detect());

        Self {
            theme,
            keymaps,
            window_state,
            renderer,
            status,
            permissions,
            acp_adapter,
            adapter,
            mcps,
            dispatcher,
            shared_config,
            start_visible,
        }
    }
}

/// Body of the Tauri `.setup(move |app| { ... })` closure. Owns the
/// "things that need a live `AppHandle`" phase: `app.manage` calls,
/// GTK font query + page zoom, layer-shell mapping via the renderer,
/// the ACP → Tauri event bridge, `RpcState` construction, signal
/// handler install, accept loop spawn.
fn setup_app(
    app: &tauri::App,
    state: RuntimeState,
    listener: UnixListener,
    started_at: Instant,
    socket_path: PathBuf,
) -> Result<()> {
    app.manage(state.theme);
    app.manage(state.keymaps);
    app.manage(state.window_state);
    app.manage(state.renderer.clone());

    let main = app
        .get_webview_window("main")
        .context("main webview window missing from tauri.conf.json")?;

    // The main window is created invisible in tauri.conf.json so that
    // `init_layer_shell()` can run before the surface is mapped — the
    // Wayland compositor rejects layer-shell init on an already-mapped
    // window with a critical assertion. `show` configures the
    // mode-specific surface and maps the window once ready.
    //
    // `--hidden` flow (`start_visible = false`): configure the
    // layer-shell role + size but don't map the surface. First
    // user-visible map happens through `overlay/show` (Hyprland
    // keybind, the tray "show" action, or the bare-`hyprpilot`
    // escape hatch). Configuring the role early avoids the
    // "init_layer_shell on a realized window" failure that surfaces
    // if the first map happens out-of-order, AND defends against
    // Tauri auto-showing the window after setup (which would paint
    // a black surface on top of the captain's workspace).
    if state.start_visible {
        state.renderer.show(&main)?;
    } else {
        state.renderer.configure_hidden(&main)?;
        info!("--hidden: surface configured but not mapped; waits on overlay/show");
    }

    // Apply the configured page zoom. Chromium-style page zoom via
    // WebKit's `set_zoom_level` — scales text + layout together,
    // unlike a CSS root font-size knob which only scales `rem`-based
    // primitives and leaves explicit `px` paddings untouched. The
    // value is seeded by `[ui] zoom` in defaults.toml; user TOMLs
    // override it. Always invoke (even at 1.0) so the log line
    // confirms the config knob is wired and what value reached the
    // webview — silent skip would make "still small" debugging
    // ambiguous.
    let zoom = state
        .shared_config
        .read()
        .expect("config rwlock poisoned")
        .ui
        .zoom
        .expect("ui.zoom seeded by defaults.toml");
    match main.set_zoom(zoom) {
        Ok(()) => info!(zoom, "applied [ui] zoom"),
        Err(err) => warn!(?err, zoom, "failed to apply [ui] zoom"),
    }

    app.manage(state.acp_adapter.clone());
    app.manage(state.permissions);
    app.manage(state.mcps.clone());
    app.manage(state.status.clone());
    app.manage(state.adapter.clone());
    state.acp_adapter.spawn_tauri_event_bridge(app.handle().clone());

    // Inline-token hydration. One scheme today (`hyprpilot://`);
    // future schemes plug in by pushing onto this registry.
    // session_submit pulls it from managed state. The hyprpilot
    // hydrator queries the focused instance's registry on every call —
    // no daemon-global cache.
    let hydrators = crate::completion::hydration::TokenHydrators::new().with(Arc::new(
        crate::completion::hydration::HyprpilotTokenHydrator::new(state.acp_adapter.clone()),
    ));
    app.manage(hydrators);

    // Composer autocomplete registry — sources walk in order, first
    // match wins. Cancellation tokens live alongside (one per
    // request_id, ripgrep checks them between matches). The shared
    // commands cache is handed to the ACP adapter so per-instance
    // `available_commands_update` notifications populate the slash
    // source.
    let completion_config = state
        .shared_config
        .read()
        .expect("config rwlock poisoned")
        .completion
        .clone();
    let (completion_registry, commands_cache) =
        build_completion_registry(state.acp_adapter.clone(), &completion_config);
    state.acp_adapter.set_commands_cache(commands_cache);
    let completion_cancellations = Arc::new(crate::completion::CompletionCancellations::default());
    app.manage(completion_registry);
    app.manage(completion_cancellations);
    // Shared config — Tauri commands needing live config slices
    // (`get_completion_config`) read from this RwLock.
    app.manage(state.shared_config.clone());
    // Shared dispatcher — the palette's `daemon_rpc` Tauri command
    // routes daemon-namespace methods through the same handler tree
    // the unix socket uses.
    app.manage(state.dispatcher.clone());

    // System tray icon — captain's "alive" indicator + quick-action
    // menu (toggle / show / hide / shutdown). Failures degrade to a
    // warn so a tray-less environment (no system tray at all) doesn't
    // abort boot.
    if let Err(err) = tray::install(app) {
        warn!(%err, "tray: install failed — daemon continues without a tray icon");
    }

    // Reconcile autostart entry against `[autostart] enabled`. Source
    // of truth is the config file; daemon edits the OS-side entry on
    // every boot to match. Failures warn-and-continue.
    if let Err(err) = autostart::reconcile(app.handle(), &state.shared_config) {
        warn!(%err, "autostart: reconcile failed — daemon continues, autostart state may drift");
    }

    let rpc_state = crate::rpc::RpcState {
        app: app.handle().clone(),
        status: state.status,
        dispatcher: state.dispatcher,
        adapter: state.adapter.clone(),
        config: state.shared_config,
        mcps: state.mcps,
        started_at,
        socket_path,
    };

    install_signal_handler(app.handle().clone(), state.adapter);
    spawn_accept_loop(listener, rpc_state.clone());
    spawn_remote_bridge(app.handle().clone(), &rpc_state);

    Ok(())
}

/// Bring up the optional TLS axum HTTP+WS server when
/// `[remote] enabled = true`. Phone (or any browser) loads the
/// embedded SPA over HTTPS; per-connection pair confirmation gates
/// every WS upgrade. Failures here warn-and-continue so a misconfigured
/// remote block doesn't abort the daemon's main overlay path.
fn spawn_remote_bridge(app: tauri::AppHandle, rpc_state: &crate::rpc::RpcState) {
    let cfg = rpc_state.config.read().expect("config lock poisoned").remote.clone();
    if !cfg.enabled() {
        return;
    }
    let bind = match crate::remote::server::parse_bind(&cfg.resolved_bind()) {
        Ok(b) => b,
        Err(err) => {
            warn!(%err, "remote: invalid bind — bridge disabled");
            return;
        }
    };
    let tls = match crate::remote::cert::resolve_or_generate(&cfg) {
        Ok(t) => t,
        Err(err) => {
            warn!(%err, "remote: failed to resolve TLS material — bridge disabled");
            return;
        }
    };
    let pairs = crate::remote::pair::PairStore::new();
    let sessions = crate::remote::session::SessionTokens::new();
    let state = crate::remote::server::RemoteState {
        app: app.clone(),
        status: rpc_state.status.clone(),
        dispatcher: rpc_state.dispatcher.clone(),
        adapter: rpc_state.adapter.clone(),
        config: rpc_state.config.clone(),
        mcps: rpc_state.mcps.clone(),
        pairs: pairs.clone(),
        sessions: sessions.clone(),
        started_at: rpc_state.started_at,
    };
    app.manage(pairs);
    app.manage(sessions);
    tauri::async_runtime::spawn(async move {
        if let Err(err) = crate::remote::server::serve(bind, tls, state).await {
            warn!(%err, "remote: serve loop terminated");
        }
    });
    info!(%bind, "remote: bridge spawned");
}

/// Install SIGINT / SIGTERM handlers that route to [`shutdown`] —
/// same path as `daemon/kill`. Second signal falls through to the
/// default handler (force-kill), so SIGINT-twice escapes a stuck
/// shutdown.
fn install_signal_handler(app: tauri::AppHandle, adapter: Arc<dyn Adapter>) {
    tauri::async_runtime::spawn(async move {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigint = match signal(SignalKind::interrupt()) {
            Ok(s) => s,
            Err(err) => {
                warn!(%err, "failed to install SIGINT handler — default behaviour takes over");
                return;
            }
        };
        let mut sigterm = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(err) => {
                warn!(%err, "failed to install SIGTERM handler — default behaviour takes over");
                return;
            }
        };
        tokio::select! {
            _ = sigint.recv()  => info!("received SIGINT, initiating clean shutdown"),
            _ = sigterm.recv() => info!("received SIGTERM, initiating clean shutdown"),
        }
        shutdown(&app, adapter.as_ref()).await;
    });
}

/// Spawn the accept loop on the bound listener. Each accepted
/// connection gets its own task running [`crate::rpc::handle_connection`];
/// `accept` errors log + continue so a transient `EAGAIN` doesn't
/// take the loop down.
fn spawn_accept_loop(listener: UnixListener, rpc_state: crate::rpc::RpcState) {
    tauri::async_runtime::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _addr)) => {
                    let state = rpc_state.clone();
                    tauri::async_runtime::spawn(async move {
                        crate::rpc::handle_connection(stream, state).await;
                    });
                }
                Err(err) => warn!(%err, "accept failed"),
            }
        }
    });
}

#[derive(Clone, serde::Serialize)]
struct SingleInstancePayload {
    argv: Vec<String>,
    cwd: String,
}

/// True when a second invocation's argv carries no subcommand other
/// than the implicit `daemon` default — bare `hyprpilot` or
/// `hyprpilot daemon`. Anything beyond (`ctl …`, `--help`, `--version`)
/// stays out of the present-on-second-instance escape hatch so
/// `hyprpilot ctl status` from a shell doesn't accidentally pop the
/// overlay.
fn argv_is_bare(argv: &[String]) -> bool {
    let tail: Vec<&str> = argv
        .iter()
        .skip(1) // skip the binary path
        .filter(|s| !s.is_empty())
        .map(String::as_str)
        .collect();
    matches!(tail.as_slice(), [] | ["daemon"])
}

/// Build the composer-autocomplete `CompletionRegistry` with the four
/// sources in priority order (slash → skills → path → ripgrep). The
/// slash source's cache is shared with the ACP adapter so each
/// instance's `available_commands_update` notification refreshes the
/// completion list in place. The skills source captures the adapter
/// so each query reads from the focused instance's per-profile
/// registry — switching instances flips the visible skill set
/// without rebuilding any state.
fn build_completion_registry(
    adapter: Arc<AcpAdapter>,
    completion_config: &crate::config::CompletionConfig,
) -> (
    Arc<crate::completion::CompletionRegistry>,
    crate::completion::source::commands::CommandsCache,
) {
    use crate::completion::source::{
        commands::{CommandsCache, CommandsSource},
        path::PathSource,
        ripgrep::RipgrepSource,
        skills::{SkillsResolver, SkillsSource},
    };
    let commands_cache: CommandsCache = Arc::new(std::sync::RwLock::new(Vec::new()));
    let skills_resolver: SkillsResolver = {
        let adapter = adapter.clone();
        Arc::new(move || {
            let adapter = adapter.clone();
            Box::pin(async move { adapter.focused_skills().await })
        })
    };
    let registry = Arc::new(
        crate::completion::CompletionRegistry::new()
            .with_source(Arc::new(CommandsSource::new(commands_cache.clone())))
            .with_source(Arc::new(SkillsSource::new(skills_resolver)))
            .with_source(Arc::new(PathSource::new()))
            .with_source(Arc::new(RipgrepSource::from_config(&completion_config.ripgrep))),
    );
    (registry, commands_cache)
}

/// Drain adapter instances, then kick Tauri's teardown. Called by
/// `rpc::server` on `daemon/kill` and by [`install_signal_handler`].
/// Socket file is not removed — next-start probes stale sockets via
/// `ECONNREFUSED`, which handles crash cases too.
///
/// Takes `&dyn Adapter` so callers route via the trait — when an HTTP
/// adapter lands the same shutdown path covers it.
pub(crate) async fn shutdown(app: &tauri::AppHandle, adapter: &dyn Adapter) {
    info!("shutdown: initiating clean shutdown");
    adapter.shutdown().await;
    info!("shutdown: adapter instances drained");
    app.exit(0);
    info!("shutdown: tauri exit dispatched");
}

#[cfg(test)]
mod tests {
    use super::argv_is_bare;

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn argv_is_bare_matches_no_subcommand() {
        assert!(argv_is_bare(&argv(&["/usr/bin/hyprpilot"])));
        assert!(argv_is_bare(&argv(&["hyprpilot"])));
    }

    #[test]
    fn argv_is_bare_matches_explicit_daemon() {
        assert!(argv_is_bare(&argv(&["hyprpilot", "daemon"])));
    }

    #[test]
    fn argv_is_bare_rejects_ctl_subcommands() {
        assert!(!argv_is_bare(&argv(&["hyprpilot", "ctl", "status"])));
        assert!(!argv_is_bare(&argv(&["hyprpilot", "ctl", "overlay", "toggle"])));
    }

    #[test]
    fn argv_is_bare_rejects_help_and_flags() {
        assert!(!argv_is_bare(&argv(&["hyprpilot", "--help"])));
        assert!(!argv_is_bare(&argv(&["hyprpilot", "--version"])));
        assert!(!argv_is_bare(&argv(&["hyprpilot", "daemon", "--socket=/tmp/foo"])));
    }

    #[test]
    fn argv_is_bare_skips_empty_strings() {
        assert!(argv_is_bare(&argv(&["hyprpilot", "", ""])));
        assert!(argv_is_bare(&argv(&["hyprpilot", "", "daemon"])));
    }
}

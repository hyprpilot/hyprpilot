mod desktop;
mod renderer;
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
use crate::mcp::{MCPsBroadcast, MCPsRegistry};
use crate::paths;
use crate::rpc::handler::ConfigLoadContext;
use crate::rpc::{RpcDispatcher, StatusBroadcast};
use crate::skills::{spawn_watcher, SkillsBroadcast, SkillsRegistry};

#[derive(Args, Debug, Default, Clone)]
pub struct DaemonArgs {
    /// Override the unix socket path (default: `$XDG_RUNTIME_DIR/hyprpilot.sock`).
    #[arg(long, env = "HYPRPILOT_SOCKET")]
    pub socket: Option<PathBuf>,
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
struct WindowState {
    mode: WindowMode,
    anchor_edge: Option<Edge>,
}

#[tauri::command]
fn get_window_state(state: State<'_, WindowState>) -> WindowState {
    state.inner().clone()
}

/// Daemon entry point. Five phases, each its own helper:
///
/// 1. Resolve socket path from cli / config / `$XDG_RUNTIME_DIR` default.
/// 2. [`bind_socket`] — stale-detection + bind, before Tauri builds.
/// 3. [`RuntimeState::new`] — every Arc construction in dependency order.
/// 4. Tauri builder + plugin chain + `invoke_handler!` registration.
/// 5. [`setup_app`] — `app.manage` calls, GTK font / page zoom,
///    [`install_signal_handler`], [`spawn_accept_loop`].
pub fn run(cfg: Config, args: DaemonArgs, config_load_context: ConfigLoadContext) -> Result<()> {
    let started_at = Instant::now();
    let socket_path = args
        .socket
        .or_else(|| cfg.daemon.socket.clone())
        .unwrap_or_else(paths::socket_path);
    info!(socket = %socket_path.display(), "starting hyprpilot daemon");

    let listener = bind_socket(&socket_path)?;
    info!(socket = %socket_path.display(), "socket bound");

    let state = RuntimeState::new(cfg);

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
        .plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            info!(?argv, ?cwd, "second instance attempted — forwarding to primary");
            if let Err(err) = app.emit("single-instance", SingleInstancePayload { argv, cwd }) {
                warn!(%err, "failed to emit single-instance event");
            }
        }));

    #[cfg(feature = "e2e-testing")]
    let builder = builder.plugin(tauri_plugin_playwright::init());

    builder
        .invoke_handler(tauri::generate_handler![
            get_theme,
            get_keymaps,
            get_window_state,
            desktop::get_gtk_font,
            desktop::get_home_dir,
            adapter_commands::session_submit,
            adapter_commands::session_cancel,
            adapter_commands::agents_list,
            adapter_commands::commands_list,
            adapter_commands::profiles_list,
            adapter_commands::session_list,
            adapter_commands::session_load,
            adapter_commands::sessions_info,
            adapter_commands::permission_reply,
            adapter_commands::instances_list,
            adapter_commands::instances_focus,
            adapter_commands::instances_shutdown,
            adapter_commands::instance_restart,
            adapter_commands::models_set,
            adapter_commands::modes_set,
            adapter_commands::mcps_list,
            adapter_commands::mcps_set,
            crate::skills::commands::skills_list,
            crate::skills::commands::skills_get,
        ])
        .setup(move |app| {
            setup_app(app, state, listener, started_at, socket_path, config_load_context)?;
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
    skills: Arc<SkillsRegistry>,
    mcps: Arc<MCPsRegistry>,
    dispatcher: Arc<RpcDispatcher>,
    shared_config: Arc<RwLock<Config>>,
}

impl RuntimeState {
    fn new(cfg: Config) -> Self {
        let theme = cfg.ui.theme.clone();
        let keymaps = cfg.keymaps.clone();
        let window_cfg: Window = cfg.daemon.window.clone();
        let skills_dirs = resolve_skills_dirs(&cfg);
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

        // The window starts visible (`true`) because each mode's setup code calls
        // `show()` / `show_all()` before the RPC loop accepts connections.
        let status = Arc::new(StatusBroadcast::new(true));
        // Single PermissionController shared between AcpClient (one per
        // live instance, accessed through AcpAdapter's instance registry)
        // and the permission_reply Tauri command — both resolve against
        // the same waiter map so UI replies reach the awaiting ACP
        // handler regardless of which instance issued the prompt.
        let permissions: Arc<dyn PermissionController> = Arc::new(DefaultPermissionController::new());
        // ACP adapter + generic `dyn Adapter` view. Tauri managed state
        // carries both — the concrete for config-adjacent commands
        // (`agents_list`, `session_load`, …) and the generic for the RPC
        // handlers which stay adapter-agnostic.
        let acp_adapter = Arc::new(AcpAdapter::with_shared_config(
            shared_config.clone(),
            status.clone(),
            permissions.clone(),
        ));
        let adapter: Arc<dyn Adapter> = acp_adapter.clone();

        // Skills registry + watcher. Errors here are logged but don't abort
        // boot — the registry still serves whatever loaded on the initial scan.
        let skills_broadcast = Arc::new(SkillsBroadcast::new());
        let skills = Arc::new(SkillsRegistry::new(skills_dirs, skills_broadcast));
        if let Err(err) = skills.reload() {
            warn!(%err, "skills registry: initial reload failed");
        }
        if let Err(err) = spawn_watcher(skills.clone()) {
            warn!(%err, "skills watcher: spawn failed — live reload disabled");
        }

        // MCP catalog registry. Static after daemon start (no live reload yet
        // — `daemon/reload` lands in K-279).
        let mcps_broadcast = Arc::new(MCPsBroadcast::new());
        let mcps_defs = shared_config.read().expect("config lock poisoned").mcps.clone();
        let mcps = Arc::new(MCPsRegistry::new(mcps_defs, mcps_broadcast));
        let dispatcher = Arc::new(RpcDispatcher::with_skills_and_mcps(skills.clone(), mcps.clone()));

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
            skills,
            mcps,
            dispatcher,
            shared_config,
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
    config_load_context: ConfigLoadContext,
) -> Result<()> {
    app.manage(state.theme);
    app.manage(state.keymaps);
    app.manage(state.window_state);
    app.manage(state.renderer.clone());

    // GTK is initialized by Tauri before the setup closure fires, so
    // gtk::Settings::default() is safe to call here. Queried once at
    // boot rather than on every `get_gtk_font` tick — the user would
    // need to relaunch the overlay to pick up a desktop font change.
    let gtk_font = desktop::query_gtk_font();
    if let Some(f) = &gtk_font {
        info!(family = %f.family, size_pt = f.size_pt, "resolved GTK font");
    }
    app.manage(gtk_font.clone());

    let main = app
        .get_webview_window("main")
        .context("main webview window missing from tauri.conf.json")?;

    // The main window is created invisible in tauri.conf.json so that
    // `init_layer_shell()` can run before the surface is mapped — the
    // Wayland compositor rejects layer-shell init on an already-mapped
    // window with a critical assertion. `show` configures the
    // mode-specific surface and maps the window once ready.
    state.renderer.show(&main)?;

    // Page zoom per the user's desktop font size. Linear ramp with
    // 10pt as the 1.0 baseline: 11pt → 1.1×, 12pt → 1.2×. Unlike
    // `html { font-size }`, `set_zoom` scales text + layout together
    // (Chromium-style page zoom via WebKit's `set_zoom_level`) — no
    // fonts-bigger-but-margins-fixed breakage that plain font-scale
    // causes on WebKitGTK.
    if let Some(f) = &gtk_font {
        let zoom = (1.0_f64 + (f64::from(f.size_pt) - 10.0) * 0.1).clamp(0.5, 2.0);
        match main.set_zoom(zoom) {
            Ok(()) => info!(zoom, size_pt = f.size_pt, "applied GTK-font page zoom"),
            Err(err) => warn!(?err, zoom, "failed to apply GTK-font page zoom"),
        }
    }

    app.manage(state.acp_adapter.clone());
    app.manage(state.permissions);
    app.manage(state.mcps.clone());
    app.manage(state.skills.clone());
    state.acp_adapter.spawn_tauri_event_bridge(app.handle().clone());

    let rpc_state = crate::rpc::RpcState {
        app: app.handle().clone(),
        status: state.status,
        dispatcher: state.dispatcher,
        adapter: state.adapter,
        acp_adapter: state.acp_adapter.clone(),
        config: state.shared_config,
        started_at,
        socket_path,
        config_load_context,
        skills: state.skills,
        mcps: state.mcps,
    };

    install_signal_handler(app.handle().clone(), state.acp_adapter);
    spawn_accept_loop(listener, rpc_state);

    Ok(())
}

/// Install SIGINT / SIGTERM handlers that route to [`shutdown`] —
/// same path as `daemon/kill`. Second signal falls through to the
/// default handler (force-kill), so SIGINT-twice escapes a stuck
/// shutdown.
fn install_signal_handler(app: tauri::AppHandle, adapter: Arc<AcpAdapter>) {
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

/// Resolve the skills roots, honouring `HYPRPILOT_SKILLS_DIR` first
/// so manual smoke tests can point at a throwaway directory without
/// editing `config.toml`. Falls back to `[skills] dirs` (each entry
/// tilde-expanded) — defaults seed `~/.config/hyprpilot/skills`.
fn resolve_skills_dirs(cfg: &Config) -> Vec<PathBuf> {
    if let Ok(raw) = std::env::var("HYPRPILOT_SKILLS_DIR") {
        if !raw.is_empty() {
            return vec![PathBuf::from(raw)];
        }
    }
    cfg.skills.resolved_dirs()
}

/// Drain adapter instances, then kick Tauri's teardown. Called by
/// `rpc::server` on `daemon/kill` and by [`install_signal_handler`].
/// Socket file is not removed — next-start probes stale sockets via
/// `ECONNREFUSED`, which handles crash cases too.
pub(crate) async fn shutdown(app: &tauri::AppHandle, adapter: &AcpAdapter) {
    info!("shutdown: initiating clean shutdown");
    adapter.shutdown_all().await;
    info!("shutdown: adapter instances drained");
    app.exit(0);
    info!("shutdown: tauri exit dispatched");
}

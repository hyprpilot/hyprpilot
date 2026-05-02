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
use crate::skills::SkillsRegistry;

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
struct WindowState {
    mode: WindowMode,
    anchor_edge: Option<Edge>,
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
    if let Some(raw) = args.cwd.as_deref() {
        let expanded = shellexpand::full(&raw.to_string_lossy())
            .map(|s| s.into_owned())
            .unwrap_or_else(|_| raw.to_string_lossy().into_owned());
        let target = PathBuf::from(&expanded);

        std::env::set_current_dir(&target)
            .with_context(|| format!("daemon: --cwd: failed to chdir to {}", target.display()))?;
        info!(cwd = %target.display(), "daemon: --cwd applied");
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
            // path the tray "show" item + the overlay/present RPC use.
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
            get_theme,
            get_keymaps,
            get_window_state,
            window_toggle,
            desktop::get_home_dir,
            desktop::get_daemon_cwd,
            desktop::read_file_for_attachment,
            adapter_commands::session_submit,
            adapter_commands::session_cancel,
            adapter_commands::agents_list,
            adapter_commands::profiles_list,
            adapter_commands::session_list,
            adapter_commands::session_load,
            adapter_commands::sessions_info,
            adapter_commands::permission_reply,
            adapter_commands::permissions_trust_snapshot,
            adapter_commands::permissions_trust_forget,
            adapter_commands::instances_list,
            adapter_commands::instances_focus,
            adapter_commands::instances_shutdown,
            adapter_commands::instances_rename,
            adapter_commands::instance_restart,
            adapter_commands::models_set,
            adapter_commands::modes_set,
            adapter_commands::instance_meta,
            adapter_commands::mcps_list,
            crate::skills::commands::skills_list,
            crate::skills::commands::skills_get,
            crate::skills::commands::skills_reload,
            crate::completion::commands::completion_query,
            crate::completion::commands::completion_resolve,
            crate::completion::commands::completion_cancel,
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
    skills: Arc<SkillsRegistry>,
    mcps: Arc<MCPsRegistry>,
    dispatcher: Arc<RpcDispatcher>,
    shared_config: Arc<RwLock<Config>>,
    /// Resolved boot visibility from `--hidden`. `true` → map the
    /// surface at setup; `false` → configure-only, wait for
    /// `overlay/present`.
    start_visible: bool,
}

impl RuntimeState {
    fn new(cfg: Config, start_visible: bool) -> Self {
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

        // Initial visible bit tracks `--hidden` (false → visible at boot,
        // true → hidden). Waybar's `custom/hyprpilot` block reads this;
        // the bit flips on every overlay/present / overlay/hide
        // transition afterwards.
        let status = Arc::new(StatusBroadcast::new(start_visible));
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

        // Skills registry. Captain-driven reload via the palette's
        // "reload skills" entry / `skills/reload` RPC; no fs watcher.
        let skills = Arc::new(SkillsRegistry::new(skills_dirs));
        if let Err(err) = skills.reload() {
            warn!(%err, "skills registry: initial reload failed");
        }

        // MCP registry — resolved at daemon boot from the JSON files
        // listed under top-level `mcps`. Empty when no files are
        // configured (default state for fresh installs). Captain
        // edits + `daemon/reload` triggers a re-read; existing
        // instances keep their cached set, only restarted ones pick
        // up changes (ACP fixes mcpServers at session/new).
        let mcps_files = shared_config
            .read()
            .expect("config lock poisoned")
            .mcps
            .clone()
            .unwrap_or_default();
        let mcps_defs = crate::mcp::loader::load_files(&mcps_files);
        let mcps = Arc::new(MCPsRegistry::new(mcps_defs));
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
            skills,
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
    // user-visible map happens through `overlay/present` (Hyprland
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
        info!("--hidden: surface configured but not mapped; waits on overlay/present");
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
    app.manage(state.skills.clone());
    app.manage(state.status.clone());
    app.manage(state.adapter.clone());
    state.acp_adapter.spawn_tauri_event_bridge(app.handle().clone());

    // Inline-token hydration. One scheme today (`skills://`); future
    // schemes (e.g. `prompt://`, `clip://`) plug in by pushing onto
    // this registry. session_submit pulls it from managed state.
    let hydrators = crate::adapters::tokens::TokenHydrators::new().with(Arc::new(
        crate::adapters::commands::SkillTokenHydrator::new(state.skills.clone()),
    ));
    app.manage(hydrators);

    // Composer autocomplete registry — sources walk in order, first
    // match wins. Cancellation tokens live alongside (one per
    // request_id, ripgrep checks them between matches). The shared
    // commands cache is handed to the ACP adapter so per-instance
    // `available_commands_update` notifications populate the slash
    // source.
    let (completion_registry, commands_cache) = build_completion_registry(state.skills.clone());
    state.acp_adapter.set_commands_cache(commands_cache);
    let completion_cancellations = Arc::new(crate::completion::CompletionCancellations::default());
    app.manage(completion_registry);
    app.manage(completion_cancellations);

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
        started_at,
        socket_path,
    };

    install_signal_handler(app.handle().clone(), state.adapter);
    spawn_accept_loop(listener, rpc_state);

    Ok(())
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
/// completion list in place.
fn build_completion_registry(
    skills: Arc<SkillsRegistry>,
) -> (
    Arc<crate::completion::CompletionRegistry>,
    crate::completion::source::commands::CommandsCache,
) {
    use crate::completion::source::{
        commands::{CommandsCache, CommandsSource},
        path::PathSource,
        ripgrep::RipgrepSource,
        skills::SkillsSource,
    };
    let commands_cache: CommandsCache = Arc::new(std::sync::RwLock::new(Vec::new()));
    let registry = Arc::new(
        crate::completion::CompletionRegistry::new()
            .with_source(Arc::new(CommandsSource::new(commands_cache.clone())))
            .with_source(Arc::new(SkillsSource::new(skills)))
            .with_source(Arc::new(PathSource::new()))
            .with_source(Arc::new(RipgrepSource::new())),
    );
    (registry, commands_cache)
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

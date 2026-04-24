mod renderer;
mod wm;
pub use renderer::WindowRenderer;

use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use anyhow::{bail, Context, Result};
use clap::Args;
use tauri::{Emitter, Manager, RunEvent, State};
use tokio::net::{UnixListener, UnixStream};
use tracing::{info, warn};

use crate::adapters::commands as adapter_commands;
use crate::adapters::permission::{DefaultPermissionController, PermissionController};
use crate::adapters::{AcpAdapter, Adapter};
use crate::config::{Config, Edge, KeymapsConfig, Theme, Window, WindowMode};
use crate::paths;
use crate::rpc::{RpcDispatcher, StatusBroadcast};

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

/// User-desktop GTK font setting, parsed from `gtk-font-name` on the default
/// `gtk::Settings`. Stored in managed state at boot so the webview can read
/// the base font size synchronously. `None` when the GTK query fails (no
/// settings singleton or unparseable font string) — the CSS fallback (browser
/// default) is the correct behaviour then.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GtkFont {
    pub family: String,
    pub size_pt: f32,
}

#[tauri::command]
fn get_gtk_font(font: State<'_, Option<GtkFont>>) -> Option<GtkFont> {
    font.inner().clone()
}

/// Parse a GTK font string ("Inter 10", "JetBrains Mono Bold 11", "Sans 10")
/// into `{ family, size_pt }`. The last whitespace-separated token is the
/// point size; every preceding token is family (joined back with spaces).
/// Returns `None` when the trailing token isn't a valid positive float or
/// the input is missing a size.
fn parse_gtk_font(raw: &str) -> Option<GtkFont> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (family, size) = trimmed.rsplit_once(char::is_whitespace)?;
    let size_pt: f32 = size.parse().ok()?;
    if !(size_pt.is_finite() && size_pt > 0.0) {
        return None;
    }
    let family = family.trim();
    if family.is_empty() {
        return None;
    }
    Some(GtkFont {
        family: family.to_string(),
        size_pt,
    })
}

/// Query the active GTK `gtk-font-name` setting. Must run on the GTK main
/// thread (the setup closure is, since Tauri has already called
/// `gtk::init` by then).
#[cfg(target_os = "linux")]
fn query_gtk_font() -> Option<GtkFont> {
    use gtk::prelude::GtkSettingsExt;
    let Some(settings) = gtk::Settings::default() else {
        tracing::warn!("gtk::Settings::default() returned None; base font will fall back to browser default");
        return None;
    };
    let Some(name) = settings.gtk_font_name() else {
        tracing::warn!("gtk-font-name is unset; base font will fall back to browser default");
        return None;
    };
    let raw = name.as_str();
    match parse_gtk_font(raw) {
        Some(font) => {
            tracing::info!(raw = raw, family = %font.family, size_pt = font.size_pt, "parsed GTK font");
            Some(font)
        }
        None => {
            tracing::warn!(
                raw = raw,
                "failed to parse GTK font name; expected form `<family> <size_pt>`"
            );
            None
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn query_gtk_font() -> Option<GtkFont> {
    None
}

pub fn run(cfg: Config, args: DaemonArgs) -> Result<()> {
    let socket_path = args
        .socket
        .or_else(|| cfg.daemon.socket.clone())
        .unwrap_or_else(paths::socket_path);

    info!(socket = %socket_path.display(), "starting hyprpilot daemon");

    // Prepare + bind the socket before the Tauri builder so a failure aborts
    // `run` with an Err — the daemon never opens a window with a broken
    // control surface. Stale-socket detection: we probe with `connect()` and
    // only remove on `ECONNREFUSED`, refusing to clobber anything that's
    // actively listening (e.g. an errant `HYPRPILOT_SOCKET=/var/run/...`).
    let listener = tauri::async_runtime::block_on(async {
        if let Some(parent) = socket_path.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }

        match UnixStream::connect(&socket_path).await {
            Ok(_) => bail!(
                "socket {} is already in use by another process — refusing to clobber it",
                socket_path.display()
            ),
            Err(e) if e.kind() == ErrorKind::ConnectionRefused => {
                tokio::fs::remove_file(&socket_path)
                    .await
                    .with_context(|| format!("failed to remove stale socket at {}", socket_path.display()))?;
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {
                // No prior socket file — nothing to clean up.
            }
            Err(e) => bail!("socket path {} is not accessible: {e}", socket_path.display()),
        }

        UnixListener::bind(&socket_path).with_context(|| format!("failed to bind socket at {}", socket_path.display()))
    })?;

    info!(socket = %socket_path.display(), "socket bound");

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

    // The window starts visible (`true`) because each mode's setup code calls
    // `show()` / `show_all()` before the RPC loop accepts connections.
    let status = Arc::new(StatusBroadcast::new(true));
    let dispatcher = Arc::new(RpcDispatcher::with_defaults());
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

    // Build the renderer from the resolved config and register it in managed
    // state so the RPC toggle handler can re-resolve dimensions against the
    // active monitor on every show transition.
    let renderer = WindowRenderer::new(window_cfg.clone(), wm::detect());

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
            get_gtk_font,
            adapter_commands::session_submit,
            adapter_commands::session_cancel,
            adapter_commands::agents_list,
            adapter_commands::profiles_list,
            adapter_commands::session_list,
            adapter_commands::session_load,
            adapter_commands::permission_reply,
        ])
        .setup(move |app| {
            app.manage(theme.clone());
            app.manage(keymaps.clone());
            app.manage(window_state.clone());
            app.manage(renderer.clone());
            // GTK is initialized by Tauri before the setup closure fires, so
            // gtk::Settings::default() is safe to call here. Queried once at
            // boot rather than on every `get_gtk_font` tick — the user would
            // need to relaunch the overlay to pick up a desktop font change.
            let gtk_font = query_gtk_font();
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
            // window with a critical assertion. `apply_initial` configures the
            // mode-specific surface and maps the window once ready.
            renderer.apply_initial(&main)?;

            // Page zoom per the user's desktop font size. Linear ramp with
            // 10pt as the 1.0 baseline: 11pt → 1.1×, 12pt → 1.2×. Unlike
            // `html { font-size }`, `set_zoom` scales text + layout together
            // (Chromium-style page zoom via WebKit's `set_zoom_level`) — no
            // fonts-bigger-but-margins-fixed breakage that plain font-scale
            // causes on WebKitGTK.
            if let Some(f) = &gtk_font {
                let zoom = 1.0_f64 + (f64::from(f.size_pt) - 10.0) * 0.1;
                let zoom = zoom.clamp(0.5, 2.0);
                if let Err(err) = main.set_zoom(zoom) {
                    warn!(?err, zoom, "failed to apply GTK-font page zoom");
                } else {
                    info!(zoom, size_pt = f.size_pt, "applied GTK-font page zoom");
                }
            }

            app.manage(acp_adapter.clone());
            app.manage(permissions.clone());
            acp_adapter.spawn_tauri_event_bridge(app.handle().clone());

            let rpc_state = crate::rpc::RpcState {
                app: app.handle().clone(),
                status: status.clone(),
                dispatcher: dispatcher.clone(),
                adapter: adapter.clone(),
                acp_adapter: acp_adapter.clone(),
                config: shared_config.clone(),
            };

            // SIGINT / SIGTERM → same shutdown path as daemon/kill.
            // Second signal falls through to the default handler
            // (force-kill), so SIGINT-twice escapes a stuck shutdown.
            let signal_app = app.handle().clone();
            let signal_adapter = acp_adapter.clone();
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
                shutdown(&signal_app, signal_adapter.as_ref()).await;
            });

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

#[derive(Clone, serde::Serialize)]
struct SingleInstancePayload {
    argv: Vec<String>,
    cwd: String,
}

/// Drain adapter instances, then kick Tauri's teardown. Called by
/// `rpc::server` on `daemon/kill` and by the signal task in `run()`.
/// Socket file is not removed — next-start probes stale sockets via
/// `ECONNREFUSED`, which handles crash cases too.
pub(crate) async fn shutdown(app: &tauri::AppHandle, adapter: &AcpAdapter) {
    info!("shutdown: initiating clean shutdown");
    adapter.shutdown_all().await;
    info!("shutdown: adapter instances drained");
    app.exit(0);
    info!("shutdown: tauri exit dispatched");
}

#[cfg(test)]
mod tests {
    use super::parse_gtk_font;

    #[test]
    fn parses_simple_family_and_size() {
        let f = parse_gtk_font("Inter 10").unwrap();
        assert_eq!(f.family, "Inter");
        assert_eq!(f.size_pt, 10.0);
    }

    #[test]
    fn parses_multi_word_family() {
        let f = parse_gtk_font("JetBrains Mono Bold 11").unwrap();
        assert_eq!(f.family, "JetBrains Mono Bold");
        assert_eq!(f.size_pt, 11.0);
    }

    #[test]
    fn parses_fractional_size() {
        let f = parse_gtk_font("Sans 10.5").unwrap();
        assert_eq!(f.family, "Sans");
        assert_eq!(f.size_pt, 10.5);
    }

    #[test]
    fn rejects_empty_input() {
        assert!(parse_gtk_font("").is_none());
        assert!(parse_gtk_font("   ").is_none());
    }

    #[test]
    fn rejects_missing_size() {
        assert!(parse_gtk_font("Inter").is_none());
    }

    #[test]
    fn rejects_non_numeric_trailing_token() {
        assert!(parse_gtk_font("Inter regular").is_none());
    }

    #[test]
    fn rejects_non_positive_size() {
        assert!(parse_gtk_font("Inter 0").is_none());
        assert!(parse_gtk_font("Inter -5").is_none());
    }
}

mod renderer;
mod wm;
pub use renderer::WindowRenderer;

use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use clap::Args;
use tauri::{Emitter, Manager, RunEvent, State};
use tokio::net::{UnixListener, UnixStream};
use tracing::{info, warn};

use crate::acp::AcpSessions;
use crate::config::{Config, Edge, Theme, Window, WindowMode};
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

pub fn run(cfg: Config, args: DaemonArgs) -> Result<()> {
    let socket_path = args.socket.or(cfg.daemon.socket).unwrap_or_else(paths::socket_path);

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
    let window_cfg: Window = cfg.daemon.window.clone();
    let agents_cfg = cfg.agents.clone();

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
    // Session registry — Tauri managed state. `SessionHandler` +
    // future `acp_*` Tauri commands both reach into this.
    let sessions = Arc::new(AcpSessions::new(agents_cfg, status.clone()));

    // Build the renderer from the resolved config and register it in managed
    // state so the RPC toggle handler can re-resolve dimensions against the
    // active monitor on every show transition.
    let renderer = WindowRenderer::new(window_cfg.clone(), wm::detect());

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            info!(?argv, ?cwd, "second instance attempted — forwarding to primary");
            if let Err(err) = app.emit("single-instance", SingleInstancePayload { argv, cwd }) {
                warn!(%err, "failed to emit single-instance event");
            }
        }))
        .invoke_handler(tauri::generate_handler![get_theme, get_window_state])
        .setup(move |app| {
            app.manage(theme.clone());
            app.manage(window_state.clone());
            app.manage(renderer.clone());

            let main = app
                .get_webview_window("main")
                .context("main webview window missing from tauri.conf.json")?;

            // The main window is created invisible in tauri.conf.json so that
            // `init_layer_shell()` can run before the surface is mapped — the
            // Wayland compositor rejects layer-shell init on an already-mapped
            // window with a critical assertion. `apply_initial` configures the
            // mode-specific surface and maps the window once ready.
            renderer.apply_initial(&main)?;

            app.manage(sessions.clone());

            let rpc_state = crate::rpc::RpcState {
                app: app.handle().clone(),
                status: status.clone(),
                dispatcher: dispatcher.clone(),
                sessions: sessions.clone(),
            };

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

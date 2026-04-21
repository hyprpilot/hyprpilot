use std::io::ErrorKind;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::Args;
use tauri::{Emitter, Manager, RunEvent, State};
use tokio::net::{UnixListener, UnixStream};
use tracing::{info, warn};

use crate::config::{Config, Theme};
use crate::paths;

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

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            info!(?argv, ?cwd, "second instance attempted — forwarding to primary");
            if let Err(err) = app.emit("single-instance", SingleInstancePayload { argv, cwd }) {
                warn!(%err, "failed to emit single-instance event");
            }
        }))
        .invoke_handler(tauri::generate_handler![get_theme])
        .setup(move |app| {
            app.manage(theme.clone());

            tauri::async_runtime::spawn(async move {
                loop {
                    match listener.accept().await {
                        Ok((_stream, _addr)) => {
                            info!("accepted socket connection (dropping — protocol not wired)");
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

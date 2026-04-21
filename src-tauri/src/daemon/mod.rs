use std::io::ErrorKind;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::Args;
#[cfg(target_os = "linux")]
use gtk_layer_shell::LayerShell;
use tauri::{Emitter, LogicalSize, Manager, Monitor, PhysicalPosition, PhysicalSize, RunEvent, State};
use tokio::net::{UnixListener, UnixStream};
use tracing::{info, warn};

use crate::config::{AnchorWindow, CenterWindow, Config, Dimension, Edge, Theme, Window, WindowMode};
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

/// Apply the resolved `[daemon.window]` config to the main Tauri window.
///
/// `anchor` mode takes the window's underlying `gtk::ApplicationWindow` and
/// turns it into a `zwlr_layer_shell_v1` surface pinned to one edge. `center`
/// mode leaves the window as a normal top-level, sized as a fraction of the
/// target monitor and centered. Both `layer = overlay` and
/// `keyboard_interactivity = on_demand` are hardcoded — see CLAUDE.md for why.
fn apply_window_config(window: &tauri::WebviewWindow, cfg: &Window) -> Result<()> {
    let mode = cfg.mode.unwrap_or(WindowMode::Anchor);

    match mode {
        WindowMode::Anchor => apply_anchor_mode(window, &cfg.anchor, cfg.output.as_deref()),
        WindowMode::Center => apply_center_mode(window, &cfg.center, cfg.output.as_deref()),
    }
}

#[cfg(target_os = "linux")]
fn apply_anchor_mode(window: &tauri::WebviewWindow, anchor: &AnchorWindow, output: Option<&str>) -> Result<()> {
    use gtk::prelude::{GtkWindowExt, WidgetExt};
    use gtk_layer_shell::{Edge as GtkEdge, KeyboardMode, Layer};

    let edge = anchor.edge.unwrap_or(Edge::Right);
    let margin = anchor.margin.unwrap_or(0);

    // Percentage dimensions need the active monitor's extent. Resolving the
    // monitor here also lets gdk_monitor_by_name below pick the right output
    // in lockstep.
    let monitor = resolve_monitor(window, output)?;
    let mon_size = monitor.size();
    let width_px = resolve_dimension(anchor.width.unwrap_or(Dimension::Percent(40)), mon_size.width);
    let height_px = anchor.height.map(|h| resolve_dimension(h, mon_size.height));

    let gtk_window = window
        .gtk_window()
        .context("failed to obtain gtk::ApplicationWindow for main")?;

    // Layer-shell init must precede map. Tauri creates the window before the
    // setup closure fires, so if `visible = true` we'd already be mapped
    // here — `tauri.conf.json` sets `visible = false` to keep the window
    // unrealized until this code maps it via `show_all` below.
    gtk_window.hide();
    gtk_window.init_layer_shell();
    gtk_window.set_layer(Layer::Overlay);
    gtk_window.set_keyboard_mode(KeyboardMode::OnDemand);
    gtk_window.set_namespace("hyprpilot");

    // Reset all anchors, then pin the configured edge. When height is unset
    // the surface also pins top + bottom so the compositor stretches it
    // full-height — the default overlay shape.
    for &e in &[GtkEdge::Top, GtkEdge::Right, GtkEdge::Bottom, GtkEdge::Left] {
        gtk_window.set_anchor(e, false);
    }
    gtk_window.set_anchor(gtk_edge(edge), true);
    if height_px.is_none() {
        gtk_window.set_anchor(GtkEdge::Top, true);
        gtk_window.set_anchor(GtkEdge::Bottom, true);
    }
    gtk_window.set_layer_shell_margin(gtk_edge(edge), margin);

    if let Some(name) = output {
        if let Some(monitor) = gdk_monitor_by_name(name) {
            gtk_window.set_monitor(&monitor);
        } else {
            warn!(%name, "configured output not found — compositor will pick a monitor");
        }
    }

    // gtk-layer-shell ignores GTK resize flags on layer surfaces — fixed size
    // is how we enforce the surface's extent. Passing -1 for height lets the
    // top+bottom anchors drive full-height fill.
    let request_height = height_px.map(|h| h as i32).unwrap_or(-1);
    gtk_window.set_size_request(width_px as i32, request_height);

    // `visible = false` in tauri.conf.json combined with the `hide()` above
    // keeps the GTK window unmapped until `init_layer_shell` has configured
    // the layer-shell role. `show_all` then maps it via the layer-shell
    // protocol instead of xdg_shell.
    gtk_window.show_all();
    gtk_window.present();

    // `init_layer_shell()` flips the GTK flag unconditionally; the compositor
    // only honors the role after `present()` commits the surface. Read the
    // flag here — pre-present it always reports true, including on
    // compositors without `wlr_layer_shell_v1` (GNOME, KDE), hiding a silent
    // degradation to a regular xdg_shell top-level.
    if gtk_window.is_layer_window() {
        info!(
            ?edge,
            margin,
            width = width_px,
            height = ?height_px,
            output = ?output,
            "anchored layer-shell surface configured"
        );
    } else {
        warn!(
            "compositor did not accept the layer-shell role — falling back to a regular xdg_shell surface. \
             Set `[daemon.window] mode = \"center\"` in your config if your compositor (e.g. GNOME, KDE) does not implement zwlr_layer_shell_v1."
        );
    }

    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn apply_anchor_mode(_window: &tauri::WebviewWindow, _anchor: &AnchorWindow, _output: Option<&str>) -> Result<()> {
    anyhow::bail!("anchor mode requires Linux + zwlr_layer_shell_v1; set [daemon.window] mode = \"center\"")
}

fn apply_center_mode(window: &tauri::WebviewWindow, center: &CenterWindow, output: Option<&str>) -> Result<()> {
    let monitor = resolve_monitor(window, output)?;
    let (w_px, h_px) = center_pixel_size(&monitor, center);

    let scale = monitor.scale_factor();
    window
        .set_size(LogicalSize::new(w_px as f64 / scale, h_px as f64 / scale))
        .context("failed to set window size")?;

    // Compute center within the target monitor — Tauri's `.center()` uses the
    // monitor the window currently sits on, which may not be `output` yet.
    let mon_size = monitor.size();
    let mon_pos = monitor.position();
    let x = mon_pos.x + ((mon_size.width as i32 - w_px as i32) / 2).max(0);
    let y = mon_pos.y + ((mon_size.height as i32 - h_px as i32) / 2).max(0);
    window
        .set_position(PhysicalPosition::new(x, y))
        .context("failed to position window")?;

    window
        .show()
        .context("failed to show main window after center-mode layout")?;

    info!(
        width_px = w_px,
        height_px = h_px,
        monitor = ?monitor.name(),
        "centered window configured"
    );

    Ok(())
}

fn resolve_monitor(window: &tauri::WebviewWindow, output: Option<&str>) -> Result<Monitor> {
    if let Some(target) = output {
        for m in window.available_monitors().context("list monitors")? {
            if m.name().map(String::as_str) == Some(target) {
                return Ok(m);
            }
        }
        warn!(%target, "configured output not found — falling back to primary monitor");
    }

    window
        .primary_monitor()
        .context("query primary monitor")?
        .or_else(|| window.available_monitors().ok().and_then(|mut v| v.pop()))
        .context("no monitors available")
}

fn center_pixel_size(monitor: &Monitor, center: &CenterWindow) -> (u32, u32) {
    let PhysicalSize { width, height } = *monitor.size();

    let w = resolve_dimension(center.width.unwrap_or(Dimension::Percent(50)), width);
    let h = resolve_dimension(center.height.unwrap_or(Dimension::Percent(50)), height);

    (w, h)
}

fn resolve_dimension(dim: Dimension, monitor_extent: u32) -> u32 {
    match dim {
        Dimension::Pixels(px) => px,
        Dimension::Percent(pct) => monitor_extent * pct as u32 / 100,
    }
}

#[cfg(target_os = "linux")]
fn gtk_edge(edge: Edge) -> gtk_layer_shell::Edge {
    use gtk_layer_shell::Edge as G;
    match edge {
        Edge::Top => G::Top,
        Edge::Right => G::Right,
        Edge::Bottom => G::Bottom,
        Edge::Left => G::Left,
    }
}

#[cfg(target_os = "linux")]
fn gdk_monitor_by_name(target: &str) -> Option<gdk::Monitor> {
    use gdk::prelude::*;

    let display = gdk::Display::default()?;
    for i in 0..display.n_monitors() {
        if let Some(m) = display.monitor(i) {
            if m.model().as_deref() == Some(target) {
                return Some(m);
            }
        }
    }
    None
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
    let window_cfg = cfg.daemon.window.clone();

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

            let main = app
                .get_webview_window("main")
                .context("main webview window missing from tauri.conf.json")?;

            // The main window is created invisible in tauri.conf.json so that
            // `init_layer_shell()` can run before the surface is mapped — the
            // Wayland compositor rejects layer-shell init on an already-mapped
            // window with a critical assertion. Each mode's setup shows the
            // window itself once the mode-specific configuration is in place.
            apply_window_config(&main, &window_cfg)?;

            let rpc_state = crate::rpc::RpcState {
                app: app.handle().clone(),
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

//! System tray icon — the captain's "is the daemon alive?" indicator
//! plus a minimal quick-action menu (toggle overlay, shutdown).
//!
//! Built on Tauri 2's core tray support (`tauri::tray::TrayIconBuilder`,
//! `tauri::menu::Menu`). No separate plugin needed.
//!
//! Click on the tray icon → toggle overlay visibility (same path as
//! `overlay/toggle`). Right-click → menu with toggle + shutdown.
//! Explicit `Show overlay` / `Hide overlay` entries were dropped —
//! `Toggle overlay` covers both directions and the captain's mental
//! model is "click tray = flip visibility", not "two separate actions".

use anyhow::{Context, Result};
use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Manager};
use tracing::{info, warn};

use crate::adapters::Adapter;
use crate::daemon::{shutdown, WindowRenderer};
use crate::rpc::protocol::{AgentState, StatusResult};
use crate::rpc::StatusBroadcast;

/// Build the tray icon, attach the menu, and wire the click handlers.
/// Called once from `setup_app`; the icon is owned by Tauri for the
/// lifetime of the app.
pub fn install(app: &App) -> Result<()> {
    let handle = app.handle();

    let toggle_item = MenuItem::with_id(handle, "tray:toggle", "Toggle overlay", true, None::<&str>)
        .context("tray: build toggle item")?;
    let separator = PredefinedMenuItem::separator(handle).context("tray: build separator")?;
    let shutdown_item = MenuItem::with_id(handle, "tray:shutdown", "Shut down", true, None::<&str>)
        .context("tray: build shutdown item")?;

    let menu = Menu::with_items(handle, &[&toggle_item, &separator, &shutdown_item]).context("tray: build menu")?;

    TrayIconBuilder::with_id("hyprpilot-tray")
        .tooltip(tooltip_for(&StatusResult {
            state: AgentState::Idle,
            visible: false,
            active_session: None,
        }))
        .icon(
            app.default_window_icon()
                .cloned()
                .context("default window icon missing")?,
        )
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| handle_menu_event(app, &event))
        .on_tray_icon_event(|tray, event| handle_tray_event(tray.app_handle(), event))
        .build(app)
        .context("tray: build icon")?;

    // Spawn a task that subscribes to StatusBroadcast and pushes
    // every state change into the tray's tooltip. Daemon publishes
    // `idle` / `streaming` / `awaiting` / `error` on every ACP
    // lifecycle transition (see `rpc/status.rs` + the
    // `acp:turn-started` / `acp:turn-ended` / `acp:instance-state`
    // emit sites) plus a `visible` axis flipped by `overlay/*`.
    // Native cross-platform icon-overlay badges aren't a thing
    // (StatusNotifier / NSStatusBar / Win32 tray APIs all lack a
    // standard "dot on the corner" surface), so the tooltip carries
    // the status text. Captain's "if not possible via native tauri
    // forget about it" — tooltip swap IS native, but a colored-dot
    // icon variant would need pre-baked PNGs per state; deferred.
    if let Some(status) = app.try_state::<std::sync::Arc<StatusBroadcast>>() {
        let status = status.inner().clone();
        let handle = app.handle().clone();

        tauri::async_runtime::spawn(async move {
            let (initial, mut rx) = status.subscribe();
            apply_tooltip(&handle, &initial);

            loop {
                match rx.recv().await {
                    Ok(next) => apply_tooltip(&handle, &next),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        // Slow tooltip-update path lost N transitions;
                        // re-pull the current snapshot so we land on
                        // truth instead of staying on a stale one.
                        tracing::warn!(n, "tray: tooltip subscriber lagged");
                        apply_tooltip(&handle, &status.get());
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    } else {
        warn!("tray: StatusBroadcast not in managed state — tooltip will stay static");
    }

    info!("tray: icon installed");
    Ok(())
}

/// One-line tooltip for the current daemon status. Tauri's tray
/// tooltip is the only natively-supported cross-platform status
/// surface — Linux StatusNotifier, macOS NSStatusBar, and Win32
/// tray all expose tooltip text but no badge-overlay primitive.
fn tooltip_for(status: &StatusResult) -> String {
    let phase = match status.state {
        AgentState::Idle => "ready",
        AgentState::Streaming => "working",
        AgentState::Awaiting => "needs permission",
        AgentState::Error => "error",
    };
    let visibility = if status.visible { "shown" } else { "hidden" };
    format!("hyprpilot — {phase} · {visibility}")
}

fn apply_tooltip(app: &AppHandle, status: &StatusResult) {
    let Some(tray) = app.tray_by_id("hyprpilot-tray") else {
        warn!("tray: tooltip update — tray icon missing from registry");
        return;
    };

    if let Err(err) = tray.set_tooltip(Some(tooltip_for(status))) {
        warn!(%err, "tray: tooltip update failed");
    }
}

/// Left-click on the tray icon → toggle overlay. Other mouse events
/// are no-ops (right-click is consumed by the menu's own handler).
fn handle_tray_event(app: &AppHandle, event: TrayIconEvent) {
    if let TrayIconEvent::Click {
        button: MouseButton::Left,
        button_state: MouseButtonState::Up,
        ..
    } = event
    {
        spawn_toggle(app.clone());
    }
}

fn handle_menu_event(app: &AppHandle, event: &MenuEvent) {
    match event.id.as_ref() {
        "tray:toggle" => spawn_toggle(app.clone()),
        "tray:shutdown" => spawn_shutdown(app.clone()),
        other => warn!(menu_id = other, "tray: unknown menu event"),
    }
}

fn spawn_toggle(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        if let Err(err) = toggle(&app).await {
            warn!(%err, "tray: toggle failed");
        }
    });
}

fn spawn_shutdown(app: AppHandle) {
    let adapter = match app.try_state::<std::sync::Arc<dyn Adapter>>() {
        Some(state) => state.inner().clone(),
        None => {
            warn!("tray: adapter not in managed state — calling app.exit directly");
            app.exit(0);
            return;
        }
    };
    tauri::async_runtime::spawn(async move {
        shutdown(&app, adapter.as_ref()).await;
    });
}

async fn toggle(app: &AppHandle) -> Result<()> {
    let renderer = renderer(app)?;
    let window = app.get_webview_window("main").context("main window missing")?;
    let _guard = renderer.lock_present().await;
    let visible = window.is_visible().context("is_visible failed")?;
    if visible {
        renderer.hide_on_main(app, &window).await.context("hide failed")?;
        if let Some(status) = app.try_state::<std::sync::Arc<crate::rpc::StatusBroadcast>>() {
            status.set_visible(false);
        }
    } else {
        renderer.show_on_main(app, &window).await.context("show failed")?;
        let _ = window.set_focus();
        if let Some(status) = app.try_state::<std::sync::Arc<crate::rpc::StatusBroadcast>>() {
            status.set_visible(true);
        }
    }
    Ok(())
}

pub(super) async fn present(app: &AppHandle) -> Result<()> {
    let renderer = renderer(app)?;
    let window = app.get_webview_window("main").context("main window missing")?;
    let _guard = renderer.lock_present().await;
    if !window.is_visible().context("is_visible failed")? {
        renderer.show_on_main(app, &window).await.context("show failed")?;
        if let Some(status) = app.try_state::<std::sync::Arc<crate::rpc::StatusBroadcast>>() {
            status.set_visible(true);
        }
    }
    let _ = window.set_focus();
    Ok(())
}

fn renderer(app: &AppHandle) -> Result<WindowRenderer> {
    Ok(app
        .try_state::<WindowRenderer>()
        .context("WindowRenderer not in managed state")?
        .inner()
        .clone())
}

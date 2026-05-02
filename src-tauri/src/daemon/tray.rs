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
        .tooltip("hyprpilot — running")
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

    info!("tray: icon installed");
    Ok(())
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

use anyhow::{Context, Result};
use tauri::{LogicalSize, Monitor, PhysicalPosition, PhysicalSize, WebviewWindow};
use tracing::{debug, info, warn};

use crate::config::{Dimension, Edge, Window, WindowMode};

/// Owns the resolved `[daemon.window]` config and performs every per-show
/// operation against it: monitor selection, dimension resolution, initial
/// map (anchor/center), and every subsequent re-map when `show` is called.
///
/// Registered in Tauri managed state so the RPC toggle handler can reach it
/// via `state.app.try_state::<WindowRenderer>()`.
#[derive(Debug, Clone)]
pub struct WindowRenderer {
    config: Window,
}

impl WindowRenderer {
    pub fn new(config: Window) -> Self {
        Self { config }
    }

    #[cfg(test)]
    pub fn config(&self) -> &Window {
        &self.config
    }

    /// Resolve a [`Dimension`] to absolute pixels against `monitor_extent`.
    ///
    /// Pure arithmetic, but lives on the renderer since the renderer is the
    /// only caller and it already owns the `Window` config it pulls dims from.
    fn resolve_dim(&self, dim: Dimension, monitor_extent: u32) -> u32 {
        match dim {
            Dimension::Pixels(px) => px,
            Dimension::Percent(pct) => monitor_extent * pct as u32 / 100,
        }
    }

    /// Resolve the pixel width (and optional height) for an anchor-mode
    /// surface against the active monitor for `window`.
    ///
    /// Returns `((width, height_or_none), Monitor)` so callers that also need
    /// the target monitor for positioning (or, today, logging) reuse the same
    /// lookup — between two `resolve_monitor` calls the monitor set can change
    /// (hotplug, resolution change, DPI rescale), which would leave size and
    /// position computed against different monitors.
    ///
    /// Called at setup and again on every show transition so the surface always
    /// reflects the monitor the user is currently on.
    pub fn resolve_anchor_size(&self, window: &WebviewWindow) -> Result<((u32, Option<u32>), Monitor)> {
        let anchor = &self.config.anchor;
        let output = self.config.output.as_deref();
        let monitor = resolve_monitor(window, output)?;
        let PhysicalSize {
            width: mon_w,
            height: mon_h,
        } = *monitor.size();

        let width_px = self.resolve_dim(
            anchor
                .width
                .expect("[daemon.window.anchor] width seeded by defaults.toml"),
            mon_w,
        );
        // `anchor.height` is intentionally optional — None = full-height fill
        // via top+bottom anchor.
        let height_px = anchor.height.map(|h| self.resolve_dim(h, mon_h));

        debug!(
            monitor_name = ?monitor.name(),
            monitor_width = mon_w,
            monitor_height = mon_h,
            scale_factor = monitor.scale_factor(),
            resolved_width_px = width_px,
            resolved_height_px = ?height_px,
            "anchor size resolved"
        );

        Ok(((width_px, height_px), monitor))
    }

    /// Resolve the pixel width and height for a center-mode window against the
    /// active monitor for `window`.
    ///
    /// Returns `((width, height), Monitor)` so `apply_center` reuses the
    /// resolved monitor for positioning instead of calling `resolve_monitor`
    /// a second time — between the two lookups the monitor set can change
    /// (hotplug, Hyprland config hot-reload), leaving size and position
    /// computed against different outputs.
    ///
    /// Called at setup and again on every show transition.
    pub fn resolve_center_size(&self, window: &WebviewWindow) -> Result<((u32, u32), Monitor)> {
        let center = &self.config.center;
        let output = self.config.output.as_deref();
        let monitor = resolve_monitor(window, output)?;
        let PhysicalSize {
            width: mon_w,
            height: mon_h,
        } = *monitor.size();

        let w_px = self.resolve_dim(
            center
                .width
                .expect("[daemon.window.center] width seeded by defaults.toml"),
            mon_w,
        );
        let h_px = self.resolve_dim(
            center
                .height
                .expect("[daemon.window.center] height seeded by defaults.toml"),
            mon_h,
        );

        debug!(
            monitor_name = ?monitor.name(),
            monitor_width = mon_w,
            monitor_height = mon_h,
            scale_factor = monitor.scale_factor(),
            resolved_width_px = w_px,
            resolved_height_px = h_px,
            "center size resolved"
        );

        Ok(((w_px, h_px), monitor))
    }

    /// Called from the setup closure once per process; dispatches to the
    /// correct per-mode apply. The window is already unrealized at this point
    /// (tauri.conf.json: `visible = false`); each mode's implementation maps
    /// it once the mode-specific configuration is in place.
    pub fn apply_initial(&self, window: &WebviewWindow) -> Result<()> {
        let mode = self.config.mode.expect("[daemon.window] mode seeded by defaults.toml");
        match mode {
            WindowMode::Anchor => self.apply_anchor(window),
            WindowMode::Center => self.apply_center(window),
        }
    }

    /// Called from the `toggle` (and future) show paths. Re-resolves
    /// dimensions against the current monitor, then sizes and maps the window
    /// in the mode-appropriate way.
    pub fn show(&self, window: &WebviewWindow) -> Result<()> {
        let mode = self.config.mode.expect("[daemon.window] mode seeded by defaults.toml");
        match mode {
            WindowMode::Anchor => self.apply_anchor(window),
            WindowMode::Center => self.apply_center(window),
        }
    }

    /// Hide the window. Provided here so the toggle handler has a single
    /// façade and never bypasses the renderer.
    pub fn hide(&self, window: &WebviewWindow) -> Result<()> {
        window.hide().context("failed to hide main window")
    }

    // -------------------------------------------------------------------------
    // Per-mode helpers — private; `apply_initial` and `show` fan out to these.
    // -------------------------------------------------------------------------

    #[cfg(target_os = "linux")]
    fn apply_anchor(&self, window: &WebviewWindow) -> Result<()> {
        use gtk::prelude::{GtkWindowExt, WidgetExt};
        use gtk_layer_shell::{Edge as GtkEdge, KeyboardMode, Layer, LayerShell};

        let anchor = &self.config.anchor;
        let edge = anchor
            .edge
            .expect("[daemon.window.anchor] edge seeded by defaults.toml");
        let margin = anchor
            .margin
            .expect("[daemon.window.anchor] margin seeded by defaults.toml");

        // Resolve percent dimensions against the active monitor. The same
        // call is made on every subsequent show transition so the size always
        // reflects the current output. The returned monitor is reused below
        // to pin the GTK layer-shell surface to the matching gdk::Monitor —
        // same authoritative monitor for both the sizing math and the GDK
        // hop, no second `resolve_monitor` call.
        let ((width_px, height_px), monitor) = self
            .resolve_anchor_size(window)
            .context("anchor size resolution failed")?;

        let gtk_window = window
            .gtk_window()
            .context("failed to obtain gtk::ApplicationWindow for main")?;

        // Layer-shell init must precede map. Tauri creates the window before
        // the setup closure fires, so if `visible = true` we'd already be
        // mapped here — `tauri.conf.json` sets `visible = false` to keep the
        // window unrealized until this code maps it via `show_all` below.
        gtk_window.hide();
        gtk_window.init_layer_shell();
        gtk_window.set_layer(Layer::Overlay);
        gtk_window.set_keyboard_mode(KeyboardMode::OnDemand);
        gtk_window.set_namespace("hyprpilot");

        // Reset all anchors, then pin the configured edge. When height is
        // unset the surface also pins top + bottom so the compositor stretches
        // it full-height — the default overlay shape.
        for &e in &[GtkEdge::Top, GtkEdge::Right, GtkEdge::Bottom, GtkEdge::Left] {
            gtk_window.set_anchor(e, false);
        }
        gtk_window.set_anchor(gtk_edge(edge), true);
        if height_px.is_none() {
            gtk_window.set_anchor(GtkEdge::Top, true);
            gtk_window.set_anchor(GtkEdge::Bottom, true);
        }
        gtk_window.set_layer_shell_margin(gtk_edge(edge), margin);

        // Pin the layer-shell surface to the same monitor the size was
        // computed against. We look up the gdk::Monitor by geometry rather
        // than name because gdk 0.18 (GTK3) only exposes model/manufacturer
        // strings on `gdk::Monitor`, not the connector name that `output`
        // uses and that Tauri's `Monitor::name()` returns. Connector names
        // are the authoritative identifier (stable across reboots; models
        // can collide across identical displays); matching by geometry
        // lets both paths agree on which monitor without reinterpreting
        // `output` against two different string namespaces.
        //
        // The GTK4 binding (`gtk4-layer-shell`) exposes
        // `gdk::Monitor::connector()` directly; swap to it when the Tauri
        // upstream lands its GTK4 migration (see CLAUDE.md runway).
        if self.config.output.is_some() {
            if let Some(gdk_monitor) = gdk_monitor_for(&monitor) {
                gtk_window.set_monitor(&gdk_monitor);
            } else {
                warn!(
                    name = ?monitor.name(),
                    "could not map resolved monitor to a gdk::Monitor — compositor will pick a monitor"
                );
            }
        }

        // gtk-layer-shell ignores GTK resize flags on layer surfaces — fixed
        // size is how we enforce the surface extent. Passing -1 for height
        // lets the top+bottom anchors drive full-height fill.
        let request_height = height_px.map(|h| h as i32).unwrap_or(-1);
        gtk_window.set_size_request(width_px as i32, request_height);

        // `visible = false` in tauri.conf.json combined with `hide()` above
        // keeps the GTK window unmapped until `init_layer_shell` has
        // configured the layer-shell role. `show_all` then maps it via the
        // layer-shell protocol instead of xdg_shell.
        gtk_window.show_all();
        gtk_window.present();

        // `init_layer_shell()` flips the GTK flag unconditionally; the
        // compositor only honors the role after `present()` commits the
        // surface. Read the flag here — pre-present it always reports true,
        // including on compositors without `wlr_layer_shell_v1` (GNOME, KDE),
        // hiding a silent degradation to a regular xdg_shell top-level.
        if gtk_window.is_layer_window() {
            info!(
                ?edge,
                margin,
                width = width_px,
                height = ?height_px,
                output = ?self.config.output,
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
    fn apply_anchor(&self, _window: &WebviewWindow) -> Result<()> {
        anyhow::bail!("anchor mode requires Linux + zwlr_layer_shell_v1; set [daemon.window] mode = \"center\"")
    }

    fn apply_center(&self, window: &WebviewWindow) -> Result<()> {
        // Resolve percent dimensions *and* pick the target monitor in one
        // step — the same call is made on every subsequent show transition.
        // The returned monitor is reused for positioning so a hotplug /
        // config-reload between two `resolve_monitor` lookups can't leave
        // the window sized against monitor A and centered on monitor B.
        let ((w_px, h_px), monitor) = self
            .resolve_center_size(window)
            .context("center size resolution failed")?;
        let scale = monitor.scale_factor();

        window
            .set_size(LogicalSize::new(w_px as f64 / scale, h_px as f64 / scale))
            .context("failed to set window size")?;

        // Compute center within the target monitor — Tauri's `.center()` uses
        // the monitor the window currently sits on, which may not be `output`
        // yet.
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
}

// ---------------------------------------------------------------------------
// Shared monitor helpers — used by both anchor and center paths.
// ---------------------------------------------------------------------------

pub(super) fn resolve_monitor(window: &WebviewWindow, output: Option<&str>) -> Result<Monitor> {
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

#[cfg(target_os = "linux")]
pub(super) fn gtk_edge(edge: Edge) -> gtk_layer_shell::Edge {
    use gtk_layer_shell::Edge as G;
    match edge {
        Edge::Top => G::Top,
        Edge::Right => G::Right,
        Edge::Bottom => G::Bottom,
        Edge::Left => G::Left,
    }
}

/// Find the [`gdk::Monitor`] that corresponds to a Tauri-resolved monitor by
/// matching on geometry.
///
/// gdk 0.18 (GTK3) doesn't expose `Monitor::connector()` — only
/// `manufacturer()` / `model()`, which are human-readable display strings
/// (`"LG HDR 4K"`), not the connector names (`"DP-1"`, `"HDMI-A-1"`) that
/// `[daemon.window] output` uses and that Tauri's `Monitor::name()` returns.
/// Using `.model()` as a connector string was a latent misidentification bug
/// — any two configs pointing at the same connector would either agree or
/// silently mismatch depending on the driver.
///
/// Geometry (origin + size) is the next-best shared identifier: it's
/// unambiguous across a single logical compositor state, and both bindings
/// read it from the same Wayland output. gdk's geometry is in logical
/// (scaled) pixels while Tauri's `size()`/`position()` are physical, so we
/// scale gdk's rectangle up by `scale_factor()` before comparing. The compare
/// is exact because both bindings derive their values from the same
/// `wl_output` events — no rounding is introduced.
#[cfg(target_os = "linux")]
pub(super) fn gdk_monitor_for(target: &Monitor) -> Option<gdk::Monitor> {
    use gdk::prelude::*;

    let display = gdk::Display::default()?;
    let target_pos = target.position();
    let target_size = target.size();
    for i in 0..display.n_monitors() {
        let Some(m) = display.monitor(i) else { continue };
        let geom = m.geometry();
        let scale = m.scale_factor();
        let gdk_x = geom.x() * scale;
        let gdk_y = geom.y() * scale;
        let gdk_w = geom.width() as i64 * scale as i64;
        let gdk_h = geom.height() as i64 * scale as i64;
        if gdk_x == target_pos.x
            && gdk_y == target_pos.y
            && gdk_w == target_size.width as i64
            && gdk_h == target_size.height as i64
        {
            return Some(m);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AnchorWindow, CenterWindow, Dimension, Edge, Window, WindowMode};

    fn make_anchor_config() -> Window {
        Window {
            mode: Some(WindowMode::Anchor),
            output: None,
            anchor: AnchorWindow {
                edge: Some(Edge::Right),
                margin: Some(0),
                width: Some(Dimension::Percent(40)),
                height: None,
            },
            center: CenterWindow {
                width: Some(Dimension::Percent(50)),
                height: Some(Dimension::Percent(60)),
            },
        }
    }

    #[test]
    fn renderer_config_roundtrip() {
        let cfg = make_anchor_config();
        let renderer = WindowRenderer::new(cfg.clone());
        assert_eq!(renderer.config(), &cfg);
    }

    // ---------------------------------------------------------------------------
    // resolve_dim tests — pure arithmetic; relocated from config/mod.rs since
    // the renderer is the only caller.
    // ---------------------------------------------------------------------------

    #[test]
    fn resolve_dim_percent_40_of_1920() {
        assert_eq!(
            WindowRenderer::new(make_anchor_config()).resolve_dim(Dimension::Percent(40), 1920),
            768
        );
    }

    #[test]
    fn resolve_dim_percent_100_of_1920() {
        assert_eq!(
            WindowRenderer::new(make_anchor_config()).resolve_dim(Dimension::Percent(100), 1920),
            1920
        );
    }

    #[test]
    fn resolve_dim_percent_0_of_1920() {
        // Percent(0) is rejected by validate_dimension (must be 1..=100), but
        // the arithmetic itself must be defined and return 0 — no panic.
        assert_eq!(
            WindowRenderer::new(make_anchor_config()).resolve_dim(Dimension::Percent(0), 1920),
            0
        );
    }

    #[test]
    fn resolve_dim_pixels_passthrough() {
        assert_eq!(
            WindowRenderer::new(make_anchor_config()).resolve_dim(Dimension::Pixels(480), 1920),
            480
        );
        assert_eq!(
            WindowRenderer::new(make_anchor_config()).resolve_dim(Dimension::Pixels(1), 9999),
            1
        );
    }

    #[test]
    fn resolve_dim_percent_against_different_extents() {
        // 40% of a 2560-wide monitor
        assert_eq!(
            WindowRenderer::new(make_anchor_config()).resolve_dim(Dimension::Percent(40), 2560),
            1024
        );
        // 50% of a 1080-tall monitor
        assert_eq!(
            WindowRenderer::new(make_anchor_config()).resolve_dim(Dimension::Percent(50), 1080),
            540
        );
    }
}

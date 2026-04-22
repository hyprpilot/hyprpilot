import { invoke } from '@ipc'

export type WindowMode = 'anchor' | 'center'
export type Edge = 'top' | 'right' | 'bottom' | 'left'

/**
 * Snapshot of the daemon's resolved `[daemon.window]` state. Mirrors
 * `src-tauri/src/daemon/mod.rs::WindowState`. `anchorEdge` is the edge the
 * layer-shell surface is pinned to in anchor mode; absent in center mode
 * (no screen-edge-relative chrome should render).
 */
export interface WindowState {
  mode: WindowMode
  anchorEdge?: Edge
}

/**
 * Fetches the window state and exposes the anchored edge as a
 * `data-window-anchor` attribute on `<html>`. CSS rules keyed on that
 * attribute paint the `--theme-window-edge` accent on the inward (visible)
 * side of the overlay. A missing Tauri host (plain `vite dev` in a browser,
 * vitest jsdom) soft-fails — the attribute stays unset and the edge accent
 * doesn't render, which is the correct no-Rust-context signal.
 */
export async function applyWindowState(): Promise<void> {
  let state: WindowState
  try {
    state = await invoke<WindowState>('get_window_state')
  } catch {
    return
  }

  const root = document.documentElement
  if (state.anchorEdge) {
    root.dataset.windowAnchor = state.anchorEdge
  } else {
    delete root.dataset.windowAnchor
  }
}

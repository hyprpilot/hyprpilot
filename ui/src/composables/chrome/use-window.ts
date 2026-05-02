import { invoke, TauriCommand, type WindowState } from '@ipc'

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
    state = await invoke(TauriCommand.GetWindowState)
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

/**
 * Snapshot of the daemon's resolved `[daemon.window]` state. Mirrors
 * `src-tauri/src/daemon/mod.rs::WindowState`. `anchorEdge` is the edge
 * the layer-shell surface is pinned to in anchor mode; absent in center
 * mode (no screen-edge-relative chrome should render).
 */
import type { Edge, WindowMode } from '@constants/wire/window'

export interface WindowState {
  mode: WindowMode
  anchorEdge?: Edge
}

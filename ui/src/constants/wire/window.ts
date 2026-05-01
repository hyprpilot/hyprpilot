/**
 * Daemon window mode + anchor edge — mirrors
 * `src-tauri/src/config/daemon.rs::WindowMode` / `Edge`.
 */
export enum WindowMode {
  Anchor = 'anchor',
  Center = 'center'
}

export enum Edge {
  Top = 'top',
  Right = 'right',
  Bottom = 'bottom',
  Left = 'left'
}

/**
 * `@ipc` barrel — Tauri bridge functions + re-export of every wire
 * type / constant. Importers reach for `@ipc` for the bridge surface
 * and `@interfaces` / `@constants` for typed shapes; the re-exports
 * here keep existing call sites working through the single barrel.
 */
export * from './bridge'
export * from '@interfaces/wire'
export * from '@constants/wire'

import { FontAwesomeIcon } from '@fortawesome/vue-fontawesome'
import { createApp } from 'vue'

import App from './App.vue'
import { applyTheme, applyWindowState, loadCompletionConfig, loadDaemonCwd, loadHomeDir, loadKeymaps, markBootDone, setBootStatus, startGitStatus } from '@composables'
import { log } from '@lib'
import '@assets/styles.css'

/**
 * Route every uncaught error / unhandled rejection / Vue render
 * error through `log.error` so they land in
 * `$XDG_STATE_HOME/hyprpilot/logs/hyprpilot.log.*` next to the Rust
 * tracing output. Without these hooks an exception thrown inside a
 * Vue handler / async task only surfaces in the webview devtools
 * console — invisible from `tail -f` on the log.
 *
 * Three hooks cover the failure surface:
 *   - `window.addEventListener('error', …)` — synchronous throws
 *     from event handlers, async callbacks, etc.
 *   - `window.addEventListener('unhandledrejection', …)` — promise
 *     rejections with no `.catch`.
 *   - `app.config.errorHandler` — errors thrown inside Vue
 *     render / lifecycle / watch.
 */
function installGlobalErrorBridge(): void {
  window.addEventListener('error', (event) => {
    log.error(
      'uncaught error',
      {
        source: 'window.error',
        filename: event.filename,
        lineno: event.lineno,
        colno: event.colno
      },
      event.error ?? event.message
    )
  })
  window.addEventListener('unhandledrejection', (event) => {
    log.error('unhandled rejection', { source: 'window.unhandledrejection' }, event.reason)
  })
}

// FontAwesome icons land via per-component direct imports
// (`import { faFoo } from '@fortawesome/free-solid-svg-icons'` +
// `<FaIcon :icon="faFoo" />`). No central `library.add(...)` registry
// — the string-array form defeats Vite's tree-shaking + forces every
// icon into the boot bundle. Per CLAUDE.md / AGENTS.md icon rule.

// Apply the palette and anchor-edge attribute before the first render so
// there is no flash of unstyled content. Both soft-fail without a Tauri host.
// Wrapped in an async boot rather than a top-level await to keep the Vite
// `safari13` build target (WebKit2GTK 4.1 webview) happy — TLA there emits a
// "tolerated transform" that can stall the webview under `tauri-plugin-playwright`'s
// eval path.
async function boot(): Promise<void> {
  // Dev preview shim — theme tokens, window-state attribute, mock IPC
  // fixtures. Gated by `VITE_HYPRPILOT_DEV_PREVIEW=1`; production
  // builds + Tauri runs leave it unset and Vite tree-shakes the
  // dynamic import. The shim lives in `tests/` so vitest fixtures and
  // the Vite dev preview share one source of truth — never bundled
  // into production source. Per CLAUDE.md "Rust is the sole source"
  // for theme; this preview is browser-mode only.
  if (import.meta.env.VITE_HYPRPILOT_DEV_PREVIEW === '1') {
    const { applyDevPreview } = await import('../../tests/dev-preview')

    applyDevPreview()
  }

  // Theme + window state apply before mount so there is no FOUC
  // window per CLAUDE.md ("Rust is the sole source; applyTheme runs
  // synchronously in main.ts before createApp().mount('#app')").
  setBootStatus('applying theme')
  await applyTheme()
  setBootStatus('configuring window')
  await applyWindowState()

  // Mount NOW with the fullscreen <Loading> visible so the
  // remaining IPC steps (GTK font probe, $HOME, keymaps) paint
  // their status pills as they progress instead of completing
  // pre-mount and leaving a blank viewport. Without this early
  // mount the user never sees the loading screen — the App.vue
  // first paints AFTER `markBootDone()`, so `done=true` already
  // and the v-if gate evaluates false on first render.
  const app = createApp(App)

  app.component('FaIcon', FontAwesomeIcon)
  app.config.errorHandler = (err, _instance, info) => {
    log.error('vue error', { source: 'vue.errorHandler', info }, err)
  }
  installGlobalErrorBridge()
  app.mount('#app')

  // Step the user through the post-mount boot work. Each
  // setBootStatus updates the live <Loading> status pill before
  // its IPC starts so the active step is visible rather than
  // mysterious dead air.
  setBootStatus('reading $HOME')
  await loadHomeDir()
  setBootStatus('reading daemon cwd')
  await loadDaemonCwd()
  setBootStatus('loading keymaps')
  await loadKeymaps()
  // Apply the captain's configured ripgrep debounce on the
  // composer's auto-trigger path. The daemon-side ripgrep source
  // already honours `auto` / `min_prefix`; only debounce_ms lives
  // UI-side because that's where keystrokes happen.
  await loadCompletionConfig()

  // Watch the active instance's cwd and pull a fresh git-status
  // snapshot on every change — drives the header `branch ↑N ↓M`
  // pill. Idempotent.
  startGitStatus()

  // Flip `bootDone` so the App root can drop the fullscreen
  // overlay. Anything that needs the keymaps (Overlay.vue's keymap
  // dispatcher gates on `keymaps.value`) automatically wakes up
  // through the reactive ref the moment loadKeymaps populates it.
  markBootDone()
}

void boot()

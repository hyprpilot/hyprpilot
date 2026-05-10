import { FontAwesomeIcon } from '@fortawesome/vue-fontawesome'
import { QueryClient, VueQueryPlugin } from '@tanstack/vue-query'
import { createApp } from 'vue'

import App from './App.vue'
import { applyBootSnapshot, applyTheme, applyWindowState, loadCompletionConfig, loadKeymaps, markBootDone, setBootStatus, startGitStatus } from '@composables'
import { ensureRemoteConnection, isRemoteHost, subscribePair } from '@ipc/remote-bridge'
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

/**
 * Resolve once the remote bridge upgrades a pending WS connection
 * to authenticated. Subscribes to `pair` frames; remote-host SPA
 * pauses the IPC-heavy boot steps until the captain confirms on
 * the desktop.
 */
function waitForPairAuthenticated(): Promise<void> {
  return new Promise<void>((resolve) => {
    const unsubscribe = subscribePair((view) => {
      if (view.authenticated) {
        unsubscribe()
        resolve()
      }
    })
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

  // Remote-host (browser hitting the daemon's HTTPS bridge): the
  // bridge gates every `invoke()` on pair authentication. Mount
  // App.vue FIRST — its `<RemotePairScreen>` v-if renders the QR +
  // 4-word code without touching IPC, so the captain has something
  // to scan / read. Theme + window state can't apply yet (Tauri
  // commands), so the screen reads bare-DOM defaults until the WS
  // authenticates and the Tauri-bridged boot steps run.
  const app = createApp(App)

  // TanStack Query backs the per-instance snapshot composables
  // (`useInstanceMetaQuery`, `useInstanceChatInfiniteQuery`,
  // `useInstanceTerminalsQuery`). One QueryClient is shared across
  // the app — sane defaults for snapshot-shaped data: server is the
  // truth (`staleTime: 0`), keep a couple of pages around for
  // backward pagination but evict promptly when unused
  // (`gcTime: 5min`), and skip Tauri's noisy window-focus refetch
  // (we have explicit `acp:instances-focused` refetch wired
  // separately).
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: {
        staleTime: 0,
        gcTime: 5 * 60 * 1000,
        refetchOnWindowFocus: false
      }
    }
  })

  app.use(VueQueryPlugin, { queryClient })
  app.component('FaIcon', FontAwesomeIcon)
  app.config.errorHandler = (err, _instance, info) => {
    log.error('vue error', { source: 'vue.errorHandler', info }, err)
  }
  installGlobalErrorBridge()

  // Single-RPC boot. `applyBootSnapshot` rolls theme + windowState +
  // keymaps + daemonCwd + completionConfig into one IPC and dispatches
  // each into its existing applier. Replaces the previous 5-await
  // sequence — particularly load-bearing on the remote bridge where
  // each round-trip rides the same WS, so the captain spent up to 5×
  // RTT staring at "configuring window…" while the daemon already had
  // every answer in hand.
  //
  // On Tauri host the snapshot still runs before mount so FOUC stays
  // shut. On remote, mount comes first (the pair screen needs DOM
  // before the WS is up); the snapshot lands after authenticate.
  if (isRemoteHost()) {
    setBootStatus('connecting to daemon…')
    app.mount('#app')
    await ensureRemoteConnection()
    await waitForPairAuthenticated()
    setBootStatus('loading')

    if (!(await applyBootSnapshot(queryClient))) {
      // Fallback path — older daemon binary that doesn't expose
      // `boot_snapshot` yet. Granular loaders soft-fail individually.
      await applyTheme()
      await applyWindowState()
      await loadKeymaps()
      await loadCompletionConfig()
    }
  } else {
    setBootStatus('loading')

    if (!(await applyBootSnapshot(queryClient))) {
      await applyTheme()
      await applyWindowState()
      await loadKeymaps()
      await loadCompletionConfig()
    }
    app.mount('#app')
  }

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

// Drop the curtain even on failure. A `void boot()` would silently
// swallow any uncaught throw inside the boot pipeline (e.g. a bad
// wire shape from an out-of-date daemon), leaving `markBootDone`
// unfired and the captain stuck on the fullscreen <Loading>. Always
// flip `bootDone` so a broken-but-visible UI is preferable to an
// invisible-but-broken one — the captain's reload triggers a fresh
// boot when ready.
boot().catch((err: unknown) => {
  log.error('boot pipeline failed', undefined, err)
  markBootDone()
})

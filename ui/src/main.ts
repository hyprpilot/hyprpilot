import { library } from '@fortawesome/fontawesome-svg-core'
import { faCircle as farCircle, faSquare as farSquare } from '@fortawesome/free-regular-svg-icons'
import {
  faArrowRightToBracket,
  faArrowTurnDown,
  faBrain,
  faChevronDown,
  faChevronRight,
  faCircle,
  faCircleCheck,
  faCircleHalfStroke,
  faCircleInfo,
  faCircleNotch,
  faCircleXmark,
  faCodeBranch,
  faCube,
  faFileLines,
  faMagnifyingGlass,
  faPen,
  faPenToSquare,
  faPlug,
  faReply,
  faSquareCheck,
  faTerminal,
  faTriangleExclamation,
  faUpDown,
  faUserGear,
  faXmark
} from '@fortawesome/free-solid-svg-icons'
import { FontAwesomeIcon } from '@fortawesome/vue-fontawesome'
import { type Component, createApp } from 'vue'

import { applyGtkFont, applyTheme, applyWindowState, loadKeymaps } from '@composables'
import App from './App.vue'
import '@assets/styles.css'

// Per-icon imports (not the full `fas` / `far` packs) — keeps the JS bundle
// below the ~350KB budget. Add new icons to this list when components adopt
// them; vitest.setup.ts mirrors the registrations for test mounts.
library.add(
  faArrowRightToBracket,
  faArrowTurnDown,
  faBrain,
  faChevronDown,
  faChevronRight,
  faCircle,
  faCircleCheck,
  faCircleHalfStroke,
  faCircleInfo,
  faCircleNotch,
  faCircleXmark,
  faCodeBranch,
  faCube,
  faFileLines,
  faMagnifyingGlass,
  faPen,
  faPenToSquare,
  faPlug,
  faReply,
  faSquareCheck,
  faTerminal,
  faTriangleExclamation,
  faUpDown,
  faUserGear,
  faXmark,
  farCircle,
  farSquare
)

// Apply the palette and anchor-edge attribute before the first render so
// there is no flash of unstyled content. Both soft-fail without a Tauri host.
// Wrapped in an async boot rather than a top-level await to keep the Vite
// `safari13` build target (WebKit2GTK 4.1 webview) happy — TLA there emits a
// "tolerated transform" that can stall the webview under `tauri-plugin-playwright`'s
// eval path.
async function boot(): Promise<void> {
  await Promise.all([applyTheme(), applyWindowState(), applyGtkFont(), loadKeymaps()])

  // Dev-only `/#design` route mounts the K-250 fixture showcase instead
  // of the overlay shell. No vue-router dep; hash is checked once at boot.
  let root: Component = App
  if (import.meta.env.DEV && window.location.hash.startsWith('#design')) {
    const mod = await import('@views/design/DesignShowcase.vue')
    root = mod.default
  }

  const app = createApp(root)
  app.component('FaIcon', FontAwesomeIcon)
  app.mount('#app')
}

void boot()

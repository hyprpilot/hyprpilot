import { createApp } from 'vue'

import { applyTheme, applyWindowState } from '@composables'
import App from './App.vue'
import '@assets/styles.css'

// Apply the palette and anchor-edge attribute before the first render so
// there is no flash of unstyled content. Both soft-fail without a Tauri host.
// Wrapped in an async boot rather than a top-level await to keep the Vite
// `safari13` build target (WebKit2GTK 4.1 webview) happy — TLA there emits a
// "tolerated transform" that can stall the webview under `tauri-plugin-playwright`'s
// eval path.
async function boot(): Promise<void> {
  await Promise.all([applyTheme(), applyWindowState()])
  createApp(App).mount('#app')
}

void boot()

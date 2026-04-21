import { createApp } from 'vue'

import { applyTheme, applyWindowState } from '@composables'
import App from './App.vue'
import '@assets/styles.css'

// Apply the palette and anchor-edge attribute before the first render so
// there is no flash of unstyled content. Both soft-fail without a Tauri host.
await Promise.all([applyTheme(), applyWindowState()])

createApp(App).mount('#app')

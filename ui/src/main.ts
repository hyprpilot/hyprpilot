import { createApp } from 'vue'

import { applyTheme } from '@composables'
import App from './App.vue'
import '@assets/styles.css'

// Apply the palette before the first render so there is no flash of
// unstyled content. `applyTheme` soft-fails when there is no Tauri host.
await applyTheme()

createApp(App).mount('#app')

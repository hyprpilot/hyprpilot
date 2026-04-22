import { createTauriTest } from '@srsholmes/tauri-playwright'

const socket = process.env.HYPRPILOT_E2E_SOCKET ?? '/tmp/tauri-playwright.sock'
const devUrl = process.env.HYPRPILOT_E2E_DEV_URL ?? 'http://127.0.0.1:1420'

export const { test, expect } = createTauriTest({
  devUrl,
  mcpSocket: socket,
  ipcMocks: {
    // `get_theme` returns the minimum shape the webview's `applyTheme`
    // walker needs so the Placeholder renders without throwing. The
    // full shape lives in `src-tauri/src/config/defaults.toml`; here we
    // only cover the leaves referenced during render.
    get_theme: () => ({
      font: { family: 'ui-monospace, monospace' },
      window: { default: '#1e2127', edge: '#d3b051' },
      surface: {
        card: { user: { bg: '#2c333d' }, assistant: { bg: '#22282f' } },
        compose: '#2c333d',
        text: '#1e2127'
      },
      fg: { default: '#abb2bf', dim: '#7c8a9d', muted: '#5c6370' },
      border: { default: '#4b5263', soft: '#2c333d', focus: '#6c778d' },
      accent: { default: '#abb2bf', user: '#e5c07b', assistant: '#98c379' },
      state: { idle: '#98c379', stream: '#e5c07b', pending: '#e06c75', awaiting: '#d19a66' }
    }),
    get_window_state: () => ({ mode: 'center', anchorEdge: null })
  }
})

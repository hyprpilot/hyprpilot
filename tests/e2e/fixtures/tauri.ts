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
      window: { default: '#0b0d12', edge: '#c99bf0' },
      surface: {
        card: { user: { bg: '#12141a' }, assistant: { bg: '#12141a' } },
        compose: '#181b22',
        text: '#0b0d12'
      },
      fg: {
        default: '#d8dde5',
        dim: '#6b7280',
        muted: '#3e4454'
      },
      border: {
        default: '#20242e',
        soft: '#2b2f3b',
        focus: '#3e4454'
      },
      accent: {
        default: '#c99bf0',
        user: '#e8c86c',
        assistant: '#8ac9a0'
      },
      state: {
        idle: '#7fcf8a',
        stream: '#e8c86c',
        pending: '#e06f6f',
        awaiting: '#e0a060'
      }
    }),
    get_window_state: () => ({ mode: 'center', anchorEdge: null })
  }
})

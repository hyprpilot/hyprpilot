import { createTauriTest } from '@srsholmes/tauri-playwright'

const socket = process.env.HYPRPILOT_E2E_SOCKET ?? '/tmp/tauri-playwright.sock'
const devUrl = process.env.HYPRPILOT_E2E_DEV_URL ?? 'http://127.0.0.1:1420'

/**
 * Onedark theme fixture — mirrors `src-tauri/src/config/defaults.toml::[ui.theme.*]`.
 * Keep in lockstep with the Rust source. The shape covers every leaf
 * the webview's `applyTheme` walker writes to `:root` so e2e tests
 * see the same palette the production daemon ships.
 */
const ONEDARK_THEME = {
  font: {
    mono: '\'JetBrains Mono\', ui-monospace, \'Fira Code\', Menlo, Consolas, monospace',
    sans: '\'Inter\', ui-sans-serif, system-ui, sans-serif'
  },
  window: { default: '#17191e', edge: '#e5c07b' },
  surface: {
    default: '#1e2127',
    bg: '#17191e',
    alt: '#22282f',
    compose: '#1e2127',
    text: '#17191e',
    card: {
      user: { bg: '#16210f' },
      assistant: { bg: '#29090b' }
    }
  },
  fg: {
    default: '#abb2bf', ink_2: '#979eab', dim: '#7c8a9d', faint: '#5c6370'
  },
  border: {
    default: '#2c333d', soft: '#38404b', focus: '#e5c07b'
  },
  accent: {
    default: '#e5c07b',
    user: '#98c379',
    user_soft: '#16210f',
    assistant: '#e06c75',
    assistant_soft: '#29090b'
  },
  state: {
    idle: '#98c379',
    stream: '#e5c07b',
    working: '#e5c07b',
    awaiting: '#d19a66',
    pending: '#e06c75'
  },
  kind: {
    read: '#61afef',
    write: '#c678dd',
    bash: '#d19a66',
    search: '#56b6c2',
    agent: '#c678dd',
    think: '#5c6370',
    terminal: '#98c379',
    acp: '#98caf6'
  },
  status: {
    ok: '#98c379', warn: '#d19a66', err: '#e06c75'
  },
  permission: { bg: '#2c2009', bg_active: '#3a2a0c' }
}

export const { test, expect } = createTauriTest({
  devUrl,
  mcpSocket: socket,
  ipcMocks: {
    get_theme: () => ONEDARK_THEME,
    get_window_state: () => ({ mode: 'center', anchorEdge: null }),
    get_gtk_font: () => ({}),
    get_home_dir: () => '/home/dev',
    get_keymaps: () => ({
      chat: {
        submit: { modifiers: [], key: 'enter' },
        newline: { modifiers: ['shift'], key: 'enter' }
      },
      approvals: {
        allow: { modifiers: [], key: 'a' },
        deny: { modifiers: [], key: 'd' }
      },
      composer: {
        paste_image: { modifiers: ['ctrl'], key: 'p' },
        tab_completion: { modifiers: [], key: 'tab' },
        shift_tab: { modifiers: ['shift'], key: 'tab' },
        history_up: { modifiers: ['ctrl'], key: 'arrowup' },
        history_down: { modifiers: ['ctrl'], key: 'arrowdown' }
      },
      palette: {
        open: { modifiers: ['ctrl'], key: 'k' },
        close: { modifiers: [], key: 'escape' },
        models: { focus: { modifiers: ['ctrl'], key: 'm' } },
        sessions: { focus: { modifiers: ['ctrl'], key: 's' } }
      },
      transcript: {}
    }),
    profiles_list: () => ({ profiles: [] }),
    agents_list: () => ({ agents: [] }),
    commands_list: () => ({ commands: [] }),
    instances_list: () => ({ instances: [] }),
    sessions_info: () => ({ sessions: [] }),
    skills_list: () => ({ skills: [] }),
    mcps_list: () => ({ mcps: [] })
  }
})

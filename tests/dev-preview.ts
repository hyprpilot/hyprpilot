/**
 * Dev-only preview shims — applied in browser-only contexts where the
 * Tauri host isn't bound (Vite dev, Playwright MCP).
 *
 * Production tauri builds tree-shake this module out via the
 * `import.meta.env.DEV` gate in `main.ts` — the import is dynamic
 * specifically so Vite's dead-code elimination can drop the module
 * graph entirely.
 *
 * Add new dev-only setup steps here (theme tokens, mock fixtures,
 * window-state attributes, IPC stubs) — the file is the single
 * landing zone for "what does the UI need to render plausibly when
 * the daemon isn't running?"
 */

/**
 * Theme tokens mirror of `src-tauri/src/config/defaults.toml::[ui.theme.*]`.
 * Keep in lockstep with the Rust source. The token names map to the
 * CSS variables emitted by `cssVarName(path)` in `use-theme.ts`.
 */
const PREVIEW_THEME_TOKENS: Record<string, string> = {
  '--theme-font-mono': "'JetBrains Mono', ui-monospace, 'Fira Code', Menlo, Consolas, monospace",
  '--theme-font-sans': "'Inter', ui-sans-serif, system-ui, sans-serif",
  '--theme-window': '#17191e',
  '--theme-window-edge': '#e5c07b',
  '--theme-surface': '#1e2127',
  '--theme-surface-bg': '#17191e',
  '--theme-surface-alt': '#22282f',
  '--theme-surface-compose': '#1e2127',
  '--theme-surface-text': '#17191e',
  '--theme-fg': '#abb2bf',
  '--theme-fg-ink-2': '#979eab',
  '--theme-fg-dim': '#7c8a9d',
  '--theme-fg-faint': '#5c6370',
  '--theme-fg-on-tone': '#121212',
  '--theme-border': '#2c333d',
  '--theme-border-soft': '#38404b',
  '--theme-border-focus': '#e5c07b',
  '--theme-accent': '#e5c07b',
  '--theme-accent-user': '#98c379',
  '--theme-accent-user-soft': '#16210f',
  '--theme-accent-assistant': '#e06c75',
  '--theme-accent-assistant-soft': '#29090b',
  '--theme-state-idle': '#98c379',
  '--theme-state-stream': '#e5c07b',
  '--theme-state-working': '#e5c07b',
  '--theme-state-awaiting': '#d19a66',
  '--theme-state-pending': '#e06c75',
  '--theme-kind-read': '#61afef',
  '--theme-kind-write': '#c678dd',
  '--theme-kind-bash': '#d19a66',
  '--theme-kind-search': '#56b6c2',
  '--theme-kind-agent': '#c678dd',
  '--theme-kind-think': '#5c6370',
  '--theme-kind-terminal': '#98c379',
  '--theme-kind-acp': '#98caf6',
  '--theme-status-ok': '#98c379',
  '--theme-status-warn': '#d19a66',
  '--theme-status-err': '#e06c75',
  '--theme-permission-bg': '#2c2009',
  '--theme-permission-bg-active': '#3a2a0c'
}

function applyPreviewTheme(): void {
  const root = document.documentElement
  for (const [varName, value] of Object.entries(PREVIEW_THEME_TOKENS)) {
    root.style.setProperty(varName, value)
    // Mirror `applyTheme`'s `-rgb` companion emission — RGBA-bearing
    // declarations like `.chat-body::before { background: rgba(var(...),
    // .14) }` need this even in browser-mode dev preview, otherwise
    // the role tint disappears.
    const m = /^#([0-9a-fA-F]{6})/.exec(value)
    if (m) {
      const hex = m[1]
      const r = Number.parseInt(hex.slice(0, 2), 16)
      const g = Number.parseInt(hex.slice(2, 4), 16)
      const b = Number.parseInt(hex.slice(4, 6), 16)
      root.style.setProperty(`${varName}-rgb`, `${r}, ${g}, ${b}`)
    }
  }
}

/**
 * Set sane window-state attributes so the body-edge accent rules in
 * `assets/styles.css` paint correctly even without `applyWindowState()`.
 * Defaults to center mode (full perimeter) which mirrors how the
 * overlay floats on non-anchored screens.
 */
function applyPreviewWindowState(): void {
  const root = document.documentElement
  if (!root.hasAttribute('data-window-mode')) {
    root.setAttribute('data-window-mode', 'center')
  }
}

/**
 * Mock Tauri IPC fixtures keyed by command name. Returned to consumers
 * verbatim — keep the shapes faithful to the Rust-side wire types in
 * `src-tauri/src/rpc/handlers/*` and `src-tauri/src/adapters/commands.rs`.
 */
const MOCK_INVOKE_FIXTURES: Record<string, unknown> = {
  get_theme: {},
  get_window_state: { mode: 'center', anchorEdge: undefined },
  get_gtk_font: {},
  get_home_dir: '/home/dev',
  get_keymaps: {
    chat: {
      submit: { modifiers: [], key: 'enter' },
      newline: { modifiers: ['shift'], key: 'enter' }
    },
    approvals: {
      allow: { modifiers: [], key: 'a' },
      deny: { modifiers: [], key: 'd' }
    },
    composer: {
      paste: { modifiers: ['ctrl'], key: 'p' },
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
  },
  profiles_list: {
    profiles: [{ id: 'captain', agent: 'claude-code', model: 'claude-opus-4', isDefault: true }]
  },
  agents_list: { agents: [{ id: 'claude-code', provider: 'acp-claude-code', model: 'claude-opus-4' }] },
  commands_list: { commands: [] },
  instances_list: { instances: [] },
  sessions_info: { sessions: [] },
  skills_list: { skills: [] },
  mcps_list: { mcps: [] }
}

/**
 * Install a minimal `window.__TAURI_INTERNALS__` shim so the
 * `@tauri-apps/api/core` `invoke` / `listen` paths don't throw in
 * browser-only previews. Mock invoke returns the fixture for the
 * known commands and `undefined` otherwise; mock listen registers
 * a no-op unlistener.
 */
function applyPreviewTauriShim(): void {
  if ('__TAURI_INTERNALS__' in window) {
    return
  }
  let nextCallbackId = 1
  const callbacks = new Map<number, (msg: unknown) => void>()
  ;(window as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
    invoke(cmd: string): Promise<unknown> {
      if (cmd in MOCK_INVOKE_FIXTURES) {
        return Promise.resolve(MOCK_INVOKE_FIXTURES[cmd])
      }
      return Promise.resolve(undefined)
    },
    transformCallback(callback: (msg: unknown) => void, _once: boolean): number {
      const id = nextCallbackId
      nextCallbackId += 1
      callbacks.set(id, callback)
      return id
    },
    unregisterCallback(id: number): void {
      callbacks.delete(id)
    },
    convertFileSrc(p: string): string {
      return p
    }
  }
}

/**
 * Boot the dev-only preview surface. Idempotent. Call once before app
 * mount in browser-only / Playwright MCP contexts. Add new shims here
 * (mock IPC fixtures, profile presets, etc.) as the design surface
 * grows.
 */
export function applyDevPreview(): void {
  applyPreviewTheme()
  applyPreviewWindowState()
  applyPreviewTauriShim()
  void exposeDevHelpers()
}

/**
 * Expose `window.__hyprpilot_dev` with helpers that route through the
 * canonical `@composables` module so visual smoke (Playwright MCP)
 * triggers Vue reactivity in the running app — bypasses the HMR
 * dynamic-import duplication that splits module identity when tests
 * try to push state via `import('/src/...')` URLs directly.
 */
async function exposeDevHelpers(): Promise<void> {
  const composables = await import('@composables')
  const types = await import('@components')

  ;(window as unknown as Record<string, unknown>).__hyprpilot_dev = {
    pushToast: composables.pushToast,
    ToastTone: types.ToastTone,
    useToasts: composables.useToasts,
    pushPermissionRequest: composables.pushPermissionRequest,
    pushTurnStarted: composables.pushTurnStarted,
    pushTranscriptChunk: composables.pushTranscriptChunk,
    pushThoughtChunk: composables.pushThoughtChunk,
    pushPlan: composables.pushPlan,
    pushToolCall: composables.pushToolCall,
    pushInstanceState: composables.pushInstanceState,
    pushSessionInfoUpdate: composables.pushSessionInfoUpdate,
    pushCurrentModeUpdate: composables.pushCurrentModeUpdate,
    pushInstanceModeState: composables.pushInstanceModeState,
    pushInstanceModelState: composables.pushInstanceModelState,
    setInstanceCwd: composables.setInstanceCwd,
    setInstanceGitStatus: composables.setInstanceGitStatus,
    setSessionRestored: composables.setSessionRestored,
    useActiveInstance: composables.useActiveInstance
  }

  seedHeaderPreview(composables)
}

/**
 * Plausible session-info defaults so the header chrome paints with
 * real chips (cwd / git / title / mode) in browser-only previews.
 * Each setter mirrors the per-source ACP / daemon split — the live
 * wire pushes the same way once the Rust event variants land.
 */
function seedHeaderPreview(composables: typeof import('@composables')): void {
  const previewId = 'preview-instance'
  composables.useActiveInstance().set(previewId)
  composables.pushSessionInfoUpdate(previewId, {
    title: 'reskin overlay header to wireframe spec'
  })
  composables.pushCurrentModeUpdate(previewId, { currentModeId: 'plan' })
  composables.pushInstanceModelState(previewId, { currentModelId: 'claude-opus-4' })
  composables.setInstanceCwd(previewId, '~/dev/hyprpilot')
  composables.setInstanceGitStatus(previewId, { branch: 'main', ahead: 2, behind: 0 })
}

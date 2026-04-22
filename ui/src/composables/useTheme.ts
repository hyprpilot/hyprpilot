import { invoke } from '@ipc'

/**
 * Palette tokens surfaced by the Rust config layer. Mirrors
 * `src-tauri/src/config/mod.rs::Theme`. Every leaf is typed as `string`
 * because `defaults.toml` is always loaded as the first layer — the
 * `defaults_populate_every_theme_token` test keeps that invariant true, so
 * by the time a Theme reaches the webview every field is guaranteed to
 * have a value. Groups may nest arbitrarily deep (e.g.
 * `surface.card.user.bg`).
 */
export interface Theme {
  font: { family: string }
  window: {
    default: string
    edge: string
  }
  surface: {
    card: {
      user: Card
      assistant: Card
    }
    compose: string
    text: string
  }
  fg: {
    default: string
    dim: string
    muted: string
  }
  border: {
    default: string
    soft: string
    focus: string
  }
  accent: {
    default: string
    user: string
    assistant: string
  }
  state: {
    idle: string
    stream: string
    pending: string
    awaiting: string
  }
}

/** A single card's painted tokens. `bg` today; future additions slot in. */
export interface Card {
  bg: string
}

/**
 * Builds the CSS custom property name from a token path. Segments named
 * `default` or `bg` are treated as the group's primary role and dropped
 * from the variable name — so `fg.default` → `--theme-fg`,
 * `surface.card.user.bg` → `--theme-surface-card-user`,
 * `surface.card.user.accent` (when added) → `--theme-surface-card-user-accent`.
 */
function cssVarName(parts: string[]): string {
  const kept = parts.filter((p) => p !== 'default' && p !== 'bg').map((p) => p.replaceAll('_', '-'))

  return `--theme-${kept.join('-')}`
}

/**
 * Fetches the resolved theme from the daemon and writes each token onto
 * `:root` as a CSS custom property. A missing `@tauri-apps/api/core` host
 * (plain `vite dev` in a browser, vitest jsdom) is a soft-fail — the UI
 * will render unstyled in those contexts, which is the intended signal
 * that the Rust-side theme source never ran.
 */
export async function applyTheme(): Promise<void> {
  let theme: Theme
  try {
    theme = await invoke<Theme>('get_theme')
  } catch {
    return
  }

  const root = document.documentElement
  walk([], theme, (path, value) => {
    root.style.setProperty(cssVarName(path), value)
  })
}

/**
 * Depth-first walk that emits every scalar string leaf with its full path.
 * Null / undefined nodes short-circuit at the entry guard — a defensive
 * catch for the "Rust sends a stray null" regression case.
 */
function walk(prefix: string[], node: unknown, emit: (path: string[], value: string) => void): void {
  if (node === null || node === undefined) return

  if (typeof node === 'string') {
    emit(prefix, node)

    return
  }

  if (typeof node === 'object') {
    for (const [key, value] of Object.entries(node as Record<string, unknown>)) {
      walk([...prefix, key], value, emit)
    }
  }
}

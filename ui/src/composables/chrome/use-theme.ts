import { invoke, TauriCommand, type Theme } from '@ipc'

/**
 * Builds the CSS custom property name from a token path. Segments named
 * `default` or `bg` are treated as the group's primary role and dropped
 * from the variable name — so `fg.default` → `--theme-fg`,
 * `permission.bg` → `--theme-permission`, `permission.bg_active` →
 * `--theme-permission-active`.
 */
function cssVarName(parts: string[]): string {
  const kept = parts.filter((p) => p !== 'default' && p !== 'bg').map((p) => p.replaceAll('_', '-'))

  return `--theme-${kept.join('-')}`
}

/**
 * Fetches the resolved theme from the daemon and writes each token onto
 * `:root` as a CSS custom property. A missing Tauri host (vitest jsdom)
 * is a soft-fail — the UI renders unstyled and the call site decides
 * what to do. Dev-mode browser preview seeds via the
 * `src/dev.ts` module loaded from `main.ts` instead, so
 * production keeps Rust as the sole token source per CLAUDE.md.
 */
export async function applyTheme(): Promise<void> {
  let theme: Theme

  try {
    theme = await invoke(TauriCommand.GetTheme)
  } catch {
    return
  }

  const root = document.documentElement

  walk([], theme, (path, value) => {
    const name = cssVarName(path)

    root.style.setProperty(name, value)
    // For colour leaves, also emit an `-rgb` companion (`r, g, b`)
    // so consumers can compose `rgba(var(--theme-X-rgb), <alpha>)`
    // without the older WebKit2GTK 4.1 webview dropping the
    // declaration. WebKit2GTK 4.1 predates CSS `color-mix()` (Safari
    // 16.2 / 2023) so any `color-mix(...)` rule silently no-ops; the
    // RGBA path has been valid since CSS3 and works everywhere.
    const rgb = parseHexRgb(value)

    if (rgb) {
      root.style.setProperty(`${name}-rgb`, rgb)
    }
  })
}

/// Parse a `#RRGGBB` or `#RRGGBBAA` hex string into an `r, g, b`
/// triplet ready for direct interpolation into `rgba(...)`. Returns
/// `undefined` for any non-hex input (font stacks, font sizes, …) so
/// `applyTheme` skips the `-rgb` emission for non-colour leaves.
function parseHexRgb(value: string): string | undefined {
  const m = /^#([0-9a-fA-F]{6})([0-9a-fA-F]{2})?$/.exec(value)

  if (!m) {
    return undefined
  }
  const hex = m[1]
  const r = Number.parseInt(hex.slice(0, 2), 16)
  const g = Number.parseInt(hex.slice(2, 4), 16)
  const b = Number.parseInt(hex.slice(4, 6), 16)

  return `${r}, ${g}, ${b}`
}

/**
 * Depth-first walk that emits every scalar string leaf with its full path.
 * Null / undefined nodes short-circuit at the entry guard — a defensive
 * catch for the "Rust sends a stray null" regression case.
 */
function walk(prefix: string[], node: unknown, emit: (path: string[], value: string) => void): void {
  if (node === null || node === undefined) {
    return
  }

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

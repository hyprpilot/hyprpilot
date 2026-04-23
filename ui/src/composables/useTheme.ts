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
  font: { mono: string; sans: string }
  window: {
    default: string
    edge: string
  }
  surface: {
    default: string
    bg: string
    alt: string
    card: {
      user: Card
      assistant: Card
    }
    compose: string
    text: string
  }
  fg: {
    default: string
    ink_2: string
    dim: string
    faint: string
  }
  border: {
    default: string
    soft: string
    focus: string
  }
  accent: {
    default: string
    user: string
    user_soft: string
    assistant: string
    assistant_soft: string
  }
  state: {
    idle: string
    stream: string
    pending: string
    awaiting: string
    working: string
  }
  kind: {
    read: string
    write: string
    bash: string
    search: string
    agent: string
    think: string
    terminal: string
    acp: string
  }
  status: {
    ok: string
    warn: string
    err: string
  }
  permission: {
    bg: string
    bg_active: string
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
 * User-desktop GTK font, as parsed from `gtk-font-name` on the default
 * `gtk::Settings`. Mirrors `src-tauri/src/daemon/mod.rs::GtkFont`.
 * `null` when the GTK query failed at boot — the CSS fallback takes
 * over in that case (browser default 16px `html { font-size }`).
 */
export interface GtkFont {
  family: string
  sizePt: number
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
 * Reads the GTK font size from the daemon and sets it as the base
 * `html { font-size }`. Every in-app `rem` unit inherits through this —
 * the user's desktop font-size preference propagates automatically.
 * Soft-fails when the command isn't available (plain browser / vitest)
 * or when the daemon couldn't query GTK — the CSS fallback (browser
 * default) takes over.
 */
export async function applyGtkFont(): Promise<void> {
  let font: GtkFont | null
  try {
    font = await invoke<GtkFont | null>('get_gtk_font')
  } catch (err) {
    console.warn('[hyprpilot] get_gtk_font invoke failed; using browser default font', err)

    return
  }
  if (!font) {
    console.warn('[hyprpilot] GTK font unavailable; using browser default font')

    return
  }
  // Page zoom (text + layout) is applied Rust-side via WebviewWindow::set_zoom
  // at boot, so nothing to do here for sizing. Override only the sans stack
  // so prose picks up the user's desktop font; mono stays on the theme stack
  // (code deserves a monospace regardless of what GTK is set to).
  document.documentElement.style.setProperty(
    '--theme-font-sans',
    `'${font.family}', ui-sans-serif, system-ui, sans-serif`
  )
  console.info(`[hyprpilot] GTK font ${font.family} ${font.sizePt}pt (zoom applied by daemon)`)
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

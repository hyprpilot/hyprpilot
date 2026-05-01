/**
 * Theme tokens surfaced by the Rust config layer. Mirrors
 * `src-tauri/src/config/theme.rs::Theme`. Every leaf is `string`
 * because `defaults.toml` always loads as the first layer — the
 * `defaults_populate_every_theme_token` test pins that invariant.
 *
 * The tokens drive `applyTheme()` which writes `:root` CSS custom
 * properties (`--theme-fg`, `--theme-surface-bg`, etc.) — see
 * `composables/chrome/use-theme.ts`. Per CLAUDE.md, Rust is the sole
 * source for these values; CSS files declare no literals.
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
    compose: string
    text: string
  }
  fg: {
    default: string
    ink_2: string
    dim: string
    faint: string
    /** Dark ink for tone-bg pills (warn / err / ok / accent fills). */
    on_tone: string
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
  terminal: {
    bg: string
    fg: string
    cursor: string
    selection: string
    black: string
    red: string
    green: string
    yellow: string
    blue: string
    magenta: string
    cyan: string
    white: string
    bright_black: string
    bright_red: string
    bright_green: string
    bright_yellow: string
    bright_blue: string
    bright_magenta: string
    bright_cyan: string
    bright_white: string
  }
  /// Shiki bundled-theme name driving fenced code-block syntax
  /// highlighting in markdown rendering. Default `one-dark-pro`. The
  /// markdown pipeline reads it at first highlight and passes it
  /// straight to Shiki's bundled-theme loader.
  shiki: string
}

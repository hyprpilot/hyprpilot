# CLAUDE.md

Agent operating manual for `utils/hyprpilot`. Read this first; the Linear project
description is the authoritative design snapshot and is referenced throughout.

## Project overview

- Single Rust binary (`hyprpilot`) that doubles as a Tauri 2 overlay daemon and a
  unix-socket CLI client, selected via subcommand (`daemon` / `ctl`).
- Frontend: Vue 3 + Vite + Tailwind v4 + shadcn-vue + reka-ui, under `ui/`.
- Backend: Rust crate at `src-tauri/` with `clap`-derive subcommand dispatch,
  `tauri-plugin-single-instance`, and a tokio `UnixListener` at
  `$XDG_RUNTIME_DIR/hyprpilot.sock`.
- Config: layered TOML — compiled defaults → `$XDG_CONFIG_HOME/hyprpilot/config.toml`
  → per-profile TOML → clap flags. The full UI theme is part of this config.
- Layout kept minimal for the scaffold — extensibility concerns (ACP bridge, MCP
  catalog, skills loader, overlay chrome) each land in their own issue.

## Toolchain (mise-pinned)

`mise install` at the repo root drops the required versions:

- `rust` (stable, components `rustfmt` + `clippy`)
- `node` 24, `pnpm` 10
- `task` 3 (go-task)
- `usage` 3 (for mise shell completions)
- `cargo-nextest` (via the `cargo:` backend — `task test` drives the Rust
  suite through nextest; plain `cargo test` still works locally for
  doc-tests or ad-hoc runs, but isn't the canonical path)

`rust-toolchain.toml` covers toolchain pinning for `cargo` invocations outside
mise.

## Tasks

Every `task` target orchestrates both Rust and the frontend where applicable.
Exactly the targets listed below exist — no others should be added without
updating this file.

| Task | Purpose |
| ---- | ------- |
| `task install` | `cargo fetch` + `pnpm install` at the workspace root (installs `ui`, `tests/e2e`, `tests/e2e/support/mock-agent` in one pass). |
| `task dev` | `./node_modules/.bin/tauri dev` — full dev cycle with Vite + Tauri. `@tauri-apps/cli` is a root-level devDep, so the binary lands in the workspace root's `node_modules/.bin`. |
| `task test` | `task test:ui` + `cargo nextest run --all-targets`. E2E stays out of the inner loop; run `task test:e2e` explicitly. |
| `task test:ui` | `pnpm --filter hyprpilot-ui test` — Vitest over every colocated `src/**/*.test.ts`. |
| `task test:e2e` | `TAURI_CONFIG=...` overlay-build → `pnpm --filter hyprpilot-ui build` → `pnpm --filter hyprpilot-e2e test`. The overlay (`src-tauri/tauri.conf.e2e.json`) inlines the Playwright-bridge capability at tauri-build time so production builds link zero plugin symbols. Browser mode today; `HYPRPILOT_E2E_MODE=tauri` for the bridge path. |
| `task format` | `cargo fmt --all` + `pnpm --filter hyprpilot-ui format` (Prettier + eslint --fix). |
| `task lint` | `cargo fmt -- --check` + `cargo clippy --all-targets -- -D warnings` + eslint + `vue-tsc --noEmit`. |
| `task build` | Debug build via `./node_modules/.bin/tauri build --debug`. |
| `task "build:release"` | Release build via `./node_modules/.bin/tauri build`. |

### Verifying UI changes — use named scripts, never `pnpm exec`

Always run UI lint / type-check / build through **named pnpm scripts**
or **task targets**, never via `pnpm exec` or `pnpm --filter <pkg> exec
<binary>`. The recursive-exec path silently exits `0` with
`Command "<binary>" not found` when the workspace root has no copy of
the binary in its `.bin/` (the binary lives in `ui/node_modules/.bin/`,
not the workspace root). A "passed" exit from a not-actually-run check
hides real errors — that's how the eight `noUnusedLocals` /
`ImageData` failures slipped through during the autostart MR.

Canonical commands per check:

| Check | Run from anywhere | Run from `ui/` |
| --- | --- | --- |
| Type-check | `pnpm --filter hyprpilot-ui run type-check` | `pnpm run type-check` |
| Lint (eslint + tsc) | `pnpm --filter hyprpilot-ui run lint` | `pnpm run lint` |
| Production build | `pnpm --filter hyprpilot-ui run build` | `pnpm run build` |
| Vitest | `pnpm --filter hyprpilot-ui test` | `pnpm test` |
| Full repo lint | `task lint` | `task lint` |
| Full repo build (Tauri) | `task build` | `task build` |

`task build` is the gold-standard pre-push verification: it runs the
UI's `pnpm build` (which runs `vue-tsc --noEmit && vite build`) and
then the Rust build. If `task build` exits 0, everything that
landing on `main` would catch is green. If it doesn't, fix it before
declaring the change verified — never claim "vue-tsc clean" on the
back of a `pnpm exec` invocation.

## Running the binary locally

```sh
# long-lived Tauri + socket
./target/release/hyprpilot                   # shorthand for `hyprpilot daemon`
./target/release/hyprpilot daemon

# CLI client
./target/release/hyprpilot ctl submit "hello there"
./target/release/hyprpilot ctl toggle
./target/release/hyprpilot ctl --help

# Status (one-shot snapshot; exits 0 even if daemon is down)
./target/release/hyprpilot ctl status
# → {"text":"","class":"idle","tooltip":"hyprpilot: idle","alt":"idle"}
# → {"text":"","class":"offline","tooltip":"hyprpilot: offline","alt":"offline"}  (daemon down)

# Status (long-running stream for waybar; reconnects with back-off on socket loss)
./target/release/hyprpilot ctl status --watch
# Emits one JSON line per state change; each line is waybar-compatible.
```

Second `hyprpilot daemon` forwards argv through `tauri-plugin-single-instance`
and exits `0` without opening a second window. When the second invocation
carries no subcommand (bare `hyprpilot` or `hyprpilot daemon`) the
single-instance callback also routes through `daemon::tray::present` —
captain's CLI escape hatch for popping the overlay when no Hyprland
keybind is bound yet. `hyprpilot ctl …` invocations stay out of this
path (the `ctl` arm runs locally, never reaches the running daemon's
single-instance callback).

The daemon boots **hidden by default** (`[daemon.window] visible = false`).
First user-visible map happens via a Hyprland keybind (`overlay/present`),
the system tray icon (left-click or "Show overlay" menu), or the bare
`hyprpilot` escape hatch above. Set `visible = true` to glue the overlay
on at boot. See `docs/autostart.md` for the autostart story
(`tauri-plugin-autostart` + the `[autostart] enabled` config knob).

### Waybar integration

Add a `custom/hyprpilot` module to your waybar config (see `docs/waybar.md`
for the full drop-in snippet):

```jsonc
"custom/hyprpilot": {
    "exec": "hyprpilot ctl status --watch",
    "return-type": "json",
    "on-click": "hyprpilot ctl toggle",
    "restart-interval": 5
}
```

`ctl status --watch` calls `status/subscribe` and streams one JSON object per
state change. `ctl status` (one-shot) is also safe for `exec` when
`restart-interval` handles polling.

## Config layering

Sources resolve in this order; later layers override earlier ones for the
fields they set.

1. Compiled defaults — `src-tauri/src/config/defaults.toml` embedded via
   `include_str!`.
2. Global config — `$XDG_CONFIG_HOME/hyprpilot/config.toml` or `--config <path>`.
3. Per-profile config — `$XDG_CONFIG_HOME/hyprpilot/profiles/<name>.toml`
   when `--config-profile <name>` / `HYPRPILOT_CONFIG_PROFILE` is
   supplied. This is the config-layering alias, not the session
   `[[profiles]]` registry — the two `profile` concepts live in
   parallel; the latter is addressed per-call via `ctl submit
   --profile <id>` / `session/submit { profile_id }`.
4. `clap` flags — override-per-invocation, never persisted.

`defaults.toml` is the **single source of truth** for default values. Rust
code consuming config leaves uses `.expect("... seeded by defaults.toml")`
rather than duplicating defaults as `unwrap_or(...)` fallbacks — the
`defaults_populate_every_daemon_window_field` test pins every
`.expect()`-ed leaf to a seeded TOML field. Removing a field from
`defaults.toml` without also removing the `.expect()` fails that test
before it ships a runtime panic. The only intentional `Option` leaf left
unset in defaults is `[daemon.window.anchor] height`, where `None` is the
"full-height fill" signal rather than a missing default.

`Config::validate()` runs after merge and fails startup with a readable error
on invalid values (e.g. unknown `logging.level`). `deny_unknown_fields` on
every section catches typos in user TOML at load time.

### Merge trait

Layer application goes through a `pub(crate) trait Merge { fn merge(self,
other: Self) -> Self; }` in `config/mod.rs`. `other` wins; `load()`'s fold
reads `acc.merge(layer)`. A blanket `impl<T> Merge for Option<T>` handles
every scalar leaf; each struct in the config tree carries a trivial
field-by-field impl; `AgentsConfig` is the one exception with a
keyed-list merge (override by `id`, append new ids, duplicates survive
for `validate_agents_ids` to flag).

### Validation strategy (garde)

Per-type invariants live on the type itself — not as free `validate_*`
functions — whenever the orphan rule allows:

- **Types we own**: `impl garde::Validate for T` + `#[garde(dive)]` at the
  field site. `Dimension` and `HexColor` follow this shape.
- **String-backed closed sets**: convert to a `#[derive(Deserialize)]`
  enum with `#[serde(rename_all = "lowercase")]`. `logging::LogLevel` is
  the example — unknown values reject at TOML parse time instead of at
  `validate()`, which is stricter.
- **Cross-field references**: higher-order `custom(fn(&self.sibling))`
  hooks. `agent.default` → `agents[].id` uses this pattern; see
  `validate_agent_default_id` in `config/validations.rs`. Documented in
  garde's README as "self access in rules".
- **Collection-level checks**: free fn + `#[garde(custom(fn))]` on the
  field. `validate_agents_ids` (uniqueness over `Vec<AgentConfig>`) stays
  here because the orphan rule blocks `impl Validate for Vec<T>` and a
  newtype would force consumers through `.0`.

Two free fns (`validate_agents_ids`, `validate_agent_default_id`) live in
`config/validations.rs` as `pub(super)` helpers. `Config::validate()` is
a one-liner that wraps the garde report in `anyhow!` — every rule is
inside the derive walk, no manual post-pass.

### `HexColor` newtype

Theme colour fields are `Option<HexColor>`, not `Option<String>`.
`#[serde(transparent)]` keeps the wire shape a bare string (the webview
sees no change through `get_theme`); `impl Validate` enforces
`#[0-9a-fA-F]{6,8}` under `#[garde(dive)]`. `impl Deref<Target = str>` +
`AsRef<str>` + `From<&str>` / `From<String>` keep consumer and test
ergonomics unchanged. `ThemeFont.family` stays `Option<String>` — it's
not a colour.

### `[skills] dirs` — multi-root catalogue

`[skills] dirs: Vec<PathBuf>` lists the roots the loader scans. Each
root is a flat directory of `<slug>/SKILL.md` bundles (one level deep
per root); compatible with the
[claude-code skill convention](https://github.com/anthropics/claude-code/blob/main/skills/README.md).

- **Defaults** seed `dirs = ["~/.config/hyprpilot/skills"]`. `~` and
  env vars expand at consume time via `SkillsConfig::resolved_dirs`.
- **User override replaces wholesale.** `dirs = ["/opt/skills/team",
  "~/personal"]` in user TOML wipes the default entry; `dirs = []`
  is the explicit "no skills" override. `None` (no `[skills]` block
  in user config) inherits the defaults — `Option<Vec<PathBuf>>`
  carries that distinction.
- **First-root-wins** on slug collision. The loader processes `dirs`
  in order and skips later roots' duplicate slugs with a `warn!`
  naming both paths (`kept = …`, `skipped = …`).
- **Missing roots warn + skip** — no auto-mkdir on boot, no
  `canonicalize` (paths are stored as the user wrote them). The
  watcher attaches one `.watch()` per existing root.
- **Watcher drops on remove.** `notify::Event::Remove(Folder)` for a
  watched root drops its `.watch()` subscription with a warn — no
  auto-rearm. Recovery: `ctl skills reload` after recreating the
  directory.

Skill delivery to the agent flows exclusively through the palette
(`Skill.path` lands on the wire so the UI can list / preview / pick);
the inline `#{skill/<slug>}` token mechanism was deleted end-to-end.
Picked skills attach to the user turn as `UserTurnInput::Prompt {
text, attachments }`; see the **ACP bridge** section.

### `mcps` — JSON-file MCP catalog

`mcps: Option<Vec<PathBuf>>` at the TOML root lists the JSON paths
the loader reads. Each path follows the standard `mcpServers` JSON
shape used by Claude Code / Codex / Cursor — **drop `~/.claude.json`
straight in and it works.** hyprpilot extends each server entry via
an optional `hyprpilot` namespace key carrying typed extension
fields:

```json
{
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
      "hyprpilot": {
        "autoAcceptTools": ["mcp__filesystem__read_*"],
        "autoRejectTools": ["mcp__filesystem__delete_*"]
      }
    }
  }
}
```

- **Merge semantics**: files iterate in order, `mcpServers` map
  collisions → later wins. Within a file, the `hyprpilot` block is
  pulled out + typed; everything else stays as opaque
  `serde_json::Value` (so future spec additions ride through to the
  agent without a daemon release).
- **Per-profile override**: `[[profiles]] mcps = [...]` wholesale-
  replaces the global default. `mcps = []` is the explicit "no MCPs"
  off-switch.
- **ACP injection**: each `session/new` and `session/load` call
  carries the resolved set as `NewSessionRequest.mcp_servers` /
  `LoadSessionRequest.mcp_servers`. Stdio (`command`) /
  HTTP (`url` + optional `type: "http"`) / SSE
  (`url` + `type: "sse"`) all project onto the typed ACP
  `McpServer` enum at projection time.
- **Permission integration**: `hyprpilot.autoAcceptTools` /
  `autoRejectTools` are matched at `PermissionController::decide`
  lane 2 via tool→server attribution by the `mcp__<server>__<tool>`
  prefix convention. See "Permissions are the vendor's concern" in
  the **ACP bridge** section.
- **No reload**: MCP catalog state is static after daemon boot.
  Restart-to-reconfigure model — captain edits the JSON file, then
  `hyprpilot daemon` again. (ACP fixes `mcpServers` at session/new
  anyway, so a reload would only land for new instances.)

## Theming

**The palette lives in Rust, not CSS.** Flow:

1. `src-tauri/src/config/defaults.toml` seeds every theme token under
   `[ui.theme.*]`.
2. User TOMLs override any subset; `merge_theme` walks the tree field-by-field
   using `.or()` over `Option<String>` leaves.
3. `config::Theme` is a typed tree. Groups:
   - `font` (`mono`/`sans`) — monospace stack for chrome, sans stack for
     inline assistant prose.
   - `window` (`default` + `edge`).
   - `surface` (`default`/`bg`/`alt`, `card.{user,assistant}`, `compose`,
     `text`) — `default` is the primary filled surface, `bg` is the body
     backdrop, `alt` the elevated variant.
   - `fg` (`default`/`ink_2`/`dim`/`faint`).
   - `border` (`default`/`soft`/`focus`).
   - `accent` (`default`/`user`/`user_soft`/`assistant`/`assistant_soft`) —
     `*_soft` pairs provide the washed-out tag/pill fill for each speaker.
   - `state` (`idle`/`stream`/`pending`/`awaiting`/`working`) — five-phase
     machine driving the overlay's live indicators.
   - `kind` (`read`/`write`/`bash`/`search`/`agent`/`think`/`terminal`/`acp`)
     — per-tool-family dispatch colors keyed by `ToolCall.kind`.
   - `status` (`ok`/`warn`/`err`) — toast / banner notification hues,
     distinct from phase state.
   - `permission` (`bg`/`bg_active`) — warm-brown panel fills for the
     permission stack.
4. The Tauri `get_theme` command serves the resolved tree to the webview.
5. `ui/src/composables/useTheme.ts::applyTheme` walks the object and writes
   every scalar leaf onto `:root` as a `--theme-<path>` CSS custom property.
   `main.ts` awaits it before `createApp(App).mount('#app')` so the first
   render already has the palette.

**CSS variable naming rule** (implemented in `cssVarName`):

- Path segments named `default` or `bg` drop from the emitted variable name
  (they represent the group's primary role).
- Remaining segments join with `-`; snake_case fields become kebab-case.
- Examples:
  - `fg.default` → `--theme-fg`
  - `surface.card.user.bg` → `--theme-surface-card-user`
  - `surface.card.user.accent` (future) → `--theme-surface-card-user-accent`

**Rules when extending the palette:**

- Add a new group by adding a `ThemeXxx` struct (`#[derive(Debug, Clone,
  Default, Deserialize, Serialize, PartialEq)]` + `#[serde(default,
  deny_unknown_fields)]`), wiring it into `Theme`, extending `merge_theme`,
  seeding values in `defaults.toml`, and updating the two token tests.
- Add a Tailwind utility alias in `ui/src/assets/styles.css::@theme inline`
  when a new token needs utility-class access (e.g. `bg-theme-<x>`).
- CSS must not declare literal theme values anywhere — not on `:root`, not
  as `var(--token, literal)` fallbacks, not inline in `.vue` scoped styles.
  Rust is the sole source; `applyTheme()` runs synchronously in `main.ts`
  before `createApp().mount('#app')` so no FOUC window exists. Exception:
  `tauri.conf.json::backgroundColor` paints before the webview mounts —
  keep it equal to `[ui.theme.window] default`.
- The Tauri window's native `backgroundColor` (in `src-tauri/tauri.conf.json`)
  is painted before the webview loads; keep it equal to
  `[ui.theme.window] default`.
- **Do not introduce new `--pilot-*` vars.** All theme tokens are `--theme-*`.
- Cards are keyed by speaker: `surface.card.user`, `surface.card.assistant`.
  Each is an object (`bg` today; `accent` / `border` / `fg` later). Do not
  name surfaces by elevation (`card_hi`, `card_alt`); name them by role.

### UI scaling — `[ui] zoom` config knob

The overlay's "make everything bigger" knob lives in **`[ui] zoom`**
(default `1.0`, range `[0.5, 2.0]`, override in user TOML). The
daemon reads it after the window maps and calls
**`WebviewWindow::set_zoom(zoom)`** — Chromium-style page zoom that
WebKit/Chromium expose as `set_zoom_level`. This scales text +
layout **uniformly**: paddings, widths, borders, gaps, fonts —
everything in the rendered tree multiplies by the zoom factor.

A CSS `:root { font-size }` knob would only scale `rem`-based
primitives. The codebase mixes `rem` typography with `px` paddings
/ widths / borders / gaps; without `set_zoom` the UI scales
text-only and looks broken. `set_zoom` is the canonical Tauri API
for this, mirroring how browser users hit `Ctrl++` to enlarge
everything proportionally.

Cross-platform — works the same on Hyprland, GNOME, KDE, macOS,
Windows. Webview DPI scaling (`scale_factor()` per monitor) layers
on top automatically, so `zoom = 1.0` on a 200%-scaled display
still renders crisply.

`get_gtk_font` is exposed so the webview can pick up the **family**
(not the size) on Linux — `useTheme::applyGtkFont()` overrides
`--theme-font-sans` with the user's GTK family so prose matches the
desktop. Optional desktop-integration nicety; sizing is fully
zoom-driven. `--theme-font-mono` stays on the configured stack
(code deserves a monospace regardless of desktop font).

**Why not the GTK desktop font size?** Earlier scaffolds wired
`gtk-font-name` → `set_zoom` with a 10pt baseline (`zoom = 1.0 +
(size_pt - 10) * 0.1`). Two problems: (a) Linux-only — macOS /
Windows users got no scaling at all; (b) the 10pt baseline assumed
every desktop's font setting maps to the same logical size, which
is false (`Segoe UI 11` on Windows ≠ `Inter 11` on Hyprland). A
config knob is the common-method replacement (mirrors how VS Code,
Zed, Discord, Obsidian all do it).

**Why not a CSS `:root { font-size }` knob?** Bumping the root
font-size only scales primitives written in `rem` units. The
codebase has lots of literal `px` for layout (paddings, widths,
borders) — those wouldn't budge, and the UI would look stretched
text in a fixed shell. `set_zoom` scales every box.

**Why not `WebKitSettings::default-font-size`?** That property
only scales the default font (unset CSS sizes); it doesn't touch
explicit `font-size: 1rem` declarations. Wrong axis.

**`text-[0.Nrem]` is still the canonical way to set a font size**
inside a primitive — full utility aliases (`text-xs`, `text-sm`, …)
are rem-based and fine. Avoid literal `font-size: Npx` so the rem
baseline stays the single source of typographic scale.

## Window surface (`[daemon.window]`)

The daemon's main window runs in one of two modes, selected by
`[daemon.window] mode`:

- `anchor` (default) — a `zwlr_layer_shell_v1` surface pinned to a configurable
  edge, painted above normal windows. Matches the Python pilot's behavior on
  Hyprland / Sway / wlroots-based compositors. Requires the compositor to
  implement `zwlr_layer_shell_v1` — **does not work on GNOME Shell or KDE
  Plasma**, which don't expose that protocol.
- `center` — a regular Tauri top-level sized as a percentage of the active
  monitor and centered by the compositor. Works on any compositor (Wayland or
  X11); the escape hatch for non-wlroots desktops and the natural home for
  future "launcher"-style UX.

Two knobs are intentionally **not exposed in config**:

- `layer = overlay` — always paints above normal and fullscreen windows. Other
  layers (`background` / `bottom` / `top`) are footguns for a chat overlay;
  there's no reasonable value other than `overlay`.
- `keyboard_interactivity = on_demand` — compose input needs to accept focus,
  and the overlay must not grab keys while idle (which would break every
  editor hotkey). `exclusive` only grabs the keyboard (mouse passes through),
  but `on_demand` is the simpler default and easier to explain.

Both are hardcoded in `src-tauri/src/daemon/mod.rs`. Do not add config knobs
for either without a new issue.

### Config shape

```toml
[daemon.window]
mode = "anchor"        # "anchor" | "center"
output = "DP-1"        # optional; defaults to primary monitor
visible = false        # boot with the surface unmapped (default).
                       #  set true to glue the overlay on at boot.

[daemon.window.anchor]
edge = "right"         # "top" | "right" | "bottom" | "left"
margin = 0             # px from the anchored edge
width = "40%"          # "N%" (of monitor) or pixel int; default 40%
# height unset         # unset → full-height fill via top+bottom anchor

[daemon.window.center]
width = "50%"          # "N%" (of monitor) or pixel int
height = "60%"
```

`width` / `height` under both `[daemon.window.anchor]` and
`[daemon.window.center]` accept either a pixel integer or an `"N%"` string;
the enum is `Dimension::{Pixels(u32), Percent(u8)}`. A custom `Deserialize`
impl handles the `%` suffix; anything else (`"50px"`, bare floats) is
rejected at load time. Percentages resolve against the active monitor's
physical size **on every show transition**, not just at boot — so moving the
overlay between monitors and toggling back on produces the correct size for
the new output. The full `[daemon.window]` config is owned by the
`WindowRenderer` struct (`daemon/renderer.rs`), registered in Tauri managed
state; its `show()` method is the single code path for both setup and toggle.

`[daemon.window.anchor] height` is intentionally unset by default. With
height unset the daemon pins top + bottom + `edge`, so the compositor
stretches the surface full-height — the Python-pilot overlay shape.
Setting an explicit `height` pins only `edge` and uses that fixed extent.

### Edge accent

The daemon exposes `get_window_state` → `{ mode, anchorEdge }`. At boot,
`ui/src/composables/useWindow.ts::applyWindowState` writes
`data-window-anchor="<edge>"` on `<html>` in anchor mode (and leaves it
unset in center mode). `ui/src/assets/styles.css` then paints
`var(--theme-window-edge)` differently per mode:

- **Anchor mode**: a single 2px stripe on the side *opposite* the
  anchored edge (the inward side where the overlay meets the desktop).
  One `html[data-window-anchor='<edge>'] body` selector per edge
  variant. The anchored edge itself stays borderless — it sits flush
  against the screen bezel, so a stripe there would be clipped.
- **Center mode**: full 2px perimeter via
  `html:not([data-window-anchor]) body`. Every edge is inward in center
  mode, so framing the whole instance reads cleanly.

A single `body { box-sizing: border-box; }` rule keeps whichever border
gets painted inside the `100vh` viewport instead of pushing content
past the anchored edge.

Extending to a new anchor edge is additive: Rust enum variant +
serialize name + one CSS selector pairing the attribute value with
the opposite-side border.

### Monitor selection — `WindowManager` adapter

Picking which monitor the overlay lands on is compositor-specific and
lives behind a trait in `src-tauri/src/daemon/wm.rs`:

```rust
pub struct MonitorInfo {
    pub name: String,           // connector ("DP-1") — identity key
    pub make: Option<String>,
    pub model: Option<String>,
    pub serial: Option<String>,
}

pub trait WindowManager: Send + Sync {
    fn name(&self) -> &'static str;
    fn focused_monitor(&self, monitors: &[Monitor]) -> Option<MonitorInfo>;
}
```

**Identity rule:** `MonitorInfo.name` is the connector name and the
only identifier that gets matched against `Monitor::name()` or
`[daemon.window] output`. `make` / `model` / `serial` come from EDID
and are metadata — useful for log lines and future stricter matching
(two identical monitors on swapped ports), never load-bearing today.
All three metadata fields are `Option` because not every source
populates them.

**Three concrete adapters**, detected at boot via env markers (see
`wm::detect()`):

| Adapter | Selected when | Source |
| -- | -- | -- |
| `WindowManagerHyprland` | `HYPRLAND_INSTANCE_SIGNATURE` is set | `hyprctl -j monitors` → the entry with `focused: true` |
| `WindowManagerSway` | `SWAYSOCK` is set (and Hyprland isn't) | `swaymsg -t get_outputs -r` → same shape |
| `WindowManagerGtk` | everything else (X11 + non-wlroots Wayland) | `gdk::Seat::pointer().position()` bounds-check |

Both compositor IPC formats emit
`[{ focused, name, make, model, serial, ... }]` with matching key
names, so `focused_monitor_info(json, "focused")` is a shared helper.
The GTK fallback only populates `name` — GDK 0.18 doesn't expose
connector strings on `gdk::Monitor`, and pulling `make` / `model`
requires the geometry hop documented below.

**Why compositor IPC over cursor query:** Wayland has no standard
client-side cursor API (privacy), and `window.cursor_position()` /
`gdk::Device::position()` frequently return stale `(0, 0)` on
multi-monitor wlroots sessions. Hyprland / Sway both expose "which
output is focused" over their IPC socket — that's the authoritative
signal for overlay placement.

**Resolution order in `WindowRenderer::resolve_monitor`:**
1. Explicit `[daemon.window] output` from config — always wins.
2. `self.wm.focused_monitor(&monitors)` → match `info.name` against
   Tauri's monitor list.
3. `window.primary_monitor()` — compositor-defined fallback.
4. Any monitor — safety net so `apply_*` never hits `unwrap`.

Extending to a new compositor is one struct + one `detect()` branch;
the trait stays stable.

**`gdk::Monitor` pinning:** the layer-shell surface is always pinned
to the resolved monitor via `gtk_window.set_monitor(&gdk_monitor)`
(not conditionally on `output` being set). Without the pin the
compositor picks an output, which can land the surface on a monitor
different from the one we sized against — reads as "40% of the wrong
monitor". `gdk_monitor_for(&Monitor)` in `renderer.rs` matches by
geometry because gdk 0.18 has no `connector()` accessor (GTK4-only);
collapses to a direct connector compare when the GTK4 migration
lands.

### Crate: `gtk-layer-shell` 0.8 (GTK3)

Tauri 2.10 on this repo still links `webkit2gtk` 4.1 (the GTK3 binding), so
we use the `gtk-layer-shell` crate (with the `v0_6` feature for
`set_keyboard_mode`). If Tauri ever switches to `webkit2gtk-6.0` / GTK4, swap
to `gtk4-layer-shell`. System package (Arch): `gtk-layer-shell`.

Layer-shell init runs inside the Tauri `.setup(...)` closure — Tauri's
`WebviewWindow::gtk_window()` returns a `gtk::ApplicationWindow`, and
`init_layer_shell` must be called before the window is realized. To
satisfy that invariant the main window is declared `visible = false` in
`tauri.conf.json`; `apply_anchor_mode` then configures the layer surface
and maps the GTK window directly via `gtk_window.show_all()`. Do not
switch to `WebviewWindow::show()` for the anchor path — on some wlroots
builds it re-maps through xdg-shell and silently drops the layer-shell
role.

## Logging

`tracing` is bootstrapped once via `logging::init`. Both the dev stderr layer
and the release file layer tag every event with its `file:line` callsite +
module target. Helpers:

- `dev_fmt_layer` — ANSI on, stderr writer.
- `file_fmt_layer` — ANSI stripped, rolling file under
  `$XDG_STATE_HOME/hyprpilot/logs/hyprpilot.log.*` via `tracing-appender`.

Filter precedence: `--log-level` → `RUST_LOG` → `info` fallback.

## Frontend testing

Two tiers, two locations — one convention per tier, no forks. Add tests
alongside the code they cover; never under `__tests__/` or `.spec.ts`
beside `.vue` files (e2e specs are the sole `.spec.ts` carriers and
live under `tests/e2e/specs/`).

| Tier | Runner | Location | File suffix |
| -- | -- | -- | -- |
| Component / composable / lib | Vitest + `@vue/test-utils` + jsdom | beside the source | `<PascalOrCamel>.test.ts` |
| End-to-end | Playwright via `@srsholmes/tauri-playwright` | `tests/e2e/specs/` | kebab-case `.spec.ts` |

```
ui/src/
├── components/PermissionPrompt.vue
├── components/PermissionPrompt.test.ts     # colocated
├── composables/useAcpAgent.ts
├── composables/useAcpAgent.test.ts
├── ipc/                                    # @ipc — invoke/listen wrappers
└── views/Placeholder.vue
└── views/Placeholder.test.ts

tests/e2e/
├── playwright.config.ts                    # browser mode default
├── fixtures/{tauri.ts, e2e-config.toml, global-{setup,teardown}.ts}
├── specs/{smoke, submit, permission}.spec.ts
└── support/mock-agent/                     # scripted ACP Node process
```

Component tests mock Tauri IPC by replacing the `@ipc` barrel with
`vi.mock('@ipc', ...)` — never monkey-patch `window.__TAURI__`. E2E
specs today run in `browser` mode against a Vite dev server with IPC
mocks (`fixtures/tauri.ts::ipcMocks`); the daemon-spawning
`tauri` mode is fully wired (Cargo `e2e-testing` feature +
`tauri.conf.e2e.json` overlay merged via `TAURI_CONFIG` env var +
mock-agent subprocess) and gates behind `HYPRPILOT_E2E_MODE=tauri` — see
`tests/e2e/README.md` for the WebKitGTK-4.1 eval-stall that keeps it
off the default lane.

### Playwright MCP for interactive UI debugging

The Playwright MCP server (`mcp__mcphub__playwright__*` tools) drives
Chromium, not the Tauri WebKit webview. It does **not** inspect a
running Tauri window; it drives a standalone headless browser.
Useful for one-off layout / flex / height debugging where the agent
needs to check computed styles against a real layout engine:

1. Start the Vite dev server alone: `pnpm --filter hyprpilot-ui dev`
   (prints the port — usually `http://localhost:1420/`).
2. `mcp__mcphub__playwright__browser_navigate { url: "http://localhost:1420/" }`.
3. `mcp__mcphub__playwright__browser_evaluate { function: "() => { ... }" }`
   for computed-style inspection, e.g. `getComputedStyle(el).height`.
4. **Screenshot output goes to `.playwright-mcp/`** at the repo root.
   Always pass `filename: ".playwright-mcp/<descriptive-name>.png"` to
   `browser_take_screenshot` so artifacts stay scoped to that folder
   (the runner respects relative paths). The folder is gitignored;
   keep screenshots there for review and don't litter the source tree
   with them.
5. Kill both the Vite server and the browser process when done
   (`pkill -f 'vite.*hyprpilot-ui'`, `pkill -f brave` / `chromium`).

**Caveats:**

- The browser mode in MCP may hang during launch in sandboxed shells
  (Brave-on-Wayland WebSocket handshake can stall). If the first
  `browser_navigate` call times out, the agent has no Playwright
  access in that environment — fall back to reading the code +
  static-checking computed-style rules manually.
- IPC-dependent UI paths (anything calling `invoke()` / `listen()`)
  surface the "tauri host missing" soft-fail in browser mode — the
  UI renders without Rust-side state. That's expected; for IPC-live
  inspection use the Playwright-tauri bridge (`HYPRPILOT_E2E_MODE=tauri`)
  once the WebKitGTK eval-stall clears. For pure layout / CSS / DOM
  debugging, browser mode is sufficient and the fastest path.
- **Do not** use Playwright MCP for scripted regression tests — those
  belong in `tests/e2e/` under the `@srsholmes/tauri-playwright`
  harness, which runs against the real Tauri build. Playwright MCP
  is for ad-hoc inspection only.

#### Visual-debug investigation flow

When the user flags a "the UI doesn't look like the wireframes" gap,
the loop is:

1. **Make sure the dev server is running** in the background
   (`pnpm --filter hyprpilot-ui dev` via `run_in_background: true`).
   Wait ~3-4s for the Vite "ready" line, then proceed.
2. **Navigate** with `mcp__mcphub__playwright__browser_navigate
   { url: "http://localhost:1420/" }`.
3. **Screenshot to `.playwright-mcp/`** — pass
   `filename: ".playwright-mcp/<descriptive-name>.png"` to
   `browser_take_screenshot`. Never let it default into the project
   root.
4. **Compare** the screenshot against the wireframe HTML / JSX in
   the design bundle (`/tmp/wireframes/hyprpilot/project/wf-d5-fusion.jsx`).
   Read the JSX block for the screen state you're checking; the
   designer's spec is in inline comments ("VISUAL LAW", "FIDELITY
   NOTE").
5. **Iterate**: edit the relevant `.vue` file, save (Vite HMR picks
   up automatically — no rebuild), re-screenshot, diff. Repeat
   until visually faithful.
6. **`browser_evaluate`** is the escape hatch for layout numbers —
   when a screenshot diff doesn't tell you *why* something's the
   wrong size, run a one-shot computed-style query
   (`getComputedStyle(el).height`) to get the actual numbers.
7. **Cleanup** when done: kill the dev-server background process
   (the runner may surface its task ID in a notification when the
   server crashes; otherwise `pkill -f 'vite.*hyprpilot-ui'`).

The dev preview pulls a non-Tauri theme + window-state shim from
`ui/src/assets/dev-preview.ts`, applied via dynamic import in
`main.ts` only when `import.meta.env.DEV && !window.__TAURI_INTERNALS__`.
Production Tauri builds tree-shake the entire module out — the real
app keeps "Rust is the sole source" intact, with no fallback path
for theme tokens or window state.

#### Tauri ↔ Playwright wiring

For wire-flow verification today, use the **Hybrid daemon-driven**
pattern below — the WebKitGTK eval-stall blocks native-webview
screenshots but every other Tauri-mode capability (spawning the
daemon, driving it via `ctl`, listening on the unix socket,
asserting log emissions) is functional. The four upstream options
listed here matter only when full native-webview DOM assertions
become required (visual regression on the layer-shell surface
itself, compositor-rendered screenshots, etc.) and the eval-stall
clears or we migrate to GTK4 + webkit2gtk-6.0 per the upstream
runway. Until then they're catalogued for completeness:

- **`@srsholmes/tauri-playwright` 0.2** (already in `tests/e2e/`).
  Three modes: `browser` (mocked IPC, works today), `tauri` (socket
  bridge to the real native webview, gated behind `HYPRPILOT_E2E_MODE=tauri`
  + the `e2e-testing` Cargo feature; stalls today on webkit2gtk-4.1),
  `cdp` (Windows-only WebView2 direct CDP). Wireframe screenshots
  via `tauriPage.screenshot({ path: '.playwright-mcp/<name>.png' })`
  hit the real native window (CoreGraphics on macOS, X11/Wayland on
  Linux).
- **`tauri-plugin-webdriver` 0.2** — full W3C WebDriver server on
  port 4445. Drop-in for Selenium / WebdriverIO / Playwright as a
  W3C client. Requires `withGlobalTauri: true` in `tauri.conf.json`
  + plugin registered in `src-tauri/src/main.rs::Builder`. Cleanest
  path if `tauri-plugin-playwright` keeps stalling.
- **`@wdio/tauri-service`** — WebdriverIO service with `embedded`
  driver provider (native, all platforms, no external driver). Adds
  window management, clipboard, file ops, screenshot capture as
  first-class commands.
- **`tauri-driver` (official)** — the canonical W3C surface, but
  needs an external native driver: `msedgedriver.exe` on Windows,
  `webkit2gtk-driver` on Linux. Heaviest setup; only worth it for
  CI parity with prod.

For **scripted regression tests** in `tests/e2e/`, the hybrid
daemon-driven pattern below is the canonical surface today. For
**ad-hoc UI inspection via Playwright MCP** (this agent driving
Chromium against the dev preview), the path forward is unchanged —
browser mode + dev-preview shim — until the upstream stall clears.
Then flip the MCP server to `mode: 'tauri'` against a running
`cargo tauri dev --features e2e-testing` for real-app screenshots.

#### Hybrid daemon-driven verification (the canonical methodology)

When debugging a wire-flow bug — anything where the question is "is
the daemon emitting / handling X correctly?" — **the WebKitGTK
eval-stall is not a blocker.** The native-webview screenshot path
stalls; everything else works.

**Always run this verification through the Playwright e2e harness,
not by spawning the daemon directly from the shell.** The harness
owns the daemon's lifecycle via `tests/e2e/fixtures/global-setup.ts`
(spawn + socket-wait) and `global-teardown.ts` (SIGTERM on exit), so
nothing leaks past the test run and the user's `task run` flow
stays untouched. A bare `setsid ./target/debug/hyprpilot daemon` is
for one-off ad-hoc inspection only — never for repeatable
verification, never inside an agent loop, never as the "default"
diagnostic.

The pattern that consistently verifies wire-side behavior under
real ACP traffic:

1. **Run the spec via `task test:e2e:live`.** The Taskfile target
   is wired; the underlying invocation is

   ```sh
   HYPRPILOT_E2E_MODE=tauri \
   HYPRPILOT_E2E_CONFIG=tests/e2e/fixtures/live-config.toml \
   pnpm --filter hyprpilot-e2e test
   ```

   `live-config.toml` pins the agent to `claude-code` + `haiku`
   model so the model / cwd / mode chips in the header have
   deterministic values to assert against. `global-setup.ts`
   spawns the binary with `HYPRPILOT_CONFIG` pointing at the
   fixture, waits for the socket, and tears it down on teardown
   — no per-spec daemon plumbing needed.

2. **Drive the wire from inside the spec via `ctl`.** Each spec
   spawns `./target/debug/hyprpilot ctl …` as a subprocess, with
   the right `XDG_RUNTIME_DIR` / `HYPRPILOT_SOCKET` env carrying
   over from `globalThis.__HYPRPILOT_E2E__`. Every `acp:*` event
   the daemon emits is reachable through this path. The two most
   useful subcommands:

   ```sh
   ctl instances spawn --agent claude-code --cwd /path/to/repo
   ctl prompts send --instance <uuid> "<prompt>"
   ```

   `tests/e2e/specs/wire-instance-meta.spec.ts` is the reference —
   it spawns an instance, sends a prompt, and asserts on the
   daemon log content. Use it as a template when adding a new
   wire-flow regression test.

3. **Trace the emit path via `HYPRPILOT_LOG_LEVEL=trace`.**
   `global-setup.ts` already routes the daemon's stdout / stderr
   into `${runtimeDir}/daemon.log`; passing
   `HYPRPILOT_LOG_LEVEL=trace` (or `RUST_LOG='hyprpilot::adapters=trace'`
   on the spawned env) makes every `app.emit` call show up as a
   `trace!` line via `emit_acp_event`. Specs read the file with
   `fs.readFileSync(path, 'utf8')` and assert via Playwright's
   `expect(log).toContain('acp:instance-meta')` patterns.

4. **Pair with Playwright MCP (browser mode) for the visual layer.**
   The UI rendering verifies separately against a Vite dev server
   with the dev-preview shim — `__hyprpilot_dev` exposes
   `pushSessionInfoUpdate`, `pushCurrentModeUpdate`,
   `setInstanceCwd`, etc. — so a `browser_evaluate` call seeds the
   exact state the live wire would produce, then
   `browser_take_screenshot { filename: '.playwright-mcp/<name>.png' }`
   captures it. **The daemon log proves the wire shape; the
   Playwright-MCP screenshot proves the chrome renders that shape
   correctly.** Together they cover what `tauriPage.screenshot`
   would have given us if the eval-stall ever clears.

**Force this loop whenever a chip / header / row "doesn't update".**
The dev-preview shim alone can fake any state, but it lies — the
Rust mapper might be dropping the wire variant into `Unknown`, or a
new ACP enum variant might not have a Tauri event bridge. Running
the e2e harness + reading its daemon log is the only way to know.

`InstanceMeta` is the canonical example: the chip would render
correctly under dev-preview seeding, but the live wire never carried
mode / cwd / model because the Rust transcript-mapper dropped them
into `Unknown` and there were no `acp:*` events to bridge them. That
gap was invisible from browser-mode tests alone — only the
e2e-driven daemon log surfaced it.



- **No backwards-compatibility layers — ever.** This repo has no stability
  contract with the outside world: the CLI, the unix-socket wire protocol,
  the config file, and the theme tree all evolve in lockstep with the daemon
  binary. When a design stops making sense, **delete it and rewire the call
  sites**; do not leave typed-shim enums, deprecated method aliases, or
  "legacy" wrappers behind a trait. The `Call` enum in `rpc/protocol.rs`
  was removed for exactly this reason once `RpcDispatcher` + `RpcHandler`
  landed — each handler now parses its own `params: Value` and
  `dispatch_line` routes on the raw method string. Apply the same rule to
  every future refactor: one shape, one code path, no aliases.
- **Stubs panic, they don't pretend.** When a feature isn't wired end-to-end
  yet (typically because its real implementation is gated behind a later
  Linear issue), the client-side entry point must `unimplemented!("<verb>:
  <why> (K-xxx)")` rather than round-trip to the server and pretty-print a
  placeholder response. Printing a fake-success JSON from a server-side stub
  looks exactly like success and hides the gap. Example: today
  `src-tauri/src/ctl/handlers.rs::SubmitHandler` / `CancelHandler` /
  `SessionInfoHandler` all `unimplemented!("… ACP bridge not yet
  implemented (K-239)")` — the server still carries echo-style stub
  responses for those methods, but the CLI never reaches them. Same rule
  applies the other direction: if a server-side `RpcHandler` returns a
  hand-rolled placeholder, nothing on the CLI side should dress it up as a
  real result. When K-239 lands, flip the `unimplemented!()` in one edit;
  never in two.
- **Never fabricate static UI text.** Every visible string —
  status chips, match badges, "first time" / "no rule" pills,
  counter labels — must read from a real signal. If the data isn't
  wired (no trust store yet, no telemetry yet, no rule registry),
  do NOT type a placeholder string and ship it: the captain reads
  it as truth, and "no rule in trust store" looks identical
  whether the trust store is empty or whether the trust store
  doesn't exist. Either wire the real signal, or omit the element
  entirely until it can be backed by data. The same rule applies
  to error toasts, tooltips, and aria-labels — fabricated copy is
  a runtime lie. Precedent: a `first time · no rule` chip in the
  permission stack header was deleted in the K-XXX D5 reskin
  because it was prose-only — not a component prop bound to any
  trust-store query. If you find yourself writing copy that
  *describes* state instead of *reading* state, stop and either
  surface the real signal through the wire or drop the element.
- **Inline single-use helpers.** A function with exactly one caller should be
  folded into that caller. Prefer `fn main() -> Result<()>` over a `try_main`
  wrapper; prefer unfolding a small setup step into the body (with a short
  comment) over extracting a one-call helper.
- **Compose behavior onto the owning type, not as free fns.** When a
  module defines a primary type (`AcpClient`, `AcpAdapter`,
  `StatusBroadcast`), helpers that operate on that type's state — or
  need to touch the channels / handles / registries it owns — go as
  methods on it, not module-level fns. Free fns are for pure
  transformations that don't read or mutate the type's invariants.
  Drove the K-240 refactor from free `forward_notification` /
  `auto_cancel_permission` + `emit_acp_event` into methods on
  `AcpClient` / `AcpAdapter`; keeps the ownership graph legible and
  avoids passing owned state by parameter.
- **Small composable primitives live in `src-tauri/src/tools/`; domain
  modules host thin adapters over them.** A type that could exist
  without the domain it first appears in (a sandbox, a terminal
  registry, an fs-with-containment wrapper) belongs in `tools/`,
  returns a domain-specific error enum (`SandboxError`, `FsError`,
  `TerminalError`), and knows nothing about the protocol that called
  it. The domain module (`adapters/acp/client.rs`) becomes a translation layer
  — parse the wire envelope, delegate to the tool, map the tool's
  error into the protocol's error. Precedent: K-244 MR 21 review
  refactored `AcpClient` from owning `sandbox_root` / `terminals`
  directly into `{ events, fs: Arc<FsTools>, terminals: Arc<Terminals> }`;
  `FsTools` / `Terminals` / `Sandbox` moved under `tools/`, only the
  ACP error-mapping stayed inside `adapters/acp/`. Every non-ACP entry point
  (a Tauri command, a future gRPC shim, unit tests) then reuses the
  primitive without inheriting the ACP envelope types.
- **Structs carry their invariants; don't re-pass context on every call.**
  When a helper needs the same configuration value ("the sandbox
  root", "the registry handle") on every invocation, wrap it in a
  struct and make the helper a method. `Sandbox { root: PathBuf }::new(root)`
  canonicalises once at construction, then `sandbox.resolve(path)`
  uses the already-validated root — not `canonicalize_within(base, path)`
  which re-runs the check every call. Runtime errors that were only
  possible because the first arg was untrusted (`MissingBase`,
  `RootNotADirectory`) collapse into construction-time errors,
  tightening the type. Same shape for `FsTools { sandbox }`,
  `Terminals { registry }`, `WindowRenderer { wm, config }`.
- **Prefer enum + match dispatch for similar handlers; reach for
  macros only when monomorphisation forces per-handler registration.**
  The first choice for a family of related operations is a closed
  enum variant + a match in the dispatcher — `SessionCommand::{Prompt,
  Cancel, Shutdown}` in `acp/runtime.rs`, `ClientEvent::{Notification,
  PermissionRequested}` in `acp/client.rs`, `RpcHandler` impls routed
  from `RpcDispatcher::dispatch_line`. One enum = one exhaustive
  match, compiler enforces coverage, adding a variant surfaces every
  call site. Use a `macro_rules!` only when the external API forces
  type-parameterised monomorphisation per handler (ACP's
  `Client.builder().on_receive_request::<Req, Resp, _>(...)` emits a
  distinct type per (Req, Resp) pair, so one `on_receive_request`
  call per method is mandatory at compile time). Then a declarative
  macro — `register_client_handler!(builder, client, $method)` in
  `acp/runtime.rs` — collapses the identical closure bodies to one
  line per method. Do NOT invent a `CapabilityHandler` wrapper trait
  with `Box<dyn ...>` just to "feel polymorphic" — it adds ceremony
  without real dispatch.
- **Traits for open extension points; closed enums for closed sets.**
  Traits pay their way when new implementers arrive from outside the
  decision you're making today — `WindowManager` for compositors
  (Hyprland / Sway / Gtk fallback), `AcpAgent` for vendors
  (claude-code / codex / opencode). The trait stays stable; adding
  one means a new struct and a new `detect()` branch. Closed enums
  for the known-at-compile-time alternatives — `AgentProvider`,
  `Dimension::{Pixels, Percent}`, `logging::LogLevel`. Mix is fine:
  `AgentProvider` is the closed enum, `AcpAgent` is the trait whose
  impls match 1:1 onto its variants; `match_provider_agent(provider)`
  is the bridge.
- **Hub-and-spokes dispatch — trait + impl-per-sub-enum, single
  delegating impl on the parent.** When you have a forest of closed
  enums each with its own `match self → call method fn` body
  (clap subcommand trees, multi-namespace command routers, any
  parent-enum-of-sub-enums shape), lift the contract to a small
  shared trait — one method, takes the per-call context, returns
  `Result<()>` — and impl it once per sub-enum in the file that
  owns the sub-enum. The parent enum gets one impl whose body
  either handles top-level shortcut variants inline or delegates to
  the inner via the same trait method (`command.dispatch(ctx)`). The
  call site collapses to `args.command.dispatch(&ctx)`.

  Canonical example: `src-tauri/src/ctl` — `pub(super) trait
  CtlDispatch { fn dispatch(self, client: &CtlClient) -> Result<()>; }`
  in `mod.rs`, one impl per namespace file (`agents.rs`, `sessions.rs`,
  …), the parent `CtlCommand` impl in `mod.rs` is the hub. Adding a
  new namespace = new file + new sub-enum + `impl CtlDispatch for
  NewSub` + one variant on the parent + one match arm that delegates.

  Earns its keep over free `dispatch` fns because: many non-trivial
  implementers (each impl carries a real match), uniform call site
  (no per-namespace fn name to remember), trait IS the routing
  protocol at the type system level. Different from the
  `CtlHandler`-style trait we deleted (one trait impl per leaf
  command, mostly trivial bodies) — there the trait was ceremony;
  here it's the shape's actual contract.

  When NOT to force it: don't introduce a dispatch trait for a
  single-level command tree (just match the one enum), don't add
  it speculatively before the second sub-enum exists, don't apply
  it to families where each "implementer" is a one-line shell
  (that's `CtlHandler`'s failure mode). The trigger is **multiple
  non-trivial sub-enum match bodies** — that's when the trait pays
  for itself.
- **Comment discipline — terse WHY, never WHAT.** Default to no comments.
  Code + well-named identifiers already describe behavior; comments earn
  their keep only when they encode a non-obvious reason (a protocol quirk, a
  data-source disagreement, a deliberate future-proofing choice). Docstrings
  stay one or two short sentences in the common case; the "grow it into a
  mini-essay so future me remembers why" temptation is wrong — that context
  goes in commit messages and CLAUDE.md. Trim aggressively on every diff.
  Examples of fair comments: "gdk 0.18 has no `connector()`, match by
  geometry instead"; "second SIGINT falls through to default handler";
  "Unknown levels reject at TOML parse (serde closed enum)". Examples of
  comments to delete: restating the function name, listing every caller,
  explaining what a `match` does.
- **Multiline fixtures use raw strings.** Any string literal containing
  more than one `\n` — TOML test fixtures, JSON-RPC request bodies, CSS
  snippets — uses a Rust raw string (`r#"..."#` / `r##"..."##` when the
  content includes `"#`, e.g. `"#ff00aa"` hex colours). Escaped
  `"[section]\nkey = \"val\"\n"` is unreadable at a glance and
  diff-noisy; raw strings render the actual content. Single-line
  literals with one trailing `\n` can stay as-is.
- **NVIDIA + Wayland workaround.** `main.rs` sets
  `WEBKIT_DISABLE_DMABUF_RENDERER=1` before any thread spawns on Wayland
  sessions. Overridable by exporting the env var. Keep that block first in
  `main()` so webkit2gtk picks it up.
- **Config structs** use `#[derive(Debug, Clone, Default, Deserialize,
  Serialize, PartialEq)]` with `#[serde(default, deny_unknown_fields)]`. Leaves
  are `Option<String>` so partial user TOMLs merge. Do not mix a scalar leaf
  and nested tables under the same Rust struct field — split into two fields
  or an explicit sub-struct.
- **Tests** live next to their module. `src-tauri/src/config/mod.rs` carries
  happy + failure paths for `load` / `validate` / merging. When you add a
  theme group, extend `defaults_populate_every_theme_token` and
  `theme_override_preserves_untouched_tokens` too.

## TypeScript / Vue conventions

### Path aliases

Scoped aliases per concern, **not** `@/*`. Kept in sync across
`ui/tsconfig.json`, `ui/vite.config.ts`, and `ui/components.json`:

| Alias | Resolves to | Used for |
| ----- | ----------- | -------- |
| `@ipc` | `./src/ipc` | Tauri `invoke` / `listen` wrappers — tests `vi.mock('@ipc', ...)` to stub. |
| `@lib` | `./src/lib` | TS helpers; `cn` lives here. |
| `@ui` | `./src/components/ui` | shadcn-vue components. |
| `@components` | `./src/components` | Non-shadcn components (future). |
| `@composables` | `./src/composables` | Vue composables. |
| `@views` | `./src/views` | Views (Vue-only). |
| `@assets` | `./src/assets` | Styles, static assets. |

### Folder barrels

- Every folder containing TypeScript (`lib/`, `composables/`, component
  folders under `components/ui/`) must expose an `index.ts` barrel. Imports
  hit the folder, never the file directly: `import { cn } from '@lib'`, not
  `@lib/style`.
- Vue-only folders (currently `views/`) skip the barrel; import the SFC
  directly: `import Placeholder from '@views/Placeholder.vue'`.
- Rename files in one commit that also updates the barrel and every import
  site.

### Type conventions

- **Optional fields use `?` syntax.** `session_id?: string`, not
  `session_id: string | null` / `string | undefined`. Component props +
  refs follow the same rule: `defineProps<{ request?: Event }>()`,
  `ref<T>()` (undefined initial) over `ref<T | null>(null)`. Rust-side
  `Option<T>` serializes to `null` on the wire today; treat that as a
  type-lie that `?` papers over cleanly at the consumer edge. If a field
  needs to disappear entirely on `None`, add
  `#[serde(skip_serializing_if = "Option::is_none")]` on the Rust
  struct. The same rule extends to function-return objects: if the API
  exposes `{ active: ComputedRef<T | undefined> }`, drop `active` and
  let callers compute it from a non-optional collection
  (`entries.value[0]`); never bake `T | undefined` into a public
  return shape. Wrap-and-thread is fine; wrap-and-bury-undefined-as-API
  is the failure mode.
- **Function options are an object, never an overloaded union.**
  `pushToast(tone, message, options: ToastOptions = {})` — never
  `pushToast(tone, message, optionsOrDuration?: number | ToastOptions)`.
  Even when the function only has one knob today, the second knob
  arrives as a backwards-compatible `options.something?` field rather
  than as an overload. Reasons: (a) the first knob's positional shape
  hard-codes call sites that all need to flip when a second knob lands;
  (b) the union-discriminator runtime check (`typeof opts === 'number'`)
  is exactly the kind of branching the type system already does for
  free with a typed object; (c) the call-site ergonomics are uniform
  (`{ durationMs: 2000 }`, `{ action: ... }`) instead of asymmetric
  (`2000` vs `{ ... }`). Apply the same rule to internal helpers — a
  one-knob function gets a one-field object, accepting the trivial
  ceremony cost upfront.
- **Closed sets use `enum`, not union string literals.** Define
  `export enum SessionState { Starting = 'starting', … }` and type
  fields as `state: SessionState`. Union string literals
  (`state: 'starting' | 'running' | …`) are banned — the enum is the
  single source of names the TS compiler can refactor. Same rule for
  discriminator tags (`kind: EventKind.Transcript`).
- **Wire-contract strings use an `@ipc` enum, not raw literals.**
  Every Tauri `invoke` command name and `listen` event name is a
  closed-set member of the Rust → UI contract; they live as enum
  values in `ui/src/ipc/commands.ts` (`TauriCommand`, `TauriEvent`)
  and the `@ipc` barrel re-exports them. `invoke` / `listen` take
  those enum types — not `string` — so a typo at a call site fails
  type-check instead of surfacing at runtime. Tests that mock `@ipc`
  must spread `vi.importActual` so the enum re-export survives the
  mock:
  `vi.mock('@ipc', async () => ({ ...await vi.importActual(…), invoke: …, listen: … }))`.
  Adding a new Tauri command = new variant in `TauriCommand` + new
  arm in the Rust `invoke_handler![…]` macro, one PR.
- **Command → response type is a lookup map, not a generic argument.**
  `ui/src/ipc/commands.ts` pairs `TauriCommand` with
  `TauriCommandResult` (and `TauriEvent` with `TauriEventPayload`) —
  an interface keyed by enum value that points at the wire shape the
  Rust side emits for that command / event. `invoke` / `listen`
  infer the response off the map so call sites drop the explicit
  generic:
  ```ts
  // wrong — re-states the wire shape, drifts silently if it changes
  const r = await invoke<{ profiles: ProfileSummary[] }>(TauriCommand.ProfilesList)

  // right — inferred from TauriCommandResult[TauriCommand.ProfilesList]
  const r = await invoke(TauriCommand.ProfilesList)
  ```
  Consequence: every wire-contract interface (`SubmitResult`,
  `WindowState`, `Theme`, `InstanceStateEventPayload`, …) lives in
  `ui/src/ipc/types.ts` — not inline in the composable that happens
  to consume it. Moving a type out of `@ipc` into a feature file
  re-forks the contract; keep them all in `ipc/types.ts`.
  Adding a new command = new variant in `TauriCommand` + new key in
  `TauriCommandResult` + (if the response is a new shape) new
  interface in `types.ts` + new arm in the Rust `invoke_handler![…]`
  macro, one PR.
- **Named types with `T[]` suffix for arrays.** Extract every inline
  object-array type (`Array<{ option_id: string, … }>`) to a named
  interface, then use `PermissionOptionView[]` — not `Array<T>`, not
  inline. One named type per wire shape.

### Naming conventions

- **Filename casing.** `.ts` files are kebab-case
  (`use-attachments.ts`, `palette-root.ts`); `.vue` SFCs stay
  PascalCase (`ChatComposer.vue`, `Frame.vue`). Barrels re-export
  members, so most call sites import by folder
  (`import { useAttachments } from '@composables'`) and don't change
  when a file is renamed.
- **Error variable names are `err`, not `error`.** Applies to both Rust
  (`Err(err) => …`) and TypeScript (`.catch((err) => …)`, `try { … }
  catch (err) { … }`). Local refs / state that carry the last error use
  `lastErr`, `bindErr`, etc. — same short form. Mirrors the Rust
  convention already in the codebase.
- **Names are additive: scope first, noun last.** The core rule across
  the whole codebase — Rust, Vue, TypeScript. Build up identifiers by
  prepending scope tags that describe *what kind of* the noun it is.
  When two things share a backend or live in the same layer, give them
  the same prefix so they group at sort time and read as a family at
  the import site.
  - **Rust protocol types** are one instance of the rule, not the
    rule itself: `Agent` → `AcpAgent` → `AcpAgentClaudeCode`;
    `AcpAgent` sits next to `AcpAdapter`, `AcpInstance`
    because they share the ACP wire protocol. That's
    a grouping, not a universal prefix mandate — `Acp*` only makes
    sense for things that actually speak ACP. Same pattern would
    apply to a future direct-HTTP sibling: `HttpAgent` +
    `HttpAdapter` + `HttpInstances` + `HttpInstance`.
  - **Drop the scope when the whole tree already carries it.** The
    overlay IS the app — `ui/src/components/` is the overlay's
    component tree, not nested under an extra `overlay/` folder.
    The window frame is `components/Frame.vue`, the button is
    `components/Button.vue`, the toast is `components/Toast.vue`.
    Only `components/ui/` carries a distinct scope (shadcn
    primitives come from an external library and deserve their own
    namespace). Same for scoped CSS classes: `.frame`, not
    `.overlay-frame` — `<style scoped>` already hashes the selector
    per SFC. Same for composables: `ui/src/composables/` is one
    tree, everything in it wires the same overlay — names drop the
    `Acp*` prefix (`useAdapter`, `useProfiles`, `useSessionHistory`,
    `useTranscript`). A future `HttpAgent`-speaking sibling would
    slot in as `useHttpAdapter`, at which point the current file
    renames to `useAcpAdapter` per the additive rule.
  - **Keep the scope when it discriminates siblings.** A
    chat-specific turn is `ChatTurn.vue`, not just `Turn.vue`,
    because the discriminator is the chat domain, not where the
    file sits. A command-palette-specific button would be
    `CommandPaletteButton.vue` — reaching the full noun chain makes
    the intent unmistakable when a generic `Button.vue` also exists
    one level up.
  - **Group related components in subfolders**; the folder name
    doubles as the short scope. `components/chat/` holds every chat
    transcript primitive (`ChatTurn`, `ChatComposer`,
    `ChatToolChips`, …). `components/command-palette/` holds
    palette primitives (`CommandPaletteShell`, `CommandPaletteRow`,
    `CommandPaletteMiniCard`). Root `components/` holds
    scope-agnostic primitives that layer directly on the overlay
    itself (today: `Frame`, `Button`, `Pill`, `Toast`,
    `BreadcrumbPill`, `KbdHint` — future additions like settings
    dialogs or wizards slot in here too). Page-level views that
    consume components live in `views/`.
  - **Rename over aliasing.** When a name that once fit becomes
    misleading (e.g. an old `AcpTurn` that doesn't actually speak
    ACP), rename it in one commit + update every caller — never
    leave a `type AcpTurn = ChatTurn` shim. The no-legacy-compat
    rule applies here too.

  - **Names carry no redundant context.** When the scope already
    names the thing — a class instance, a composable's returned
    interface, a config struct, a module — its members drop the
    repeated noun. The scope IS the context. Specifically:

    - **Composables**: always `useFoo`. Always. The returned
      interface uses bare verbs when the noun is obvious from
      context, or short qualifiers when there are multiple of the
      same verb:
      - `useToasts() → { entries, push, dismiss, clear }` —
        not `pushToast` / `dismissToast`. The composable's name
        already says "toast"; methods use the verb only.
      - `useSessionInfo() → { info, setMode, setModel, setCwd }` —
        verb + qualifier when there's more than one thing to set,
        but **never** `setSessionInfoMode`.
      - `useAdapter() → { submit, cancel, agentsList, profilesList }`
        — methods name *what they do*, not *what they do FROM*.

    - **Component props / events**: bare adjective when the
      meaning is unambiguous given the component:
      - `Modal { dismissable }` — not `dismissableOnClickOutside`.
        If the component fires another dismissal path, that's its
        own prop.
      - `Toast { tone, body }` — not `toastTone`, `toastBody`.
      - **`@dismiss`** on a Modal — not `@modalDismiss` or
        `@onClose`. The event name is the verb.

    - **Methods on a single-purpose type**: `.set` / `.get` /
      `.list` / `.clear` when the type's job is one thing. A
      `Trust` store has `.allow(tool)` / `.deny(tool)`; a
      `Sandbox` has `.resolve(path)`; an `InstanceRegistry` has
      `.insert` / `.get` / `.shutdown`. Never `.allowTool` /
      `.resolvePath` — the parameter type already says what it
      operates on.

    - **Config structs**: field names live inside their parent's
      scope. `[ui.theme.surface] default` reads as "surface
      default colour". Don't write `surface_default_color` inside
      the surface struct. Same applies in Rust: `Capabilities {
      load_session, set_mode }` — not `Capabilities {
      capability_load_session }`.

    The smell signals: a name reads correctly in isolation but
    repeats redundantly at the call site (`useToasts().pushToast()`,
    `Modal.dismissableOnClickOutside`, `sandbox.resolveSandboxPath`).
    Strip the qualifier; rely on the call site's grammar to do the
    work the namespace already did. **Apply this rule when adding
    every new identifier — naming is the one change with zero
    runtime cost; bad names compound forever.**

### Style conventions

- **Always brace single-statement control-flow bodies in TypeScript.** Never
  write `if (cond) return x`, `if (cond) continue`, `for (…) do(x)` on one
  line — always open a scope:

  ```ts
  // wrong
  if (!agent) return []

  // right
  if (!agent) {
    return []
  }
  ```

  Applies to `if` / `else` / `for` / `while` / `do` in `.ts` and `<script
  setup lang="ts">` blocks. Reason: the one-liner hides new siblings when
  the branch grows — a second statement silently escapes the conditional
  and the bug is visible only at runtime. Braces make the scope explicit
  so the next edit can't slip outside it. Rust's `if` / `match` as
  expressions stay as-is — that's a different language contract.
- **No `__` in class names.** Use `-` as the separator — `.placeholder-header`,
  not `.placeholder__header`.
- **No `--pilot-*` CSS variables.** All theme tokens are `--theme-*`.
- **No custom animations.** Every animated primitive uses Tailwind v4's
  built-in utilities — `animate-pulse`, `animate-spin`, `animate-bounce`,
  `animate-ping`. Defining a new keyframe in `@theme {}` or scoped CSS
  is forbidden; if the visual demands a non-built-in cadence, reach for
  arbitrary-value variants (`[animation-duration:1.2s]`,
  `motion-safe:animate-pulse`) over a fresh keyframe. Discipline:
  custom animations metastasize — every primitive ends up wanting its
  own pulse / blink / fade, and the resulting menagerie is harder to
  scan than four built-ins applied uniformly.
- **`<style scoped>` in every Vue SFC, no `lang="postcss"`.** Tailwind
  v4's vite plugin only transforms virtual modules whose query ends in
  `.css`; `lang="postcss"` emits `lang.postcss` and silently bypasses
  the plugin, leaving `@apply` unresolved until lightningcss minify
  trips on it. Tailwind v4 handles `@apply` + variants + nesting
  (`&:focus`) natively inside a plain `<style>` block, so the
  `lang="postcss"` tag buys nothing and actively breaks the pipeline.
  Each scoped block that uses `@apply` starts with
  `@reference "../assets/styles.css";` — Tailwind v4 compiles isolated
  CSS chunks independently, so scoped SFC blocks need the reference
  directive to see the design tokens and utility aliases declared in
  the global stylesheet. Prefer `@apply` for layout / spacing /
  typography utilities; reserve property-level CSS for theme-variable
  reads (`background-color: var(--theme-...)`) and the rare custom
  that has no Tailwind alias.
- Tailwind utility classes use the short aliases declared in
  `ui/src/assets/styles.css::@theme inline` (e.g. `bg-theme-accent`,
  `text-theme-pending`, `border-theme-border-soft`). Add new aliases as new
  tokens land.
- Type scalar theme fields as `string`, not `string | null` — the
  defaults-always-load invariant makes nullable shapes misleading.

### UI stack reference

- **shadcn-vue** component templates live under `ui/src/components/ui/`.
  Copy-paste / `npx shadcn-vue@latest add <component>` drops them in; they
  can be edited freely.
- **reka-ui** provides headless primitives (Vue port of Radix). shadcn-vue
  components import from it.
- **class-variance-authority** (`cva`) for typed component variant APIs.
- **clsx + tailwind-merge** composed into `cn()` at `ui/src/lib/style.ts` —
  the canonical class-joining helper.

### Components compose, they don't bag

**Where consumers need rendering flexibility, accept a slot, a render
function, or a component reference — never a structured prop bag of
primitives the component pattern-matches over.** The smell: a prop
typed `actions: { id, label, tone, icon, variant }[]` or a composable
that takes `(message: string, options: { action: { label, run } })`.
The signal: when a consumer says "I need an extra knob on this item"
and the answer is "extend the type", the type is hiding a slot that
wants to exist.

**Apply this rule by default; don't wait for a second consumer.**
Even at one call site, the bag is a footgun — the next ask
("can the action button show a loading spinner?", "can the toast
have an icon?") forces a type-widening churn that a slot would have
absorbed for free. The trade is six lines of consumer composition for
permanent flexibility.

**Concrete shapes we use:**

- **`Modal`** (`components/Modal.vue`): an `#actions` slot for the
  header button row AND a default body slot taking the renderer SFC.
  Consumers pick `<MarkdownBody>` / `<TextBody>` / a custom body and
  compose `<Button>` actions:
  ```vue
  <Modal :title="..." :tone="...">
    <template #actions>
      <Button tone="err" @click="onDeny">reject</Button>
      <Button tone="ok" variant="solid" @click="onAllow">accept</Button>
    </template>
    <MarkdownBody :source="plan" />
  </Modal>
  ```
  Never `actions: ModalAction[]`. Never `markdown` / `text` /
  `richText` body-shape props on the modal — the renderer IS the
  slot content. The modal owns the chrome (backdrop, header, action
  row); the body is a composition decision the caller makes per use.

- **`pushToast`** (`composables/ui-state/use-toasts.ts`): `body` is
  `string | (() => VNode) | { component, props }`. The toast chrome
  (tone-stripe, dismiss button) is fixed; everything inside is the
  consumer's call. The simple case stays one line — `pushToast(tone,
  "session started")`. The rich case (cancel-turn toast with a delete
  button) builds a small SFC (`CancelToastBody.vue`) and passes
  `{ component, props }`. Never `(message, { action })`.

- **`Toast.vue`**: renders the body via an inline functional component
  in `<script setup>` (`function RenderBody(): VNode | null`). String
  body → `<span class="toast-message">`; render-fn → call it; component
  ref → `h(component, props)`.

**Keep prop bags only for uniform lists** of identically-shaped items
where there's no rendering decision to defer — `PlanItem[]` /
`PermissionPrompt[]` / `QueuedMessage[]` / `BreadcrumbCount[]`. If a
consumer wants per-item customisation, that's the slot signal.

**Refactor checklist when introducing a new component / composable
that takes user data:** before adding `actions: X[]` or `body:
{ message, action }`, ask: "would the consumer want to render
something I haven't typed?" If yes (or maybe), it's a slot. The
default is composition; the bag is the exception that needs
justification.

### Source layout — `src/{components,views,composables,interfaces,constants,lib,ipc}`

The top-level `ui/src/` shape is fixed:

- `components/` — **reusable, scope-agnostic** Vue building blocks. A
  component lives here when more than one feature consumes it (or
  realistically would). Anything single-feature lives under that
  feature's `views/` folder (see below).
- `views/<feature>/` — feature partials. The chat surface, the
  composer, the palette, the idle/start screen are all features —
  each owns its own `views/<feature>/` folder containing its SFCs
  AND the composables that exclusively serve them. Treat them as
  page-level slices, not "shared UI".
- `composables/` — **only** composables that more than one feature
  reads. Cross-cutting concerns (theme, keymaps, active-instance
  tracking, toasts/loading) live here. Per-feature composables live
  next to the feature's SFC files.
- `interfaces/<domain>/<sub-domain>.ts` — every TypeScript `interface`
  / `type` in the codebase lands here, organised by domain and
  sub-domain. **`types.ts` files are forbidden** — they're
  dumping grounds. Mirror the source folder structure when there's
  a natural one (e.g. `interfaces/chat/turn.ts`,
  `interfaces/composer/queue.ts`); otherwise group by wire boundary
  (`interfaces/ipc/transcript.ts`).
- `constants/<domain>/<sub-domain>.ts` — every `enum` and constant
  table. Same shape rule as `interfaces/`. **`types.ts` is not a
  place for enums either.**
- `lib/` — pure helpers (no Vue, no Tauri). `cn()`, markdown
  pipeline, image encoding, MIME dispatch, anything reusable that
  doesn't reach for reactivity.
- `ipc/` — Tauri command + event bridge. Wire types belong under
  `interfaces/ipc/<...>.ts`; `ipc/` is the *bridge*, not the type
  catalog.

When a `components/` SFC turns out to be single-feature only
(zero importers outside that feature), move it under that
feature's `views/<feature>/`. The default is "live next to your
caller"; promotion to `components/` is earned by a second consumer.

### Composables: self-contained `useX(): UseXApi` shape

**Every composable returns a typed interface.** Define
`UseFooApi` next to the composable (or in `interfaces/<domain>/`)
and have `useFoo()` return that interface explicitly. Export the
interface alongside the function. Example:

```ts
export interface UseAdapterApi {
  submit: (args: SubmitArgs) => Promise<SubmitResult>
  cancel: (args: CancelArgs) => Promise<CancelResult>
  agentsList: () => Promise<AgentSummary[]>
  // ...
}

export function useAdapter(): UseAdapterApi {
  return { submit, cancel, agentsList /* ... */ }
}
```

The shape forces every consumer-facing knob into the interface;
internal helpers stay file-local. **No drive-by exports** — if a
function is exported from a composable file, it MUST be in the
interface returned by the `useX()` factory. Module-level
`pushFoo` / `setFoo` / `lookupX` exports are the smell — those
should be methods on the returned interface OR moved to a
sibling helper file with no `useX` shape.

**Test-only helpers** (`__resetFooForTests`, `__seedX`) belong
in `tests/<feature>/<helper>.ts`, not the production module. The
production composable knows nothing about test scaffolding; the
test file imports both the composable and its test helper from
`tests/`.

### Component composition contract — caller passes the renderer

**Where a component renders user-supplied content (markdown body,
plain-text body, custom row, custom badge), the consumer passes
the renderer in via slot or component prop — the component never
hardcodes a "what to render" branch.**

The canonical example: `Modal.vue` accepts a `body` slot AND
exports reusable body renderers (`ModalMarkdownBody`,
`ModalPreBody`, …) the caller can drop in. Consumers compose:

```vue
<Modal title="plan" tone="warn">
  <ModalMarkdownBody :source="plan" />
</Modal>
```

…or pass any other component. Same shape applies to `Toast`,
`Modal`, list rows, banners — anywhere the chrome is fixed and
the body is variable. The component owns layout / chrome /
interaction state; the consumer owns the inner render. Reusable
renderers live alongside the component (`Modal.vue` +
`ModalMarkdownBody.vue` + `ModalPreBody.vue` in the same folder).

This is a stronger statement of the **components compose, they
don't bag** rule above: the component shouldn't even pattern-match
on a `body` discriminator (`string | () => VNode | { component }`)
when a slot would do. Reach for the discriminator only when the
caller is a non-Vue composable (e.g. `pushToast` accepts a
component ref because the toast queue isn't a template scope).

### Icons — direct imports only, no `library.add(...)` registry

FontAwesome (`@fortawesome/fontawesome-svg-core` `library.add(...)`)
is **forbidden**. Never register icons centrally. Each component
imports the specific icons it uses directly:

```ts
import { faCircle, faCheck } from '@fortawesome/free-solid-svg-icons'
```

…and binds them in the template via the explicit object form:

```vue
<FaIcon :icon="faCircle" />
```

Not `<FaIcon :icon="['fas', 'circle']" />` — the string-array
indirection defeats Vite's tree-shaking and forces every icon
into the boot bundle. Direct imports + direct prop binding lets
the bundler drop unused icons per component.

### `invoke()` / `listen()` typing — interface-indexed args, no `Record<string, unknown>`

**Every Tauri command's argument shape is in
`interfaces/ipc/invoke.ts` (or similar) keyed by the
`TauriCommand` enum.** `invoke()` / `listen()` infer the args
type from the command name; consumers pass typed objects. No
`args?: Record<string, unknown>` on the bridge.

```ts
// in interfaces/ipc/invoke.ts
export interface TauriCommandArgs {
  [TauriCommand.SessionSubmit]: SessionSubmitArgs
  [TauriCommand.SessionCancel]: SessionCancelArgs | undefined
  // ...
}

// in ipc/bridge.ts
export async function invoke<K extends TauriCommand>(
  command: K,
  args: TauriCommandArgs[K]
): Promise<TauriCommandResult[K]> { /* ... */ }
```

Mirror the existing `TauriCommandResult` map. `undefined` for
no-args commands — never overload the call signature with
optional args.

**No named wrapper functions** like `getProfiles()` /
`listSessions()` / `submitTurn()`. Always
`invoke(TauriCommand.X, args)` at the call site. Wrappers
duplicate the type contract and drift from it; the typed
`invoke()` IS the API.

### Tool formatter registry — composable + per-adapter init

The tool-formatter system (`lib/tools/registry.ts`) becomes a
composable: `useToolRegistry()` returns the registry shape,
adapters register their formatters at init. Per-adapter
divergence (claude-code's `Switch mode`, codex's `bash_id`,
opencode's `diagnostics`) lands as registration calls, not
hand-edited lookup tables. The base set still ships with the
adapter-agnostic formatters; adapters extend.

### Dev preview shim lives in `tests/`, gated by env var

`ui/src/dev.ts` (the browser-mode theme + window-state shim)
moves to `tests/` because its consumers are **the test
harness and the Vite dev preview only — never production**. The
boot path in `main.ts` flips to an env-var gate:

```ts
if (import.meta.env.VITE_HYPRPILOT_DEV_PREVIEW === '1') {
  const { applyDevPreview } = await import('../tests/dev-preview')
  applyDevPreview()
}
```

Not a `__TAURI_INTERNALS__` window probe (browser-detection by
absence of a Tauri property is fragile and surprising). The dev
script set `VITE_HYPRPILOT_DEV_PREVIEW=1` in the dev `.env`;
production builds leave it unset. Vitest fixtures import the
preview directly from `tests/dev-preview` when they need themed
DOM.

## Frontend linting / formatting

The `ui/` package consumes the workspace-wide config at
`https://gitlab.kilic.dev/config/eslint-config`:

- `ui/eslint.config.mjs` imports the `@cenk1cenk2/eslint-config/vue-typescript`
  subpath and appends `utils.configImportGroup` — mirrors `utils/md-printer`.
  A local parser override re-applies `vue-eslint-parser` + `typescript-eslint`
  for `<script setup lang="ts">` blocks because the upstream
  `createConfig({ extends: [] })` call skips the parser-insertion path.
- `ui/.prettierrc.mjs` re-exports `@cenk1cenk2/eslint-config/prettierrc`.
- `eslint` is pinned to `^9.39.4`. `eslint-plugin-import@2.32.0` (transitive
  dep) calls `SourceCode` APIs removed in eslint 10; upgrade once the
  `config/eslint-config` workspace switches to `eslint-plugin-import-x`.

Do not add ad-hoc rules to either config file without updating this manual.

## Agents

- `.mcp.json` at the repo root is the repo-scoped MCP server registry. Starts
  empty — add servers you need during a task, remove them at merge if they
  aren't load-bearing.
- Every issue is picked up in a dedicated branch (worktree optional). Never
  implement on `main`.
- Issue workflow (see the Linear project description for the full contract):
  `linear-issue-implement` → `git-branch` → `agents-sequential` /
  `agents-team` → `git-commit` → `gitlab-pr-create` → review → merge.
- Commit style: conventional commits with a `refs K-<id>` or `closes K-<id>`
  trailer referencing the issue the branch targets.
- Prefer MCP tools over CLIs for git, GitLab, Linear, Obsidian, Tmux, etc.
  Fall back to CLI only when the MCP server lacks the operation.

## JSON-RPC over the daemon socket

The `ctl` subcommands and the daemon talk over
`$XDG_RUNTIME_DIR/hyprpilot.sock` using newline-delimited JSON (NDJSON) —
one JSON-RPC 2.0 object per line, both directions. Implementation lives
in `src-tauri/src/rpc/`; the client is `src-tauri/src/ctl/client.rs`.
Every accept spawns a per-connection task so a slow / misbehaving peer
can't block others.

### Methods

| Method | Params | Result | Notes |
| ------ | ------ | ------ | ----- |
| `session/submit` | `{ "text": "...", "instance_id"?: "<uuid>", "agent_id"?: "...", "profile_id"?: "..." }` | `{ "accepted": true, "agent_id": "...", "profile_id": "..." \| null, "instance_id": "<uuid>", "session_id": "..." \| null }` | When `instance_id` is given, routes to (or adopts) that UUID so the webview can push its user turn optimistically against the key it just generated. When omitted, mints a fresh UUID and spawns a new instance for the resolved `(agent, profile)` — N twins of the same profile are addressable by distinct UUIDs. |
| `session/cancel` | *(none)* or `{ "instance_id"?: "<uuid>", "agent_id"?: "..." }` | `{ "cancelled": bool, "reason"?: "..." }` | `instance_id` addresses a specific live instance (preferred); `agent_id` is the legacy fallback that cancels the first live instance of that agent. |
| `session/info` | *(none)* | `{ "sessions": [...] }` | Live session list across every active agent + profile. |
| `window/toggle` | *(none)* | `{ "visible": bool }` | Flips main window visibility; updates `StatusBroadcast` visible bit. |
| `overlay/present` | `{ "instanceId"?: "<uuid>" }` | `{ "visible": true, "focusedInstanceId": "<uuid>" \| null }` | Show + focus the overlay (idempotent). When `instanceId` is given, also focuses that instance via `Adapter::focus`. Hyprland-bind surface (e.g. `bind = SUPER, space, exec, hyprpilot ctl overlay toggle`). |
| `overlay/hide` | *(none)* | `{ "visible": false }` | Hide the overlay (idempotent; webview stays warm). |
| `overlay/toggle` | *(none)* | `{ "visible": bool }` | Flip the overlay's visibility. Race-safe across concurrent calls — every `overlay/*` entry serialises through `WindowRenderer::lock_present`. |
| `daemon/kill` | *(none)* | `{ "exiting": true }` | Calls `app.exit(0)` after write + flush. Delivery is best-effort: the process may tear down before the peer finishes reading. |
| `daemon/status` | *(none)* | `{ "pid", "uptimeSecs", "socketPath", "version", "instanceCount" }` | Snapshot for support tickets / `ctl daemon status`. |
| `daemon/version` | *(none)* | `{ "version", "commit"?, "buildDate"? }` | Version string (`CARGO_PKG_VERSION`). `commit` / `buildDate` populate when `HYPRPILOT_BUILD_COMMIT` / `HYPRPILOT_BUILD_DATE` env vars are present at compile time. |
| `daemon/reload` | *(none)* | `{ "profiles", "skillsCount", "mcpsCount" }` | Re-runs `config::load` against the original CLI overlay layers, then `SkillsRegistry::reload()`. Publishes a `DaemonReloaded` event on the registry broadcast (Tauri name: `daemon:reloaded`; topic: `daemon.reloaded`). |
| `daemon/shutdown` | `{ "force"?: bool }` | `{ "exiting": true }` | Graceful counterpart to `daemon/kill`. Without `--force`, refuses with `-32603` when any instance has an in-flight turn (busy = an emitted `TurnStarted` without matching `TurnEnded`). The `data` payload carries `{ counts: { instances, busyInstances }, busyInstanceIds }`. |
| `diag/snapshot` | *(none)* | `{ daemon, instances, profiles, skills, mcps, configPaths }` | Read-only structural dump for "what is this daemon doing" tickets. **Redacted**: profile `env` values + transcript bodies never appear. |
| `status/get` | *(none)* | `StatusResult` | One-shot status snapshot. |
| `status/subscribe` | *(none)* | `StatusResult` (initial) | Registers connection as subscriber; server pushes `status/changed` notifications. |
| `status/changed` | `StatusResult` | *(notification, no id)* | Server-push on every state transition. Clients receive this after `status/subscribe`. |
| `config/profiles` | *(none)* | `{ "profiles": [{ id, agent, model, is_default }] }` | Read-only profile list for the chat-shell picker (K-246). |

`StatusResult` shape: `{ "state": "idle" | "streaming" | "awaiting" | "error", "visible": bool, "active_session": string | null }`.

**Namespace convention.** Every method name on the wire uses the
`namespace/name` form, matching ACP's own methods (`session/prompt`,
`session/new`):

- `session/*` — anything scoped to an agent session (prompts, cancel, info).
- `window/*` — overlay window state (`window/toggle`; future
  `window/show`, `window/hide`, `window/focus`).
- `overlay/*` — race-safe present/hide/toggle for hyprland-bind users;
  accepts `instanceId` to focus alongside the present.
- `daemon/*` — daemon lifecycle / introspection (`daemon/kill`,
  `daemon/status`, `daemon/version`, `daemon/reload`,
  `daemon/shutdown`).
- `diag/*` — read-only operator diagnostics (`diag/snapshot`).
- `status/*` — agent state broadcasts (drives waybar).
- `config/*` — read-only config slices consumed by UI pickers
  (`config/profiles` today; future `config/agents`).
- Reserved: `agents/*` (listing / switching), `permissions/*` (trust
  store — UI-only today, no `ctl` surface yet).

Bare method names — the pre-K-239 `submit` / `cancel` / `toggle` / `kill`
/ `session-info` scaffold — are intentionally dead. Clients hitting them
receive `-32601 method not found`; there is no backwards-compat layer.

Methods without params (`session/cancel`, `session/info`, `window/toggle`,
`daemon/kill`) omit the `params` key entirely — the server accepts
`{"method":"window/toggle"}` with no `params` and responds normally.
`status/changed` is a server-push notification — it carries no `id` and
is not a response to a request.

Request ids on the client side are per-call UUID v4 strings
(`uuid::Uuid::new_v4().to_string()`). The server treats ids as opaque and
echoes them verbatim; any `RequestId` variant (`Number` or `String`) is
accepted on the wire.

### Error codes

The server surfaces JSON-RPC 2.0 standard error codes:

- `-32700` parse error (invalid JSON on the wire). `id` echoes as `null`.
- `-32600` invalid request (valid JSON, wrong shape — missing `jsonrpc`,
  bad version, malformed params).
- `-32601` method not found.
- `-32603` internal error (handler failed — `window/toggle` against a
  missing window, serializer failures, etc.).

`-32000 ..= -32099` is reserved for hyprpilot-specific errors; none are
defined yet.

### Design notes

- **Framing**: NDJSON on top of `tokio::io::BufReader::lines`. Matches
  what ACP uses on its own pipe, so future ACP work reuses the same
  framing primitives.
- **Dispatcher**: hand-rolled on `serde_json`. `rpc::server::dispatch_line`
  parses the envelope (`jsonrpc`, `id`, `method`, `params`) directly off
  a `serde_json::Value` — there is no typed `Call` / `Request` enum
  between the wire and the handlers (removed in round 3; see
  "no backwards-compatibility layers" in Rust conventions). Each
  handler implements `RpcHandler` and parses its own `params: Value`
  into a typed struct with `serde_json::from_value`, surfacing
  `-32602 invalid_params` on deserialization failure. Extending the
  RPC surface = one new `RpcHandler` impl + one line in
  `RpcDispatcher::with_defaults`. `jsonrpsee` / `jsonrpc-v2` would be
  heavier than warranted here; revisit if method count crosses ~20.
- **No auth**: single-user assumption. We don't check `SO_PEERCRED` or
  similar. Revisit when a multi-user deployment is a real concern.
- **`ctl` is one-shot** for most commands: no retry / reconnect. A connection failure
  (`ENOENT` / `ECONNREFUSED`) prints `"hyprpilot daemon is not running"`
  to stderr and exits `1`.
- **`ctl status --watch` is persistent**: after `status/subscribe` the
  connection stays open and the server pushes `status/changed` notifications.
  The client reconnects with back-off (1s → 2s → 5s) on socket loss, emitting
  an offline payload between attempts so waybar always has valid output.
- **`StatusBroadcast`** (`src-tauri/src/rpc/status.rs`): wraps a `tokio::sync::broadcast`
  channel (capacity 32) + a `Mutex<StatusResult>` snapshot. `set_visible()` is
  called from the `toggle` handler; K-239's ACP bridge will call `set()` for
  agent-state transitions. Slow consumers drop messages — waybar re-renders from
  the next tick.

### Client-side handler pattern (`ctl`)

The `ctl` CLI mirrors the server's `RpcHandler` split. One struct per
subcommand, one trait, a shared connection factory — clap dispatches
subcommand → handler instance → `handler.run(&client)`:

```rust
// src-tauri/src/ctl/client.rs
pub struct CtlClient { socket: PathBuf }
impl CtlClient {
    pub fn connect(&self) -> Result<CtlConnection> { /* ... */ }
}

pub struct CtlConnection { /* unix socket + BufReader */ }
impl CtlConnection {
    pub fn fire<Req, Resp>(&mut self, method: &str, params: Req) -> Result<Resp>
    pub fn call(&mut self, method: &str, params: Value) -> Result<Outcome>
    pub fn into_reader(self) -> BufReader<UnixStream>   // for subscription streams
}

// src-tauri/src/ctl/handlers.rs
pub trait CtlHandler {
    fn run(self, client: &CtlClient) -> Result<()>;
}

pub struct SubmitHandler  { pub text: String }
pub struct CancelHandler;
pub struct ToggleHandler;
pub struct KillHandler;
pub struct SessionInfoHandler;
pub struct StatusHandler { pub watch: bool }
```

**Why `&CtlClient` (factory) and not `CtlConnection`:** `StatusHandler
--watch` reconnects in a loop with back-off when the socket drops;
that needs the path, not a live connection. One-shot handlers call
`client.connect()?` once and exit. Passing the factory satisfies both
without branching in the trait or leaking "is this a streaming
handler?" into the dispatcher.

**Why the trait with zero-sized structs and not a free fn per
subcommand:** uniformity with the server's `RpcHandler`, and a single
place (`ctl::run`'s match) where clap maps subcommands to wire
behavior. Adding a subcommand is one new struct + one new `impl
CtlHandler` + one new match arm — no changes to existing handlers.

**Status is the only non-plain handler.** Everything status-specific
lives on `StatusHandler` as associated functions:
`one_shot(client)` / `watch_loop(client)` / `stream_once(client)` /
`subscribe(conn)` / `offline()` / `format(status)`. The
`StatusChangedNotification` stream and the `StatusStream` iterator
type both live in `handlers.rs` next to `StatusHandler`, not on
`CtlConnection` — the transport layer stays generic. `StatusHandler`
also never exits non-zero; waybar's `exec` expects a valid JSON
payload even when the daemon is down, so transport / RPC errors fall
through to the `offline()` sentinel and exit 0.

**Shared helper:** `connect_and_print(client, method, params)` is the
body for the five plain subcommands that differ only in method +
params (`submit` stub, `cancel` stub, `toggle`, `kill`, `session-info`
stub). RPC error or serialization failure → `error!(...)` + stderr
message + `exit(1)`.

The `Submit` / `Cancel` / `SessionInfo` handlers hit the live
`session/submit` / `session/cancel` / `session/info` wire methods
today — those go through `AcpAdapter` on the server side, which
returns pre-live-session stubbed shapes (`{ accepted: true, text }`
/ `{ cancelled: false, reason }` / `{ sessions: [] }`) until the
runtime plumbing lands.

## ACP bridge (K-239 scaffold + K-240 live runtime + K-242 profiles)

The daemon fronts one or more ACP-speaking agent subprocesses.
`session/submit` resolves the addressed profile (or falls back
through `[agent] default_profile` → first `[[agents]]` entry),
spawns the configured vendor on first hit, wires a
`Client.builder().connect_with(transport, …)` pipeline against its
stdio, and streams `SessionUpdate`s through to the webview
(`acp:transcript` Tauri events) + the `ctl status` broadcast. Follow-up
prompts against the same `(agent_id, profile_id)` pair reuse the live
session; a different profile against the same agent spawns its own
actor so system-prompt + model overlays stay deterministic.

### Module layout (`src-tauri/src/adapters/`)

The generic adapter layer + the ACP impl as one transport among
many. `rpc::` / `ctl::` / `daemon::` talk to `dyn Adapter` or to the
concrete `AcpAdapter` re-exported from `adapters::*`; they never
`use crate::adapters::acp::*` directly (enforced by the
`no_acp_imports_outside_adapters` test in `adapters/mod.rs::tests`).

- **Generic layer** (`src-tauri/src/adapters/`):
  - `mod.rs` — `Adapter` trait + `AdapterId` + `Capabilities` +
    `AdapterError` + `Bootstrap`. Re-exports `AcpAdapter`,
    `AdapterRegistry`, `InstanceActor`, `InstanceInfo`,
    `InstanceKey`, `SpawnSpec` for out-of-layer consumers.
  - `registry.rs` — `AdapterRegistry<H: InstanceActor>`. Generic
    instance registry. Owns the `HashMap<InstanceKey, Arc<H>>`, the
    insertion-order vec, the focused-id pointer, and the
    `broadcast::Sender<InstanceEvent>` the per-instance actors
    publish onto. ACP adapter composes it; future `HttpAdapter`
    will too. See the "Composable registry" section below.
  - `commands.rs` — Tauri `#[command]`s: `session_submit`,
    `session_cancel`, `agents_list`, `profiles_list`,
    `session_list`, `session_load`, `permission_reply`. Dispatch
    through the concrete `Arc<AcpAdapter>` today; the generic RPC
    surface (`rpc::handlers::*`) dispatches through `Arc<dyn Adapter>`.
  - `instance.rs` — `InstanceKey` (UUID newtype, empty-string
    rejecting `parse`), `InstanceState`, `InstanceEvent` (with
    `topic()` returning the dot-separated wire topic),
    `InstanceEventStream`, `InstanceInfo`, `InstanceActor` trait,
    `SpawnSpec`.
  - `transcript.rs` — `TranscriptItem`, `TurnRecord`,
    `ToolCallRecord`, `UserTurnInput`, `Attachment`, `Speaker`.
  - `permission.rs` — `PermissionPrompt`, `PermissionReply`,
    `PermissionOptionView`.
  - `tool.rs` — `ToolCall`, `ToolCallContent`, `ToolState`.
  - `profile.rs` — `ResolvedInstance` (carries `mode`) + re-exports
    `AgentConfig`, `ProfileConfig`, `AgentProvider` from `config::`.
- **ACP impl** (`src-tauri/src/adapters/acp/`):
  - `mod.rs` — re-exports `AcpAdapter`.
  - `agents/{mod,claude_code,codex,opencode}.rs` — `AcpAgent` trait +
    three vendor unit structs. `match_provider_agent(provider)`
    resolves a `Box<dyn AcpAgent>` off the closed `AgentProvider`
    enum.
  - `resolve.rs` — thin re-export of
    `crate::adapters::profile::ResolvedInstance`.
  - `spawn.rs` — `spawn_agent(&AgentConfig, system_prompt:
    Option<&str>)` — wraps `AcpAgent::spawn` +
    `AcpAgent::inject_system_prompt`.
  - `client.rs` — `AcpClient` — `on_receive_*` handlers the ACP
    `Client.builder()` takes. `SessionNotification`s fan out onto a
    per-instance mpsc.
  - `runtime.rs` — one `tokio::spawn`ed actor per instance. Takes a
    `ResolvedInstance` + `InstanceKey` and drives `initialize` →
    `session/new` → `session/prompt` for the first prompt, then
    loops on an mpsc of `InstanceCommand::{Prompt, Cancel,
    ListSessions, Shutdown}`. Publishes ACP-shape `InstanceEvent`s;
    `AcpAdapter` bridges onto the generic `adapters::InstanceEvent`.
  - `instance.rs` — `AcpInstance` per-actor handle.
    `impl InstanceActor` surfaces identity + mode via
    `InstanceInfo`; `shutdown()` sends `InstanceCommand::Shutdown`
    and waits up to 2s for the ack.
  - `instances.rs` — `AcpAdapter`. Composes
    `Arc<AdapterRegistry<AcpInstance>>`. Owns the config, the
    permission controller, and the runtime-events → generic-events
    bridge task. ACP-specific methods (`submit_text`,
    `cancel_active`, `spawn_instance`, `restart_instance`,
    `shutdown_instance`, `focus_instance`, `list_sessions`,
    `load_session`, `list_agents`, `list_profiles`) stay here;
    generic methods delegate to the registry via `impl Adapter for
    AcpAdapter`.
  - `mapping.rs` — `From` / `TryFrom` bridges between ACP wire DTOs
    and the generic `adapters::*` vocabulary.

### Composable registry

The generic `AdapterRegistry<H: InstanceActor>` is the single
owner of per-transport instance state. Every adapter facade
(`AcpAdapter` today, `HttpAdapter` later) composes
`Arc<AdapterRegistry<TheirInstance>>` and implements the generic
methods (`list` / `focus` / `shutdown_one` / `restart` /
`info_for` / `subscribe`) as one-line delegations. The facade
owns the transport-specific bits (resolve / spawn / submit /
cancel / load_session); the registry owns the shared machinery.

**Auto-focus policy:**

- **Empty-registry → first-spawn.** The very first `insert` on an
  empty registry auto-focuses the new key and emits
  `InstancesFocused` in addition to `InstancesChanged`. UI never
  sees an empty-focus state after launch.
- **Shutdown of focused → oldest survivor.** Dropping the focused
  key reassigns focus to `order.first()` (insertion-order
  oldest). Registry empties → focus clears to `None` + emits
  `InstancesFocused { instance_id: None }`.
- **Restart preserves slot.** `restart` goes
  `drop_preserving_slot` → `insert_at_slot(slot, same_key,
  new_handle)`. The `InstanceKey` (UUID) is preserved across the
  swap too, so subscribers stay bound.

**Documented races:**

- `shutdown_one` releases all registry locks before awaiting the
  actor's shutdown ack (2s timeout). A concurrent `insert`
  between drop and the auto-focus step can land on
  `order.first()`; that's the value auto-focus picks. Callers
  reconcile via the `InstancesFocused` event stream.
- `focus` holds the `instances` + `focused` locks across the
  membership check + focus write (TOCTOU-safe). A `shutdown_one`
  on another task serializes; one wins, the other reports
  `InvalidRequest`.

**Broadcast + lagged contract:** `AdapterRegistry::subscribe`
returns a `broadcast::Receiver<InstanceEvent>` over a capacity-256
channel. Every consumer MUST handle
`broadcast::error::RecvError::Lagged` or the channel silently
drops notifications. Today the Tauri bridge +
`runtime_events_bridge` each log + continue on Lagged; K-276
(ctl-side subscribe) + K-277 (UI delta-subscribe) must do the
same in their own subscriber loops.

**Topic naming — two axes:**

- **Tauri event names** (colon-separated, consumed by
  `app.emit`): `acp:instance-state`, `acp:transcript`,
  `acp:permission-request`, `acp:instances-changed`,
  `acp:instances-focused`.
- **Topic strings** (dot-separated, returned by
  `InstanceEvent::topic()`): `instance.state`,
  `instance.transcript`, `instance.permission_request`,
  `instances.changed`, `instances.focused`. Used by tracing
  spans today and the K-276 subscription filter layer.

Both wire shapes are stable; the two axes carry distinct
conventions (Tauri-side colons / dot-separated topics) so neither
bleeds into the other.

### Per-vendor system-prompt injection

One hook, one return value. `AcpAgent::inject_system_prompt(cmd,
prompt) -> SystemPromptInjection` runs at spawn time and either:

- mutates `cmd` directly (CLI flag, `-c` override, env var) and
  returns `SystemPromptInjection::Handled`; or
- leaves `cmd` alone and returns
  `SystemPromptInjection::FirstMessage(text)`, in which case the
  runtime prepends `text` onto the first `session/prompt` text block
  (with `\n\n` separation) and clears the slot — follow-up prompts
  pass through untouched.

Default returns `Handled` (silent drop). Vendors pick exactly one
strategy.

| Vendor | Strategy | Reason |
| ------ | -------- | ------ |
| `acp-claude-code` | `FirstMessage` | `@zed-industries/claude-code-acp` never reads `process.argv`; its only hook is `_meta.systemPrompt` on `session/new`, which `agent-client-protocol` 0.11 doesn't expose as a typed field yet |
| `acp-codex` | `Handled` (mutates `cmd` with `-c instructions="<json-escaped>"`) | codex-acp forwards argv to the native `codex-acp` binary, which merges `-c` overrides into the TOML config. JSON escapes (via `serde_json::to_string`) are a valid subset of TOML basic string escapes |
| `acp-opencode` | `FirstMessage` | No launch-time hook exists today |

### `agent-client-protocol` 0.11 runtime notes

The 0.11 crate exposes a builder API — `Client.builder()
.on_receive_notification(…) .on_receive_request(…)
.connect_with(transport, main_fn)` — whose futures are all `Send`. No
`LocalSet` or `current_thread` runtime is required; the daemon stays on
the default Tauri-managed multi-thread runtime. Transport is
`ByteStreams::new(stdin.compat_write(), stdout.compat())` over the
agent subprocess's stdio.

### Agents + profiles config (flattened at TOML root)

```toml
[agent]                          # singleton: global agent-scope config
default = "claude-code"          # agent id when nothing else picks one
default_profile = "ask"          # profile id used when submit gets no profile_id

[[agents]]                       # registry: per-agent entries
id = "claude-code"
provider = "acp-claude-code"     # closed enum; see AgentProvider
model = "claude-sonnet-4-5"      # optional; translated per vendor at spawn time
command = "bunx"                 # optional; defaults to the vendor's own
args = ["--bun", "@zed-industries/claude-code-acp"]

[agents.env]                     # optional per-agent env overlay

[[profiles]]                     # registry: per-profile presets
id = "strict"
agent = "claude-code"            # must reference a real [[agents]] id
model = "claude-opus-4-5"        # optional override — profile > agent > vendor
system_prompt = "..."            # inline (mutually exclusive with system_prompt_file)
# system_prompt_file = "~/.config/hyprpilot/prompts/strict.md"
```

Singular `[agent]` parallels plural `[[agents]]` / `[[profiles]]` —
TOML's single-table vs array-of-tables distinction carries the
"global config vs registry" split. Future global knobs (shared env
overlay, timeout, cwd defaults) slot under `[agent]` without another
top-level rename.

Merge semantics (shared by `[[agents]]` and `[[profiles]]`): user
entries with an existing `id` override the whole default entry; new
`id`s append. Whole-entry replace, no field-level merge inside an
entry — "override `system_prompt`, keep old `model`" would read
surprising.

Cross-field rules inside the garde derive:

- `agent.default` → `[[agents]].id` (must match).
- `agent.default_profile` → `[[profiles]].id` (must match).
- `profile.agent` → `[[agents]].id` (must match).
- `profile.system_prompt` XOR `profile.system_prompt_file` — pair
  exclusion checked post-walk in `Config::validate`.

`AgentProvider` is a **closed enum** keyed by wire name
(`acp-claude-code` / `acp-codex` / `acp-opencode`); adding a provider
means a new enum variant + a new `AcpAgent` impl + a new match arm
in `match_provider_agent`.

### Shutdown orchestration

Process lifecycle lives in `daemon`, not `rpc`. `daemon::shutdown(app,
adapter)` is the one orchestrator; it drains adapter instances via
`AcpAdapter::shutdown_all`, then calls `app.exit(0)` (which closes
webviews, drops every `app.manage(...)` value — flushing the tracing
`WorkerGuard`, the `StatusBroadcast`, and the socket listener — and
exits with code 0).

Four call sites funnel through this one fn:

1. **`daemon/kill` RPC** — `DaemonHandler` returns
   `{"killed": true}` in the result; `rpc::server::handle_connection`
   inspects the payload after the flush and calls
   `daemon::shutdown`. No side-channel flag threaded through the
   dispatcher tuple — the marker is the response itself, so any
   future respond-then-shut-down handler just emits the same
   `{"killed": true}` shape.
2. **`daemon/shutdown` RPC** — graceful counterpart with a busy
   check (`AcpAdapter::busy_instance_ids`). Refuses with `-32603`
   when any instance has an in-flight turn unless `force = true`.
   On accept, returns `{"exiting": true}`; the same flush-then-shutdown
   path inspects either marker (`killed` OR `exiting`) so adding
   another graceful surface costs zero new code.
3. **SIGINT (Ctrl-C)** — tokio signal task spawned in `daemon::run`.
4. **SIGTERM** — same task; systemd / `pkill` both use this.

First signal triggers the orchestrator; a second signal during
shutdown falls through to the default handler (force-kill) — standard
Unix "SIGINT-twice" escape.

Socket file is not explicitly removed — next-start probes stale
sockets via `ECONNREFUSED`, which also covers the crash case.

### Permissions are the vendor's concern

ACP itself just delivers a `PermissionOption[]` array per
`session/request_permission` and expects the client to pick one
option id. Hyprpilot does **not** ship a policy layer on top of
that: claude-code-acp's plan mode, codex-acp's approval modes,
and opencode's tool filters already give users granular permission
control — re-implementing a three-way `ask` / `accept-edits` /
`bypass` knob here would just duplicate vendor behavior poorly.

The daemon forwards every permission request straight to the
webview as an `acp:permission-request` Tauri event; the user picks
an option via the dialog and replies with `permission_reply`.

Client-side auto-accept / auto-reject lives on the
`PermissionController` and runs as a unified two-lane pipeline:

1. **Runtime trust store** — `(instance_id, tool_name) → Allow|Deny`,
   populated by the UI's "always allow" / "always deny" buttons via
   `permission_reply { remember }`. Cleared on instance shutdown /
   restart. In-memory only (no disk persistence yet).
2. **Per-server hyprpilot extension globs** — each MCP JSON entry's
   optional `hyprpilot.autoAcceptTools` / `autoRejectTools` glob
   lists. Tool→server attribution by `mcp__<server>__<tool>` prefix
   convention (shared across claude-code-acp / codex-acp /
   opencode-acp).

Reject beats accept inside each lane. Vendor-native tools (Bash, Read,
…) carry no `mcp__` prefix and skip lane 2 entirely — they only
short-circuit when the captain has clicked an "always" button. Misses
on both lanes fall through to AskUser.

### Tauri commands + events (live)

| Command | Purpose |
| ------- | ------- |
| `session_submit { text, attachments?, instance_id?, agent_id?, profile_id? }` | Webview compose-box submit. `instance_id` is a client-generated UUID the overlay mints on first turn (`crypto.randomUUID()`); subsequent submits pass the same id to continue the session. `attachments: Attachment[]` carries palette-picked skills (slug + path + body + optional title) — see "User-turn input + attachments" below. Delegates to `AcpAdapter::submit_prompt`. |
| `session_cancel { agent_id? }` | Mid-turn cancel. Sends `CancelNotification` to the addressed session. |
| `permission_reply { session_id, request_id, option_id, remember?, instance_id?, tool? }` | UI replies to a pending permission. `option_id` is either an ACP option id or one of the synthetic shortcuts `'allow'` / `'deny'` resolved by `pick_*_option_id`. `remember = 'allow' \| 'deny'` (combined with `instance_id` + `tool`) writes a runtime trust-store entry so future calls of the same tool short-circuit at decide() lane 1 — that's the UI's "always allow / always deny" path. |
| `agents_list` | Populates the agent-switcher dropdown from the `[[agents]]` registry. |
| `profiles_list` | Populates the profile picker from `[[profiles]]`; parallels the `config/profiles` wire method. |
| `session_list { instance_id?, agent_id, profile_id?, cwd? }` | Calls ACP `session/list` through the addressed instance when `instance_id` resolves to a live actor; otherwise resolves via `(agent, profile)` and spawns an ephemeral `Bootstrap::ListOnly` actor (initialize → list → shutdown, never registered). Returns the raw ACP `ListSessionsResponse` — agent owns storage, hyprpilot just passes through. |
| `session_load { instance_id?, agent_id, profile_id?, session_id }` | Tears down any live handle at the given `instance_id` (minting a fresh UUID when omitted), then starts a fresh actor with `Bootstrap::Resume(session_id)`. Gated on `InitializeResponse.agent_capabilities.load_session` — vendors that don't advertise resume get a `-32601`-shaped error. Replay streams through the normal `acp:transcript` fanout. |

| Event | Payload | When |
| ----- | ------- | ---- |
| `acp:transcript` | `{ agent_id, instance_id, session_id, update }` | Every `SessionNotification` the agent streams; `update` is the raw `SessionUpdate` JSON. |
| `acp:permission-request` | `{ agent_id, instance_id, session_id, options }` | Every `session/request_permission` — auto-denied but surfaced for observability. |
| `acp:instance-state` | `{ agent_id, instance_id, session_id?, state }` | On every lifecycle transition (`starting` / `running` / `ended` / `error`). Renamed from `acp:session-state` in K-251; the event now addresses our instance owner, not the ACP wire session. |

Event names use `:` (Tauri-side convention); the JSON-RPC wire keeps
`/` (`session/submit` etc.); config uses `.`; CSS uses `-`.

### User-turn input + attachments

`UserTurnInput::Prompt { text, attachments }` is the only variant
today (the `Text(String)` variant + `expand_tokens` inline-token
expansion were deleted end-to-end in K-268). `Attachment` is the
generic palette-picked-context shape:

```rust
pub struct Attachment {
    pub slug: String,        // "git-commit"
    pub path: PathBuf,       // /home/.../skills/git-commit/SKILL.md
    pub body: String,        // snapshot at pick time
    pub title: Option<String>,
}
```

ACP mapping in `adapters/acp/mapping.rs::build_prompt_blocks`
projects each attachment onto a `ContentBlock::Resource` carrying
an `EmbeddedResource { resource: TextResourceContents { uri:
"file://<path>", mime_type: Some("text/markdown"), text: <body> } }`,
prepended before the trailing `ContentBlock::Text` — the agent
reads context first, then the user's instructions. Body
snapshots at palette-pick time so what the user sees is what
the agent receives; re-pick to refresh after edits.

Wire shapes: `session_submit` (Tauri), `session/submit` (RPC),
and the `skills/get` response (`{ slug, title, description, body,
path, references }`) all share the same `path` field on the wire
so the UI can build pills + the daemon can build resource URIs
without a second lookup.

### Glossary

- **session** — the ACP wire session id (issued by the agent via
  `session/new`). Only meaningful inside `adapters::acp`.
- **instance** — our owner/record of a running agent process + its
  ACP session + its channels. Keyed by `InstanceKey` (a `Uuid`
  newtype). Outlives any single `session/new` cycle; multiple
  instances of the same `(agent, profile)` are supported by
  construction.
- **profile** — user-config bundle of agent + model + cwd + system
  prompt + mode. Drives `SpawnSpec` / `ResolvedInstance`.
- **mode** — per-instance operational mode (e.g. claude-code's
  `plan` / `edit`). Threaded through
  `SpawnSpec → ResolvedInstance → AcpInstance → InstanceInfo`;
  vendor-specific wire injection is the agent impl's concern.
- **agent** — the vendor process/binary (claude-code, codex, opencode).
- **adapter** — the transport trait (`adapters::Adapter`). ACP is one
  impl; HTTP-based agents will be another.
- **registry** — `AdapterRegistry<H: InstanceActor>` — the generic
  per-adapter instance map + insertion-order vec + focus pointer +
  event broadcast. Composed by each adapter facade.

## What is not in the scaffold

The following deliberately land in their own issues — do not bolt them onto
scaffold work:

- Persistent disk-backed trust store (today's runtime store is
  in-memory; "always" decisions reset on instance restart).
- Real branding icon — `src-tauri/icons/icon.png` is a generated 32×32
  placeholder.
- Release bundling (`bundle.active = false` in `tauri.conf.json`).
- CI / `.gitlab-ci.yml`.

## Upstream migration runway

Pending upstream moves that will drive a hyprpilot bump. Keep this list
accurate — whenever an upstream ships a tracked migration, follow the
linked checklist in the same commit that bumps the dep, and **delete the
row from this section when the work lands** so the runway always
reflects debt we still carry.

### wry / Tauri → GTK4 + webkit2gtk-6.0

| | Reference | Status |
| --- | --- | --- |
| Tracking issue | [`tauri-apps/wry#1474`](https://github.com/tauri-apps/wry/issues/1474) | open, prioritized, assigned to core maintainer |
| Active port PR | [`tauri-apps/wry#1530`](https://github.com/tauri-apps/wry/pull/1530) | open; unmerged |
| Current binding here | GTK3 via `gtk = "0.18"` / `gdk = "0.18"` / `gtk-layer-shell = "0.8" (v0_6)`, webview via `webkit2gtk` 4.1 |

Why it matters: the gtk-rs GTK3 crates are archived
(RUSTSEC-2024-0411..0420) and `glib < 0.2` carries a known unsoundness.
We inherit that exposure for as long as Tauri pins `wry 0.54.x`.

When wry#1530 merges and Tauri publishes a release consuming it,
migrate in one PR:

1. Bump `tauri` in `src-tauri/Cargo.toml`, enabling whatever feature the
   new wry exposes (likely `linux-webkit2gtk-6` or becomes the default).
2. Swap Linux-target deps: `gtk` → `gtk4`, `gdk` → `gdk4`,
   `gtk-layer-shell` → `gtk4-layer-shell`. Drop the `v0_6` feature
   (GTK4 binding exposes `KeyboardMode::OnDemand` natively).
3. Update `src-tauri/src/daemon/mod.rs::apply_anchor_mode`:
   - `use gtk::prelude::...` → `use gtk4::prelude::...`.
   - `use gtk_layer_shell::...` → `use gtk4_layer_shell::{..., LayerShell}`
     (the GTK4 crate exposes layer-shell methods via an extension trait,
     not inherent methods).
   - `gtk_window.show_all()` → `gtk_window.set_visible(true)` (GTK4
     dropped `show_all`; children auto-show).
   - `gtk_window.hide()` → `gtk_window.set_visible(false)`.
   - `gtk_window.present()` stays — it is the load-bearing commit that
     makes the compositor actually map the layer surface, verified
     against Hyprland 0.54.3 during K-235.
4. Revisit the Wayland env workaround in `src-tauri/src/main.rs`:
   `WEBKIT_DISABLE_DMABUF_RENDERER=1` is set unconditionally on Wayland
   to work around `Gdk-Message: Error 71` on NVIDIA + webkit2gtk 4.1.
   webkit2gtk 6.0 has had multiple DMABUF-path fixes; test with + without
   on the NVIDIA box and drop the workaround if 6.0 handles it cleanly.
5. Swap the system-library note in this file from `gtk-layer-shell` to
   `gtk4-layer-shell`. Both are packaged on Arch; no other OS-level
   friction expected.
6. Run the full verification path (see next section) and paste the
   `hyprctl layers` output pre- and post-bump into the PR description so
   the surface behavior is provably equivalent.

**Do not preempt upstream.** Vendoring wry's fork or cherry-picking
wry#1530 trades compile-time pain + merge conflicts for a feature that
is already prioritized. Wait for the release, follow the checklist.

### Other open debt worth tracking

- **Playwright `tauri` mode against WebKitGTK.** Scaffold lands with
  `@srsholmes/tauri-playwright 0.2` + `tauri-plugin-playwright 0.2`
  wired behind the `e2e-testing` Cargo feature, but the bridge's
  `webview.eval` callback stalls on `webkit2gtk-4.1` — every `eval` /
  `title` / `content` command hits the plugin's 30s timeout. E2E runs
  in `browser` mode (Vite + Chromium + IPC mocks) by default;
  `HYPRPILOT_E2E_MODE=tauri` flips over once the stall clears (likely
  resolves with the GTK4 + webkit2gtk-6.0 migration above, or with a
  WebdriverIO fallback — see `tests/e2e/README.md`).
- **Release bundling.** `tauri.conf.json` has `bundle.active = false`.
  Lifting it needs real icons and the pipelines issue (see below).
- **CI / `.gitlab-ci.yml`.** Not yet created; scaffold verifies locally.
  When it lands, every check listed in "Manual verification patterns"
  below should have a matching CI job.
- **Real branding icon.** `src-tauri/icons/icon.png` is a programmatic
  32×32 solid-fill placeholder.

## Manual verification patterns

`task test`, `task lint`, `task format` are the automated bar. Beyond
that, **every feature that changes runtime behavior lands with a manual
smoke-test block in its PR description** — concrete commands + literal
observed output so a reviewer can re-run against the branch and
compare. "Should pass" is not evidence; paste the actual response.

### Baseline smokes (extend per feature)

These cover the scaffold's surface and should stay green on every PR:

- `task install && task build` — produces `target/debug/hyprpilot`.
- `./target/debug/hyprpilot --help`, `... daemon --help`, `... ctl --help`
  render via clap.
- `./target/debug/hyprpilot daemon` opens a window and
  `ls $XDG_RUNTIME_DIR/hyprpilot.sock` confirms the socket is bound.
- Second `hyprpilot daemon` invocation exits `0` via
  `tauri-plugin-single-instance` without spawning a second window.
- `./target/debug/hyprpilot ctl <cmd>` round-trips through the JSON-RPC
  endpoint; daemon-not-running → exit 1, stderr
  `"hyprpilot daemon is not running"`.
- A deliberately broken `config.toml` (e.g. `logging.level = "verbose"`,
  `[ui.theme.window] default = "not-a-color"`,
  `[daemon.window.center] width = "200%"`, `[daemon.window.anchor]
  margin = -5`) aborts startup with a readable garde error naming the
  field path.
- Partial config overrides compose: e.g. setting only
  `[ui.theme.surface.card.user] bg = "#..."` keeps every sibling token
  falling through to `defaults.toml`.

### Layer-shell / anchor mode (K-235)

- `hyprctl layers` (or `swaymsg -t get_tree` on Sway) lists a layer with
  `namespace: hyprpilot` and the configured `xywh`.
- Flipping `[daemon.window.anchor] edge = "left"` via `--config` moves
  the surface without a rebuild.
- `[daemon.window] mode = "center"` yields a regular top-level — **no**
  entry under `hyprctl layers`, a client with `class: hyprpilot` shows
  up at the computed pixel size.
- Overriding `[daemon.window.anchor] margin = 20` shifts the surface by
  20 px from the anchored edge.

### JSON-RPC / ctl (K-236)

- `ctl toggle`, `ctl submit`, `ctl cancel`, `ctl session-info`,
  `ctl kill` all round-trip; stdout is the pretty-printed JSON `result`,
  exit 0 on success.
- Raw socket probes (via `socat`, `ncat`, or a short python
  `UnixStream`): a valid request returns a `result` envelope; `not json`
  returns `-32700` with `id: null`; missing `jsonrpc` field returns
  `-32600`; unknown method returns `-32601`.

### When a check needs a Wayland session

Most layer-shell / window checks require running on Hyprland or Sway.
Call that out in the PR's verification block so a non-Wayland reviewer
knows why it isn't reproducible from CI — and once the pipelines issue
lands, wire a Wayland-capable runner to re-assert the checks in
automation.

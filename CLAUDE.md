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
and exits `0` without opening a second window.

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

### Base font size — page zoom from GTK desktop font

The overlay inherits its base size from the user's GTK desktop font
(`gtk-font-name` via `gtk::Settings::default()`). Daemon queries it
once in `tauri::Builder::setup(...)` — GTK is initialized by then —
and applies the result via **`WebviewWindow::set_zoom(f64)`**, the
Chromium-style page zoom that wry forwards to WebKit's
`set_zoom_level`. This scales **text + layout together** (not just
font-size), avoiding the classic WebKitGTK font-scaling-factor bug
where fonts grow but margins/padding don't (WebKit bug 250138).

The mapping is linear with a 10pt baseline: `zoom = 1.0 + (size_pt -
10) * 0.1`, clamped `[0.5, 2.0]`. `Segoe UI 11` → 1.1×; `DejaVu Sans
10` → 1.0×; `Inter 12` → 1.2×. Missing settings singleton,
unparseable font string, or `set_zoom` failure → no zoom call;
webview stays at its 1.0× default.

`get_gtk_font` is still exposed so the webview can pick up the
**family** (not the size) — `useTheme::applyGtkFont()` overrides
`--theme-font-sans` with the user's GTK family so prose matches the
desktop; `--theme-font-mono` stays on the configured stack (code
deserves a monospace regardless of desktop font).

**CSS must not set `font-size` in `px`.** Every primitive uses `rem`
(or `em`). `text-[0.Nrem]` Tailwind arbitrary-value utility is the
canonical way to set a font size inside a primitive; full utility
aliases (`text-xs`, `text-sm`, …) are rem-based and fine. No literal
`font-size: Npx` anywhere under `ui/src/`. The `set_zoom` call is
the single scale authority; adding a second `html { font-size }`
write on top would double-count.

**Why not `WebKitSettings::default-font-size`?** That property only
scales the default font (unset CSS sizes); it doesn't touch explicit
`font-size: 1rem` declarations, AND it's the exact axis WebKitGTK
treats as "font-only, layout fixed" — same bug as manipulating
`html { font-size }`. Page zoom is the correct knob.

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
4. Kill both the Vite server and the browser process when done
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
- **Inline single-use helpers.** A function with exactly one caller should be
  folded into that caller. Prefer `fn main() -> Result<()>` over a `try_main`
  wrapper; prefer unfolding a small setup step into the body (with a short
  comment) over extracting a one-call helper.
- **Compose behavior onto the owning type, not as free fns.** When a
  module defines a primary type (`AcpClient`, `AcpSessions`,
  `StatusBroadcast`), helpers that operate on that type's state — or
  need to touch the channels / handles / registries it owns — go as
  methods on it, not module-level fns. Free fns are for pure
  transformations that don't read or mutate the type's invariants.
  Drove the K-240 refactor from free `forward_notification` /
  `auto_cancel_permission` + `emit_acp_event` into methods on
  `AcpClient` / `AcpSessions`; keeps the ownership graph legible and
  avoids passing owned state by parameter.
- **Small composable primitives live in `src-tauri/src/tools/`; domain
  modules host thin adapters over them.** A type that could exist
  without the domain it first appears in (a sandbox, a terminal
  registry, an fs-with-containment wrapper) belongs in `tools/`,
  returns a domain-specific error enum (`SandboxError`, `FsError`,
  `TerminalError`), and knows nothing about the protocol that called
  it. The domain module (`acp/client.rs`) becomes a translation layer
  — parse the wire envelope, delegate to the tool, map the tool's
  error into the protocol's error. Precedent: K-244 MR 21 review
  refactored `AcpClient` from owning `sandbox_root` / `terminals`
  directly into `{ events, fs: Arc<FsTools>, terminals: Arc<Terminals> }`;
  `FsTools` / `Terminals` / `Sandbox` moved under `tools/`, only the
  ACP error-mapping stayed inside `acp/`. Every non-ACP entry point
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
  struct.
- **Closed sets use `enum`, not union string literals.** Define
  `export enum SessionState { Starting = 'starting', … }` and type
  fields as `state: SessionState`. Union string literals
  (`state: 'starting' | 'running' | …`) are banned — the enum is the
  single source of names the TS compiler can refactor. Same rule for
  discriminator tags (`kind: EventKind.Transcript`).
- **Named types with `T[]` suffix for arrays.** Extract every inline
  object-array type (`Array<{ option_id: string, … }>`) to a named
  interface, then use `PermissionOptionView[]` — not `Array<T>`, not
  inline. One named type per wire shape.

### Naming conventions

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
    `AcpAgent` sits next to `AcpSession`, `AcpSessions`,
    `AcpSessionHandle` because they share the ACP wire protocol.
    That's a grouping, not a universal prefix mandate — `Acp*` only
    makes sense for things that actually speak ACP. Same pattern
    would apply to a future direct-HTTP sibling: `HttpAgent` +
    `HttpSession` + `HttpSessionHandle`.
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
- **Custom animations go in `@theme {}` Tailwind blocks** (`--animate-<name>:
  <keyframes>`), not `:root { --<name>-shorthand: ... }`. Consumers reach
  them via the Tailwind utility class (`animate-<name>`), not raw
  `animation:` declarations. Today the tree ships `animate-pulse-slow` (the
  tool-running pulse) and `animate-blink` (palette / terminal caret).
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
| `session/submit` | `{ "text": "...", "agent_id"?: "...", "profile_id"?: "..." }` | `{ "accepted": true, "agent_id": "...", "profile_id": "..." \| null, "session_id": "..." \| null }` | Resolves `(agent_id, profile_id)` via `acp::resolve`; distinct profiles spawn distinct actors even against the same agent. |
| `session/cancel` | *(none)* or `{ "agent_id"?: "..." }` | `{ "cancelled": bool, "reason"?: "..." }` | Sends `CancelNotification` to the addressed session. |
| `session/info` | *(none)* | `{ "sessions": [...] }` | Live session list across every active agent + profile. |
| `window/toggle` | *(none)* | `{ "visible": bool }` | Flips main window visibility; updates `StatusBroadcast` visible bit. |
| `daemon/kill` | *(none)* | `{ "exiting": true }` | Calls `app.exit(0)` after write + flush. Delivery is best-effort: the process may tear down before the peer finishes reading. |
| `status/get` | *(none)* | `StatusResult` | One-shot status snapshot. |
| `status/subscribe` | *(none)* | `StatusResult` (initial) | Registers connection as subscriber; server pushes `status/changed` notifications. |
| `status/changed` | `StatusResult` | *(notification, no id)* | Server-push on every state transition. Clients receive this after `status/subscribe`. |
| `config/profiles` | *(none)* | `{ "profiles": [{ id, agent, model, has_prompt, is_default }] }` | Read-only profile list for the chat-shell picker (K-246). |

`StatusResult` shape: `{ "state": "idle" | "streaming" | "awaiting" | "error", "visible": bool, "active_session": string | null }`.

**Namespace convention.** Every method name on the wire uses the
`namespace/name` form, matching ACP's own methods (`session/prompt`,
`session/new`):

- `session/*` — anything scoped to an agent session (prompts, cancel, info).
- `window/*` — overlay window state (`window/toggle`; future
  `window/show`, `window/hide`, `window/focus`).
- `daemon/*` — daemon lifecycle (`daemon/kill`; future `daemon/status`,
  `daemon/reload`).
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
today — those go through `AcpSessions` on the server side, which
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

### Module layout (`src-tauri/src/acp/`)

- `agents/{mod,claude_code,codex,opencode}.rs` — `AcpAgent` trait +
  three vendor unit structs. Each carries no runtime state; vendor
  quirks (launch command, model flag, system-prompt injection site)
  live in trait-method bodies. `match_provider_agent(provider)`
  resolves a `Box<dyn AcpAgent>` off the closed `AgentProvider` enum.
- `resolve.rs` — `resolve(&Config, profile_id?) -> ResolvedSession`.
  Flattens the layered config into `{ agent, profile_id, model,
  system_prompt }`. Model precedence is profile > agent > vendor
  default; `system_prompt_file` contents are read from disk here so
  a missing file fails on submit, not inside the actor.
- `spawn.rs` — `spawn_agent(&AgentConfig, system_prompt: Option<&str>)
  -> (Child, ChildStdio)`. Wraps `AcpAgent::spawn` +
  `AcpAgent::inject_system_prompt` and captures stdin/stdout for the
  ACP connection.
- `client.rs` — `on_receive_*` shims the ACP `Client.builder()` takes.
  `SessionNotification`s fan out onto a per-session mpsc; every
  `RequestPermissionRequest` auto-`Cancelled` until
  `PermissionController` lands (the webview still sees an
  `acp:permission-request` event for observability).
- `runtime.rs` — one `tokio::spawn`ed actor per session. Takes a
  `ResolvedSession` and drives `initialize` → `session/new` →
  `session/prompt` for the first prompt, then loops on an mpsc of
  `SessionCommand::{Prompt, Cancel, Shutdown}`. Broadcasts
  `SessionEvent` upstream.
- `session.rs` — `AcpSessions` (Tauri managed state). Keyed
  `HashMap<(agent_id, profile_id?), SessionHandle>` so distinct
  profiles against the same agent get distinct actors. The RPC
  surface + Tauri commands both route through it.
- `commands.rs` — Tauri `#[command]`s: `acp_submit`, `acp_cancel`,
  `agents_list`, `profiles_list`, `session_list`, `session_load`,
  `permission_reply` (unimplemented stub until K-6).

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
sessions)` is the one orchestrator; it drains ACP sessions via
`AcpSessions::shutdown`, then calls `app.exit(0)` (which closes
webviews, drops every `app.manage(...)` value — flushing the tracing
`WorkerGuard`, the `StatusBroadcast`, and the socket listener — and
exits with code 0).

Three call sites funnel through this one fn:

1. **`daemon/kill` RPC** — `DaemonHandler` returns
   `{"killed": true}` in the result; `rpc::server::handle_connection`
   inspects the payload after the flush and calls
   `daemon::shutdown`. No side-channel flag threaded through the
   dispatcher tuple — the marker is the response itself, so any
   future respond-then-shut-down handler just emits the same
   `{"killed": true}` shape.
2. **SIGINT (Ctrl-C)** — tokio signal task spawned in `daemon::run`.
3. **SIGTERM** — same task; systemd / `pkill` both use this.

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

Client-side auto-accept / auto-reject rules (per-tool allowlists,
persistent trust store) are the scope of a separate future
`PermissionController` issue, modeled on the original Python pilot's
`auto_accept_tools` / `auto_reject_tools` configuration rather than
a coarse policy enum. Until that lands every prompt is live-UI.

### Tauri commands + events (live)

| Command | Purpose |
| ------- | ------- |
| `acp_submit { text, agent_id?, profile_id? }` | Webview compose-box submit. Delegates to `AcpSessions::submit`. |
| `acp_cancel { agent_id? }` | Mid-turn cancel. Sends `CancelNotification` to the addressed session. |
| `permission_reply { session_id, request_id, option_id }` | `unimplemented!` until `PermissionController` (K-6) lands — the runtime auto-`Cancelled`s every permission request today, so the webview never reaches this path. |
| `agents_list` | Populates the agent-switcher dropdown from the `[[agents]]` registry. |
| `profiles_list` | Populates the profile picker from `[[profiles]]`; parallels the `config/profiles` wire method. |
| `session_list { agent_id, profile_id?, cwd? }` | Calls ACP `session/list` through the live `(agent_id, profile_id)` adapter; spawns an ephemeral `Bootstrap::ListOnly` actor when none is live (initialize → list → shutdown, never registered). Returns the raw ACP `ListSessionsResponse` — agent owns storage, hyprpilot just passes through. |
| `session_load { agent_id, profile_id?, session_id }` | Tears down any live handle for `(agent_id, profile_id)`, then starts a fresh actor with `Bootstrap::Resume(session_id)`. Gated on `InitializeResponse.agent_capabilities.load_session` — vendors that don't advertise resume get a `-32601`-shaped error. Replay streams through the normal `acp:transcript` fanout. |

| Event | Payload | When |
| ----- | ------- | ---- |
| `acp:transcript` | `{ agent_id, session_id, update }` | Every `SessionNotification` the agent streams; `update` is the raw `SessionUpdate` JSON. |
| `acp:permission-request` | `{ agent_id, session_id, options }` | Every `session/request_permission` — auto-denied but surfaced for observability. |
| `acp:session-state` | `{ agent_id, session_id, state }` | On every lifecycle transition (`starting` / `running` / `ended` / `error`). |

Event names use `:` (Tauri-side convention); the JSON-RPC wire keeps
`/` (`session/submit` etc.); config uses `.`; CSS uses `-`.

## What is not in the scaffold

The following deliberately land in their own issues — do not bolt them onto
scaffold work:

- MCP server(s), skills loader, permissions store, markdown
  rendering, profile switcher UI.
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

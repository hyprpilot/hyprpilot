# CLAUDE.md

Agent operating manual for `hyprpilot`. Read this first; the Linear project
description is the authoritative design snapshot.

## Project overview

- Single Rust binary (`hyprpilot`) that doubles as a Tauri 2 overlay daemon and
  a unix-socket CLI client, selected via subcommand (`daemon` / `ctl`).
- Frontend: Vue 3 + Vite + Tailwind v4 + shadcn-vue + reka-ui under `ui/`.
- Backend: Rust crate at `src-tauri/` with `clap`-derive subcommand dispatch,
  `tauri-plugin-single-instance`, and a tokio `UnixListener` at
  `$XDG_RUNTIME_DIR/hyprpilot.sock`.
- Config: layered TOML — compiled defaults → `$XDG_CONFIG_HOME/hyprpilot/config.toml`
  → per-profile TOML → clap flags. The full UI theme is part of this config.

## Toolchain (mise-pinned)

`mise install` at the repo root drops: `rust` (stable + `rustfmt` +
`clippy`), `node` 24, `pnpm` 10, `task` 3 (go-task), `usage` 3,
`cargo-nextest` (`task test` drives the Rust suite through it; `cargo
test` still works for doc-tests). `rust-toolchain.toml` pins toolchain
for `cargo` invocations outside mise.

## Tasks

Every `task` target orchestrates both Rust and the frontend where applicable.
Exactly the targets listed below exist — no others should be added without
updating this file.

| Task | Purpose |
| ---- | ------- |
| `task install` | `cargo fetch` + `pnpm install` at the workspace root (installs `ui`, `tests/e2e`, `tests/e2e/support/mock-agent`). |
| `task dev` | `./node_modules/.bin/tauri dev` — full dev cycle with Vite + Tauri. |
| `task test` | `task test:ui` + `cargo nextest run --all-targets`. E2E stays out of the inner loop. |
| `task test:ui` | `pnpm --filter hyprpilot-ui test` — Vitest over every colocated `src/**/*.test.ts`. |
| `task test:e2e` | overlay-build → `pnpm --filter hyprpilot-ui build` → `pnpm --filter hyprpilot-e2e test`. Browser mode by default; `HYPRPILOT_E2E_MODE=tauri` for the Playwright bridge path. |
| `task format` | `cargo fmt --all` + `pnpm --filter hyprpilot-ui format`. |
| `task lint` | `cargo fmt -- --check` + `cargo clippy --all-targets -- -D warnings` + eslint + `vue-tsc --noEmit`. |
| `task build` | Debug build via `./node_modules/.bin/tauri build --debug`. |
| `task "build:release"` | Release build via `./node_modules/.bin/tauri build`. |

### Verifying UI changes — use named scripts, never `pnpm exec`

**Rule**: always run UI lint / type-check / build through **named pnpm
scripts** or **task targets**, never via `pnpm exec` or `pnpm --filter
<pkg> exec <binary>`. **Why**: the recursive-exec path silently exits
`0` with `Command "<binary>" not found` when the workspace root has no
copy of the binary in its `.bin/`, hiding real errors.

Canonical commands: `pnpm --filter hyprpilot-ui run type-check`,
`pnpm --filter hyprpilot-ui run lint`,
`pnpm --filter hyprpilot-ui run build`,
`pnpm --filter hyprpilot-ui test`. From inside `ui/`, the same scripts
without the filter (`pnpm run lint`, etc.).

**Pre-push verification — run all three.** `task build` covers compile
+ type-check, `task test` covers test suites, `task lint` covers `cargo
fmt --check`, `cargo clippy -- -D warnings`, **eslint**, and `vue-tsc
--noEmit`. **`task build` exiting 0 is NOT sufficient** — eslint runs
ONLY through `task lint`, and CI's lint job rejects on stylistic-rule
violations (`object-curly-newline`,
`padding-line-between-statements`, etc.) that the build step is blind
to. The bar is `task build && task lint && task test` exits 0; CI runs
the three as separate jobs and any one red rejects.

**Autopilot CI watch.** When operating in autopilot, after `git push`
+ `gh pr create` the agent stays attached until the latest workflow
run on the head ref reports `conclusion = success` for every check.
Poll with `gh pr checks <pr>` (or `gh run list --branch <branch>
--limit 1 --json status,conclusion`); when a job fails, fetch the
failed step's log via `gh run view <id> --log-failed`, fix
root-cause, push the fix to the same branch, wait for re-run. The PR
is "done" only when CI is fully green — the captain shouldn't have to
catch CI failures.

## Running the binary locally

```sh
# long-lived Tauri + socket
./target/release/hyprpilot                   # shorthand for `hyprpilot daemon`
./target/release/hyprpilot daemon
./target/release/hyprpilot daemon --cwd ~/projects/foo  # chdir before any setup

# CLI client
./target/release/hyprpilot ctl submit "hello there"
./target/release/hyprpilot ctl toggle
./target/release/hyprpilot ctl --help

# Status (one-shot snapshot; exits 0 even if daemon is down)
./target/release/hyprpilot ctl status

# Status (long-running stream for waybar; reconnects with back-off on socket loss)
./target/release/hyprpilot ctl status --watch
```

Second `hyprpilot daemon` forwards argv through `tauri-plugin-single-instance`
and exits `0` without opening a second window. When the second invocation
carries no subcommand (bare `hyprpilot` or `hyprpilot daemon`) the
single-instance callback also routes through `daemon::tray::present` —
the CLI escape hatch for popping the overlay when no Hyprland keybind is
bound. `hyprpilot ctl …` invocations stay out of this path.

The daemon boots **hidden by default** (`[daemon.window] visible = false`).
First user-visible map happens via a Hyprland keybind (`overlay/present`),
the system tray icon, or the bare `hyprpilot` escape hatch above. Set
`visible = true` to glue the overlay on at boot. See `docs/autostart.md`
for the autostart story.

### Waybar integration

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
   when `--config-profile <name>` / `HYPRPILOT_CONFIG_PROFILE` is supplied.
   This is the config-layering alias, distinct from the session
   `[[profiles]]` registry (addressed per-call via `ctl submit --profile <id>`).
4. `clap` flags — override-per-invocation, never persisted.

**Rule**: `defaults.toml` is the **single source of truth** for default values.
Rust code consuming config leaves uses
`.expect("... seeded by defaults.toml")` rather than duplicating defaults
as `unwrap_or(...)` fallbacks. **Why**: a paired test pins every
`.expect()`-ed leaf to a seeded TOML field, so a missing default fails the
test before it ships a runtime panic.

`Config::validate()` runs after merge and fails startup with a readable error
on invalid values. `deny_unknown_fields` on every section catches typos in
user TOML at load time.

### Merge trait

Layer application goes through a `pub(crate) trait Merge { fn merge(self,
other: Self) -> Self; }` in `config/mod.rs`. `other` wins; `load()`'s fold
reads `acc.merge(layer)`. A blanket `impl<T> Merge for Option<T>` handles
every scalar leaf; each struct in the config tree carries a trivial
field-by-field impl. Keyed-list merges (override by `id`, append new ids) are
the documented exception per collection.

### Validation strategy (garde)

**Rule**: Per-type invariants live on the type itself — not as free
`validate_*` functions — whenever the orphan rule allows.

- **Types we own**: `impl garde::Validate for T` + `#[garde(dive)]` at the
  field site.
- **String-backed closed sets**: convert to a `#[derive(Deserialize)]` enum
  with `#[serde(rename_all = "lowercase")]`. **Why**: unknown values reject
  at TOML parse time instead of at `validate()`, which is stricter.
- **Cross-field references**: higher-order `custom(fn(&self.sibling))` hooks
  (garde's "self access in rules").
- **Collection-level checks**: free fn + `#[garde(custom(fn))]` on the field
  (the orphan rule blocks `impl Validate for Vec<T>` and a newtype would
  force consumers through `.0`).

`Config::validate()` is a one-liner that wraps the garde report in
`anyhow!` — every rule is inside the derive walk.

### `HexColor` newtype

Theme colour fields are `Option<HexColor>`, not `Option<String>`.
`#[serde(transparent)]` keeps the wire shape a bare string; `impl Validate`
enforces `#[0-9a-fA-F]{6,8}` under `#[garde(dive)]`. `impl Deref<Target = str>`
+ `AsRef<str>` + `From<&str>` / `From<String>` keep consumer ergonomics
unchanged.

### Skills — multi-root catalogue, per-instance registries

`[skills] dirs: Vec<PathBuf>` lists roots the loader scans. Each root is
a flat directory of `<slug>/SKILL.md` bundles. Defaults seed
`["~/.config/hyprpilot/skills"]`; `~` / env vars expand at consume time.
User override **replaces wholesale**; `dirs = []` is the explicit "no
skills" override. **First-root-wins** on slug collision (warn + skip
later duplicates). Missing roots warn + skip; no auto-mkdir.

The skills registry is **per-instance**, not daemon-global. Each spawned
`AcpInstance` owns an `Arc<SkillsRegistry>` built once at spawn time
from the active profile's `skills = [...]` (with profile→global
fallback). **Wholesale-replace semantics** (mirrors `mcps`): profile's
`skills = [...]` wins over the global; `skills = []` is the explicit
off-switch; unset inherits the global. **Why per-instance, not a
filtered global view**: profiles aren't required to share roots; a
union-with-filter view can't represent "different profiles, no shared
dirs" cleanly.

**Reload paths**: `skills/reload { instance_id? }` Tauri command (the
palette's refresh entry) and `daemon/reload` JSON-RPC method (fans out
to every live instance). **No fs watcher** — explicit reload only;
editor / git noise burns through any debouncer.

Skill delivery flows exclusively through the palette. Picked skills
attach to the user turn as `UserTurnInput::Prompt { text, attachments }`.

### `mcps` — JSON-file MCP catalog

`mcps: Option<Vec<PathBuf>>` at the TOML root lists JSON paths. Each
path follows the standard `mcpServers` shape used by Claude Code / Codex
/ Cursor. hyprpilot extends each server entry via an optional
`hyprpilot` namespace key:

```json
{
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
      "hyprpilot": {
        "autoAcceptTools": ["read_*"],
        "autoRejectTools": ["delete_*"]
      }
    }
  }
}
```

- **Merge semantics**: files iterate in order, map collisions → later
  wins. The `hyprpilot` block is typed; everything else stays as opaque
  `serde_json::Value`.
- **Per-profile override**: `[[profiles]] mcps = [...]` wholesale-replaces
  the global default. `mcps = []` is the explicit "no MCPs" off-switch.
- **ACP injection**: each `session/new` and `session/load` carries the
  resolved set as `mcp_servers`. Stdio / HTTP / SSE project onto the
  typed ACP `McpServer` enum.
- **Permission integration**: `hyprpilot.autoAcceptTools` /
  `autoRejectTools` matched at `PermissionController::decide` lane 2.
  Globs are **server-relative** — write `read_*` inside the server
  block, the `mcp__<server>__` prefix is implicit.
- **No reload**: catalog is static after daemon boot.
  Restart-to-reconfigure.

## Theming

**The palette lives in Rust, not CSS.** Flow:

1. `defaults.toml` seeds every theme token under `[ui.theme.*]`.
2. User TOMLs override any subset; the merge trait walks the tree
   field-by-field over `Option` leaves.
3. `config::Theme` is a typed tree.
4. The Tauri `get_theme` command serves the resolved tree to the webview.
5. `ui/src/composables/useTheme.ts::applyTheme` walks the object and writes
   every scalar leaf onto `:root` as a `--theme-<path>` CSS custom property.
   `main.ts` awaits it before `createApp(App).mount('#app')` so the first
   render already has the palette.

**Theme groups** (under `[ui.theme.*]`): `font`, `window`, `surface`,
`fg`, `border`, `accent`, `state` (five-phase live indicator), `kind`
(per-tool-family dispatch colours keyed by `ToolCall.kind`), `status`
(toast / banner hues), `permission`. See `defaults.toml` for the
authoritative leaf list.

**CSS variable naming rule** (implemented in `cssVarName`):

- Path segments named `default` or `bg` drop from the emitted variable name.
- Remaining segments join with `-`; snake_case fields become kebab-case.
- Examples: `fg.default` → `--theme-fg`; `surface.card.user.bg` →
  `--theme-surface-card-user`.

**Rules:**

- Add a new group by adding a `ThemeXxx` struct (standard config-struct
  derives), wiring it into `Theme`, seeding values in `defaults.toml`,
  and updating the two token tests. Add a Tailwind utility alias in
  `ui/src/assets/styles.css::@theme inline` when a new token needs
  utility-class access.
- **CSS must not declare literal theme values anywhere** — not on
  `:root`, not as `var(--token, literal)` fallbacks, not inline in
  `.vue` scoped styles. Rust is the sole source. The Tauri window's
  native `backgroundColor` (in `tauri.conf.json`) is painted before the
  webview loads; keep it equal to `[ui.theme.window] default`.
- **Do not introduce new `--pilot-*` vars.** All theme tokens are
  `--theme-*`.
- Cards are keyed by speaker, not elevation: `surface.card.user`,
  `surface.card.assistant`. Do not name surfaces `card_hi`, `card_alt`.

### UI scaling — rem-based layout + `[ui] zoom`

**Rule**: every layout primitive resolves through `rem`. Typography,
paddings, widths, gaps, border-radius, min-heights — everything that should
track UI scale is written in `rem` (or via Tailwind utilities, which
compile down to `rem`). `:root { font-size: 16px }` is the rem anchor.
Reserve literal `px` for **hairlines** (1px borders) and viewport
media-query thresholds.

Two scaling axes compose on top of that rem-based tree:

1. **`[ui] zoom`** (default `1.0`, range `[0.5, 2.0]`). The daemon calls
   `WebviewWindow::set_zoom(zoom)` after window-map — Chromium-style page
   zoom that scales text + layout uniformly.
2. **Mobile / browser baseline.** When the SPA is served over HTTPS to a
   phone or remote browser, `set_zoom` doesn't run — there's no Tauri
   webview. Phones get the rem tree at the browser's default `16px` root.

`get_gtk_font` is exposed so the webview picks up the **family** (not the
size) on Linux — `useTheme::applyGtkFont()` overrides `--theme-font-sans`
with the user's GTK family. `--theme-font-mono` stays on the configured
stack.

Use rem (or Tailwind utilities) for new layout / typography. `text-[0.Nrem]`
is the escape hatch for arbitrary font sizes.

## Window surface (`[daemon.window]`)

The daemon's main window runs in one of two modes:

- `anchor` (default) — a `zwlr_layer_shell_v1` surface pinned to a
  configurable edge, painted above normal windows. Requires the compositor
  to implement `zwlr_layer_shell_v1` — does **not** work on GNOME Shell or
  KDE Plasma.
- `center` — a regular Tauri top-level sized as a percentage of the active
  monitor and centered. Works on any compositor.

Two knobs are intentionally **not exposed**:

- `layer = overlay` — always paints above normal and fullscreen windows.
- `keyboard_interactivity = on_demand` — compose input needs focus, but the
  overlay must not grab keys while idle.

### Config shape

```toml
[daemon.window]
mode = "anchor"        # "anchor" | "center"
output = "DP-1"        # optional; defaults to primary monitor
visible = false        # boot with the surface unmapped (default)

[daemon.window.anchor]
edge = "right"         # "top" | "right" | "bottom" | "left"
margin = 0             # px from the anchored edge
width = "40%"          # "N%" (of monitor) or pixel int; default 40%
# height unset         # unset → full-height fill via top+bottom anchor

[daemon.window.center]
width = "50%"          # "N%" (of monitor) or pixel int
height = "60%"
```

`width` / `height` accept either a pixel integer or an `"N%"` string;
`Dimension::{Pixels(u32), Percent(u8)}`. Percentages resolve against the
active monitor on every show transition, not just at boot. The full
`[daemon.window]` config is owned by the `WindowRenderer` struct
(`daemon/renderer.rs`); its `show()` method is the single code path for
both setup and toggle.

`[daemon.window.anchor] height` unset pins top + bottom + `edge`, so the
compositor stretches the surface full-height. Setting an explicit `height`
pins only `edge` and uses that fixed extent.

### Edge accent

The daemon exposes `get_window_state` → `{ mode, anchorEdge }`.
`useWindow.ts::applyWindowState` writes `data-window-anchor="<edge>"` on
`<html>` in anchor mode. CSS paints `var(--theme-window-edge)`:

- **Anchor mode**: a single 2px stripe on the side *opposite* the anchored
  edge. The anchored edge stays borderless (flush against the screen bezel).
- **Center mode**: full 2px perimeter via `html:not([data-window-anchor]) body`.

A single `body { box-sizing: border-box; }` rule keeps the painted border
inside the `100vh` viewport.

### Monitor selection — `WindowManager` adapter

Monitor picking lives behind a `WindowManager` trait in
`src-tauri/src/daemon/wm.rs` (`focused_monitor(monitors) ->
Option<MonitorInfo>`). `MonitorInfo.name` is the connector name and the
only identifier matched against `Monitor::name()` or `[daemon.window]
output`; EDID metadata is for log lines, never load-bearing.

Three concrete adapters detected via env markers:

| Adapter | Selected when | Source |
| -- | -- | -- |
| `WindowManagerHyprland` | `HYPRLAND_INSTANCE_SIGNATURE` set | `hyprctl -j monitors` |
| `WindowManagerSway` | `SWAYSOCK` set (Hyprland not) | `swaymsg -t get_outputs -r` |
| `WindowManagerGtk` | everything else | `gdk::Seat::pointer().position()` bounds-check |

**Resolution order in `WindowRenderer::resolve_monitor`:** explicit
`[daemon.window] output` → `wm.focused_monitor` → `primary_monitor()`
fallback → any monitor (safety net).

The layer-shell surface is always pinned to the resolved monitor via
`gtk_window.set_monitor(&gdk_monitor)`. `gdk_monitor_for(&Monitor)`
matches by geometry (gdk 0.18 has no `connector()` accessor); collapses
to a direct connector compare when GTK4 lands.

### Crate: `gtk-layer-shell` 0.8 (GTK3)

Tauri 2.10 still links `webkit2gtk` 4.1 (GTK3 binding), so `gtk-layer-shell`
with the `v0_6` feature for `set_keyboard_mode`. Layer-shell init runs
inside the Tauri `.setup(...)` closure — `init_layer_shell` must be called
before the window is realized. To satisfy that invariant the main window is
declared `visible = false` in `tauri.conf.json`; `apply_anchor_mode` then
configures the layer surface and maps the GTK window via
`gtk_window.show_all()`. **Do not** switch to `WebviewWindow::show()` —
some wlroots builds re-map through xdg-shell and silently drop the
layer-shell role.

## Logging

`tracing` is bootstrapped via `logging::init`. **Always writes to
stderr** — both debug and release. ANSI colours stay on in debug, off
in release. Every event tags `file:line` + module target. Surfaces:
`journalctl --user -u hyprpilot.service -f` (systemd unit) or direct
stderr (`RUST_LOG=… ./target/release/hyprpilot daemon`). Filter
precedence: `--log-level` → `RUST_LOG` → `info` fallback.

### Diagnostic trace targets

Targeted `trace`-level emissions for hard-to-reproduce wire bugs, silent
by default. Lifecycle-only and chunk variants are split so streaming
chunk traffic (~30 lines/sec) doesn't drown the lifecycle stream.

| Target family | Lifecycle | Chunk (opt-in) |
| --- | --- | --- |
| ACP wire payloads | `acp::wire`, `acp::thought` | — |
| Per-instance actor broadcast | `acp::emit` | `acp::emit::chunk` |
| Tauri bridge → webview | `tauri::emit` | `tauri::emit::chunk` |
| Daemon-side mirror cache | `snapshot::mirror` | `snapshot::mirror::chunk` |
| Snapshot RPC responses | `snapshot::meta`, `snapshot::chat`, `snapshot::terminals` | — |

Example for the thinking-block path:

```sh
RUST_LOG='info,acp::wire=trace,acp::thought=trace,webview=trace' \
  hyprpilot daemon
```

The `webview=trace` directive is load-bearing for UI-side
`log.trace(...)` — `tauri-plugin-log` builds records with
`target: "webview"`.

UI-side counterparts run through structured `log.trace(...)`. Search
prefixes: `snapshot.brim-sync.*`, `snapshot.focus-prefetch.*`,
`snapshot.live-patch.*`, `snapshot.fetch-older.*`,
`snapshot.page-trim.evicted`, `snapshot.hydrate.*`.

## Frontend testing

Two tiers, two locations — one convention per tier.

| Tier | Runner | Location | File suffix |
| -- | -- | -- | -- |
| Component / composable / lib | Vitest + `@vue/test-utils` + jsdom | beside the source | `<PascalOrCamel>.test.ts` |
| End-to-end | Playwright via `@srsholmes/tauri-playwright` | `tests/e2e/specs/` | kebab-case `.spec.ts` |

Component / composable tests sit colocated with the source
(`PermissionPrompt.vue` + `PermissionPrompt.test.ts`); e2e specs live
under `tests/e2e/specs/` with the harness fixtures in
`tests/e2e/fixtures/` and the scripted ACP mock under
`tests/e2e/support/mock-agent/`.

**Rule**: component tests mock Tauri IPC by replacing the `@ipc` barrel
with `vi.mock('@ipc', ...)`. **Never** monkey-patch `window.__TAURI__`.

E2E specs today run in `browser` mode against a Vite dev server with IPC
mocks. The daemon-spawning `tauri` mode is wired and gated behind
`HYPRPILOT_E2E_MODE=tauri`; see `tests/e2e/README.md` for the WebKitGTK-4.1
eval-stall that keeps it off the default lane.

### Playwright MCP for interactive UI debugging

The Playwright MCP server (`mcp__mcphub__playwright__*`) drives
Chromium, not the Tauri WebKit webview. Use for one-off layout
debugging:

1. Start the Vite dev server in the background:
   `pnpm --filter hyprpilot-ui dev` (typically `http://localhost:1420/`).
2. `browser_navigate` → `browser_evaluate` for computed-style inspection.
3. **Screenshot output goes to `.playwright-mcp/`** (gitignored). Always
   pass `filename: ".playwright-mcp/<name>.png"`.
4. Kill the Vite + browser processes when done.

**Caveats:**

- Browser mode in MCP can hang during launch in sandboxed shells. Fall
  back to static checks if `browser_navigate` times out.
- IPC-dependent UI paths surface the "tauri host missing" soft-fail.
- **Do not** use Playwright MCP for scripted regression tests — those
  belong in `tests/e2e/`.

The dev preview pulls a non-Tauri theme + window-state shim from
`tests/dev-preview` (env-gated by `VITE_HYPRPILOT_DEV_PREVIEW=1`).
Production builds tree-shake it out.

#### Hybrid daemon-driven verification

When debugging a wire-flow bug ("is the daemon emitting / handling X
correctly?") the WebKitGTK eval-stall is not a blocker — the
native-webview screenshot path stalls; everything else works.

**Always run wire verification through the Playwright e2e harness.** The
harness owns the daemon's lifecycle (`tests/e2e/fixtures/global-setup.ts`
spawn + socket-wait, `global-teardown.ts` SIGTERM).

1. **Run the spec via `task test:e2e:live`** (sets `HYPRPILOT_E2E_MODE=tauri`
   + a live config fixture).
2. **Drive the wire via `ctl`** from inside the spec
   (`./target/debug/hyprpilot ctl instances spawn …`,
   `ctl prompts send …`).
3. **Trace the emit path** via `HYPRPILOT_LOG_LEVEL=trace`.
   `global-setup.ts` routes daemon stdout/stderr into
   `${runtimeDir}/daemon.log`; specs assert via
   `expect(log).toContain('acp:instance-meta')`.
4. **Pair with Playwright MCP (browser mode)** for the visual layer. The
   daemon log proves the wire shape; the Playwright-MCP screenshot proves
   the chrome renders it.

**Force this loop whenever a chip / header / row "doesn't update".** The
dev-preview shim alone can fake any state but it lies; running the e2e
harness + reading its daemon log is the only way to know whether a Rust
mapper is dropping a wire variant or a new ACP enum has no Tauri bridge.

## Rust conventions

- **Enums whenever feasible — never `String` for a closed set.** This is
  the load-bearing convention behind half the others. If a value can
  only be one of N known things at compile time, it's a `#[derive(...)]
  enum` with `#[serde(rename_all = "...")]` for wire types, NOT a
  `String` / `&str` / `Cow<'_, str>`. Applies to: wire-protocol fields
  (agent state, stop reasons, update kinds, tool-call statuses,
  permission outcomes), config closed sets (window mode, anchor edge,
  agent provider, log level, dimension flavour), dispatch keys (RPC
  namespace, Tauri command name, palette leaf id), tone discriminators
  (toast tone, status colour, phase), bootstrap variants (Fresh /
  Resume), and every `match` over "what kind is this?". **Why**: the
  compiler enforces exhaustiveness at every match site, unknown values
  reject at TOML parse / serde deserialize time instead of slipping
  through `validate()`, refactor renames are mechanical, and IDE
  autocomplete tells the next reader the full set. A `String` field
  hides those affordances. The free-form `String` is reserved for
  user-supplied content (titles, paths, prompts) — values not bounded
  by the protocol.
- **No backwards-compatibility layers — ever.** The CLI, the unix-socket
  wire protocol, the config file, and the theme tree all evolve in
  lockstep with the daemon binary. When a design stops making sense,
  **delete it and rewire the call sites**; do not leave typed-shim
  enums, deprecated method aliases, or "legacy" wrappers.
- **Stubs panic, they don't pretend.** When a feature isn't wired
  end-to-end, the client-side entry point must `unimplemented!("<verb>:
  <why>")` rather than round-trip to the server and pretty-print a
  placeholder. **Why**: a fake-success JSON looks exactly like success
  and hides the gap.
- **Never fabricate static UI text.** Every visible string must read
  from a real signal. If the data isn't wired, omit the element
  entirely until it can be backed by data. Fabricated copy is a
  runtime lie.
- **Inline single-use helpers.** A function with exactly one caller
  should be folded into that caller. Prefer `fn main() -> Result<()>`
  over a `try_main` wrapper.
- **Compose behavior onto the owning type, not as free fns.** When a
  module defines a primary type, helpers that operate on its state — or
  need to touch the channels / handles / registries it owns — go as
  methods, not module-level fns. Free fns are for pure transformations.
- **Small composable primitives live in `src-tauri/src/tools/`; domain
  modules host thin adapters over them.** A type that could exist
  without the domain it first appears in (a sandbox, a terminal
  registry, an fs-with-containment wrapper) belongs in `tools/`,
  returns a domain-specific error enum, and knows nothing about the
  protocol that called it. The domain module becomes a translation
  layer.
- **Structs carry their invariants; don't re-pass context on every
  call.** When a helper needs the same configuration value on every
  invocation, wrap it in a struct and make the helper a method.
  `Sandbox { root: PathBuf }::new(root)` canonicalises once at
  construction; runtime errors that were only possible because the
  first arg was untrusted collapse into construction-time errors.
- **Prefer enum + match dispatch for similar handlers; reach for macros
  only when monomorphisation forces per-handler registration.** The
  first choice for a family of related operations is a closed enum
  variant + a match in the dispatcher. **Why**: one enum = one
  exhaustive match, compiler enforces coverage.
- **Traits for open extension points; closed enums for closed sets.**
  Traits pay their way when new implementers arrive from outside the
  decision (`WindowManager` for compositors, `AcpAgent` for vendors).
  Closed enums for known-at-compile-time alternatives (`AgentProvider`,
  `Dimension::{Pixels, Percent}`, `LogLevel`).
- **Hub-and-spokes dispatch — trait + impl-per-sub-enum, single
  delegating impl on the parent.** When you have a forest of closed
  enums each with its own `match self → call method fn` body (clap
  subcommand trees, multi-namespace command routers), lift the contract
  to a small shared trait. The parent enum gets one impl whose body
  delegates via the same trait method. Don't force it for a
  single-level tree or families where each implementer is a one-line
  shell.
- **Comment discipline — terse WHY, never WHAT.** Default to no
  comments. Code + well-named identifiers already describe behavior;
  comments earn their keep only when they encode a non-obvious reason.
- **Multiline fixtures use raw strings** (`r#"..."#` / `r##"..."##`).
- **NVIDIA + Wayland workaround.** `main.rs` sets
  `WEBKIT_DISABLE_DMABUF_RENDERER=1` on Wayland sessions before any
  thread spawns. Overridable by exporting the env var.
- **Config structs** use `#[derive(Debug, Clone, Default, Deserialize,
  Serialize, PartialEq)]` with `#[serde(default, deny_unknown_fields)]`.
  Leaves are `Option<String>` so partial user TOMLs merge.
- **Tests** live next to their module.

## TypeScript / Vue conventions

### Path aliases

Scoped aliases per concern, **not** `@/*`. Kept in sync across
`ui/tsconfig.json`, `ui/vite.config.ts`, and `ui/components.json`:

| Alias | Resolves to | Used for |
| ----- | ----------- | -------- |
| `@ipc` | `./src/ipc` | Tauri `invoke` / `listen` wrappers — tests `vi.mock('@ipc', ...)`. |
| `@lib` | `./src/lib` | TS helpers; `cn` lives here. |
| `@ui` | `./src/components/ui` | shadcn-vue components. |
| `@components` | `./src/components` | Non-shadcn components. |
| `@composables` | `./src/composables` | Vue composables. |
| `@views` | `./src/views` | Views (Vue-only). |
| `@assets` | `./src/assets` | Styles, static assets. |

### Folder barrels

- Every folder containing TypeScript must expose an `index.ts` barrel.
  Imports hit the folder, never the file: `import { cn } from '@lib'`,
  not `@lib/style`.
- Vue-only folders (currently `views/`) skip the barrel; import the SFC
  directly.
- Rename files in one commit that also updates the barrel and every
  import site.

### Type conventions

- **Optional fields use `?` syntax.** `session_id?: string`, not
  `session_id: string | null`. **Why**: Rust-side `Option<T>` serializes
  to `null` on the wire; `?` papers over the type-lie at the consumer
  edge. If a field should disappear entirely on `None`, add
  `#[serde(skip_serializing_if = "Option::is_none")]`. Never bake
  `T | undefined` into a public return shape.
- **Function options are an object, never an overloaded union.**
  `pushToast(tone, message, options: ToastOptions = {})` — never
  `pushToast(tone, message, optionsOrDuration?: number | ToastOptions)`.
  A second knob arrives as a backwards-compatible `options.something?`
  field.
- **Closed sets use `enum`, not union string literals.** Define
  `export enum SessionState { Starting = 'starting', … }` and type
  fields as `state: SessionState`.
- **Wire-contract strings use an `@ipc` enum, not raw literals.** Every
  Tauri `invoke` command name and `listen` event name lives in
  `ui/src/ipc/commands.ts` (`TauriCommand`, `TauriEvent`). Tests that
  mock `@ipc` must spread `vi.importActual` so the enum re-export
  survives.
- **Command → response type is a lookup map, not a generic argument.**
  `TauriCommandResult` (and `TauriEventPayload`) point at the wire
  shape; `invoke` / `listen` infer the response off the map. Drop the
  explicit generic at call sites:

  ```ts
  // right — inferred from TauriCommandResult[TauriCommand.ProfilesList]
  const r = await invoke(TauriCommand.ProfilesList)
  ```

  Every wire-contract interface lives in `ui/src/ipc/types.ts`, not
  inline in the consuming composable.
- **Named types with `T[]` suffix for arrays.** Extract every inline
  object-array type to a named interface
  (`PermissionOptionView[]`) — not `Array<T>`, not inline.

### Naming conventions

- **Filename casing.** `.ts` files are kebab-case (`use-attachments.ts`);
  `.vue` SFCs stay PascalCase (`ChatComposer.vue`).
- **Error variable names are `err`, not `error`.** Applies to Rust
  (`Err(err) => …`) and TypeScript (`.catch((err) => …)`).
- **Names are additive: scope first, noun last.** Build identifiers by
  prepending scope tags. **Drop the scope when the whole tree already
  carries it** (the overlay IS the app — `components/Frame.vue`); **keep
  the scope when it discriminates siblings** (`ChatTurn.vue`, not
  `Turn.vue`). Group related components in subfolders; the folder name
  doubles as the short scope. **Rename over aliasing** — never leave a
  `type X = Y` shim.
- **Names carry no redundant context.** When the scope already names the
  thing, members drop the repeated noun:
  - Composables: always `useFoo`; the returned interface uses bare verbs
    (`useToasts() → { entries, push, dismiss, clear }`, not
    `pushToast`).
  - Component props / events: `Modal { dismissable }`, not
    `dismissableOnClickOutside`. `@dismiss`, not `@modalDismiss`.
  - Methods on a single-purpose type: `.resolve(path)` on a `Sandbox`,
    not `.resolvePath` — the parameter type already says what it
    operates on.
  - Config structs: `[ui.theme.surface] default`, not
    `surface_default_color`.

  The smell signal: a name reads correctly in isolation but repeats
  redundantly at the call site (`useToasts().pushToast()`).

### Style conventions

- **Always brace single-statement control-flow bodies in TypeScript.**
  Never `if (cond) return x` on one line — always open a scope. **Why**:
  the one-liner hides new siblings when the branch grows. Rust's `if` /
  `match` as expressions stay as-is.
- **No `__` in class names.** Use `-` as the separator —
  `.placeholder-header`, not `.placeholder__header`.
- **No `--pilot-*` CSS variables.** All theme tokens are `--theme-*`.
- **No custom animations.** Every animated primitive uses Tailwind v4's
  built-in utilities — `animate-pulse`, `animate-spin`, `animate-bounce`,
  `animate-ping`. Reach for arbitrary-value variants
  (`[animation-duration:1.2s]`) over a fresh keyframe.
- **`<style scoped>` in every Vue SFC, no `lang="postcss"`.** **Why**:
  Tailwind v4's vite plugin only transforms virtual modules whose query
  ends in `.css`; `lang="postcss"` silently bypasses the plugin. Each
  scoped block that uses `@apply` starts with
  `@reference "../assets/styles.css";`.
- Tailwind utility classes use the short aliases declared in
  `ui/src/assets/styles.css::@theme inline` (e.g. `bg-theme-accent`,
  `text-theme-pending`).
- Type scalar theme fields as `string`, not `string | null` — the
  defaults-always-load invariant makes nullable shapes misleading.

### UI stack reference

- **shadcn-vue** component templates live under `ui/src/components/ui/`.
  Copy-paste / `npx shadcn-vue@latest add <component>`.
- **reka-ui** provides headless primitives (Vue port of Radix).
- **class-variance-authority** (`cva`) for typed component variant APIs.
- **clsx + tailwind-merge** composed into `cn()` at
  `ui/src/lib/style.ts`.

### Components compose, they don't bag

**Rule**: where consumers need rendering flexibility, accept a slot, a
render function, or a component reference — never a structured prop bag
of primitives the component pattern-matches over. **Why**: the bag is a
footgun — the next ask ("can the action button show a loading
spinner?") forces a type-widening churn that a slot would have absorbed
for free.

The smell: a prop typed `actions: { id, label, tone, icon, variant }[]`.
When a consumer says "I need an extra knob" and the answer is "extend
the type", the type is hiding a slot.

`Modal.vue` accepts a `#actions` slot for the header button row + a
default body slot. Never `actions: ModalAction[]`; never `markdown` /
`text` body-shape props. Reach for a discriminator (`pushToast`'s `body:
string | (() => VNode) | { component, props }`) only when the caller is
a non-Vue composable — the toast queue isn't a template scope.

**Keep prop bags only for uniform lists** of identically-shaped items
(`PlanItem[]`, `PermissionPrompt[]`). If a consumer wants per-item
customisation, that's the slot signal.

### Source layout — `src/{components,views,composables,interfaces,constants,lib,ipc}`

- `components/` — **reusable, scope-agnostic** Vue building blocks.
- `views/<feature>/` — feature partials. Each owns its SFCs AND the
  composables that exclusively serve them.
- `composables/` — **only** composables that more than one feature reads.
- `interfaces/<domain>/<sub-domain>.ts` — every TypeScript `interface` /
  `type`. **`types.ts` files are forbidden.**
- `constants/<domain>/<sub-domain>.ts` — every `enum` and constant table.
- `lib/` — pure helpers (no Vue, no Tauri).
- `ipc/` — Tauri command + event bridge. Wire types belong under
  `interfaces/ipc/`.

When a `components/` SFC turns out to be single-feature only, move it
under `views/<feature>/`. The default is "live next to your caller";
promotion to `components/` is earned by a second consumer.

### Composables: self-contained `useX(): UseXApi` shape

**Every composable returns a typed interface.** Define `UseFooApi` next
to the composable and have `useFoo()` return it explicitly.

**No drive-by exports** — if a function is exported from a composable
file, it MUST be in the interface returned by `useX()`. Module-level
`pushFoo` / `setFoo` exports are the smell — methods on the returned
interface OR a sibling helper file.

**Test-only helpers** (`__resetFooForTests`) belong in
`tests/<feature>/<helper>.ts`, not the production module.

#### Two-tier composables: store API vs sibling-store mutation surface

The "no drive-by exports" rule has one exception: **instance-keyed store
composables** under `ui/src/composables/instance/`. Tier 1 is the store
API (`useFoo(instanceId?): UseFooApi`); tier 2 is an internal
store-mutation surface (free fns like `pushFooStarted`, `resetFoo`) the
wire-listener routers call to push raw event payloads into the store.

The mutation-surface fns are NOT a casual escape hatch — every caller is
the wire router OR a sibling instance store. Each instance composable
carries a comment header above its mutation block
(`// ── Internal store-mutation surface ───`).

### Icons — direct imports only, no `library.add(...)` registry

FontAwesome `library.add(...)` is **forbidden**. Each component imports
the specific icons it uses directly:

```ts
import { faCircle, faCheck } from '@fortawesome/free-solid-svg-icons'
```

…and binds them via the explicit object form: `<FaIcon :icon="faCircle" />`.
Not `<FaIcon :icon="['fas', 'circle']" />` — **why**: the string-array
indirection defeats Vite's tree-shaking.

### `invoke()` / `listen()` typing — interface-indexed args

**Every Tauri command's argument shape is in `interfaces/ipc/invoke.ts`
keyed by the `TauriCommand` enum.**

```ts
export interface TauriCommandArgs {
  [TauriCommand.SessionSubmit]: SessionSubmitArgs
  [TauriCommand.SessionCancel]: SessionCancelArgs | undefined
}

export async function invoke<K extends TauriCommand>(
  command: K,
  args: TauriCommandArgs[K]
): Promise<TauriCommandResult[K]> { /* ... */ }
```

`undefined` for no-args commands — never overload the call signature
with optional args.

**No named wrapper functions** (`getProfiles()`, `submitTurn()`). Always
`invoke(TauriCommand.X, args)` at the call site.

### Wire shapes for second frontends

A future Neovim plugin plugs into the same wire. Three load-bearing
surfaces a second frontend must speak: **path resolution**
(`paths_resolve` Tauri command + `paths/resolve` JSON-RPC; daemon owns
`~`-expansion + `${VAR}` interpolation + relative→absolute joining);
**caller-supplied candidate ranking** (`completion_rank` Tauri command,
fuzzy-ranked via nucleo, identity order on empty query); **tool-call
presentation** (`formatted: FormattedToolCall` on every record;
frontends read it, only icon resolution stays per-frontend).

### Tool-call formatting lives on the daemon side

The Vue UI is a **dumb consumer** of pre-rendered tool-call views.
Every `acp:transcript` and `acp:permission-request` event carries a
`formatted: FormattedToolCall` field the UI renders verbatim — no
client-side formatter registry.

Implementation lives in `src-tauri/src/formatting/`: closed-set wire
enums + `FormattedToolCall` in `types.rs`; wire-name canonicalisation;
`FormatterRegistry` with per-vendor `register_override` + a four-step
`dispatch` precedence; cross-formatter primitives in `shared.rs`;
per-tool modules under `formatters/<tool>/`. Adding a tool = new module
+ new line in `formatters/mod.rs::register_all`.

The per-instance ACP actor maintains a `ToolCallCache` (running merged
state per `tool_call.id`) and re-formats on every `tool_call_update`.
The UI replaces the prior `formatted` snapshot wholesale by id.

The ONE piece of presentation logic that stays UI-side is the `IconKey`
→ FontAwesome map in `ui/src/lib/tools/icon-map.ts`. A future Neovim
plugin would carry its own version mapping the same keys onto its
icon system.

### Dev preview shim lives in `tests/`, gated by env var

Browser-mode theme + window-state shim lives in `tests/dev-preview` —
its consumers are the test harness and the Vite dev preview only,
never production. `main.ts` gates the import on
`VITE_HYPRPILOT_DEV_PREVIEW === '1'`. The dev script sets the var; prod
builds leave it unset and tree-shake the module out. Not a
`__TAURI_INTERNALS__` window probe — browser-detection by absence is
fragile.

## Frontend linting / formatting

The `ui/` package consumes the workspace-wide config at
`https://gitlab.kilic.dev/config/eslint-config`:

- `ui/eslint.config.mjs` imports `@cenk1cenk2/eslint-config/vue-typescript`
  + appends `utils.configImportGroup`. A local parser override re-applies
  `vue-eslint-parser` + `typescript-eslint` for `<script setup lang="ts">`.
- `ui/.prettierrc.mjs` re-exports `@cenk1cenk2/eslint-config/prettierrc`.
- `eslint` pinned to `^9.39.4`; upgrade when the workspace switches to
  `eslint-plugin-import-x`.

Do not add ad-hoc rules without updating this manual.

## YAML conventions

**Block style only — never JSON-like flow mappings in YAML.** Applies to
every YAML file the repo ships.

```yaml
# wrong — flow-style mapping
- uses: actions/checkout@v6
  with: { lfs: true }

# right — block style
- uses: actions/checkout@v6
  with:
    lfs: true
```

GitHub Actions expression syntax `${{ … }}` stays as-is — it is a string
substitution context, not YAML structure.

## Agents

- `.mcp.json` at the repo root is the repo-scoped MCP server registry.
  Add servers you need during a task, remove them at merge if they
  aren't load-bearing.
- Every issue is picked up in a dedicated branch. Never implement on
  `main`.
- Issue workflow: `linear-issue-implement` → `git-branch` →
  `agents-sequential` / `agents-team` → `git-commit` →
  `gitlab-pr-create` → review → merge.
- Commit style: conventional commits with a `refs K-<id>` or
  `closes K-<id>` trailer.
- Prefer MCP tools over CLIs for git, GitLab, Linear, Obsidian, Tmux.
  Fall back to CLI only when the MCP server lacks the operation.

## JSON-RPC over the daemon socket

The `ctl` subcommands and the daemon talk over
`$XDG_RUNTIME_DIR/hyprpilot.sock` using newline-delimited JSON (NDJSON) —
one JSON-RPC 2.0 object per line, both directions. Implementation lives
in `src-tauri/src/rpc/`; the client is `src-tauri/src/ctl/client.rs`.
Every accept spawns a per-connection task.

### Methods

Live methods, grouped by namespace. Result shapes are abbreviated; see
`src-tauri/src/rpc/handlers/` for authoritative types.

- **`daemon/*`** — `kill` (calls `app.exit(0)` after flush; best-effort
  delivery), `status`, `version`, `reload` (re-runs `config::load` +
  `SkillsRegistry::reload()`; publishes `DaemonReloaded`),
  `shutdown { force? }` (graceful; refuses with `-32603` when any
  instance has an in-flight turn unless `force`).
- **`diag/snapshot`** — read-only structural snapshot:
  `{ daemon, instances, profiles, skills, mcps, configPaths }`.
  **Redacted**: profile `env` values + transcript bodies never appear.
- **`events/subscribe`** — live `InstanceEvent` stream as JSON-RPC
  notifications. Optional `{ instanceId? }` filter scopes per-instance
  events; daemon-global events (`daemon:reloaded`,
  `acp:instances-changed`, `acp:instances-focused`) always pass through.
  Single subscription per connection (`-32600` on second). Notification
  shape: `{ method: "events/changed", params: { name, payload, instanceId? } }`
  where `name` is the colon-separated public event name
  (`acp:transcript`, `acp:turn-started`, etc., from
  `InstanceEvent::event_name()`). Lag surfaces as `events/lagged`
  with a `skipped` count — peer should re-fetch via
  `instance/snapshot/chat`.
- **`instance/snapshot/{meta, chat, terminals}`** — per-instance state
  mirror reads. `meta` returns header chrome (mode, model, available
  modes/models, cwd, mcps_count, profile id, pending permissions,
  usage); `chat` paginates the transcript backwards via `before` cursor
  + `limit`; `terminals` returns the full per-`terminal_id` map.
  Powers second-frontend hydration on connect.
- **`instances/*`** — `list`, `focus`, `spawn`, `restart`, `shutdown`,
  `info`, `rename`. Live process management. `focus` accepts
  `{ ensure: true }` to auto-spawn-if-missing; `spawn` accepts
  `{ restore: true }` to resume an existing session id.
- **`overlay/*`** — `present { instanceId? }`, `hide`, `toggle`.
  Race-safe across concurrent calls — every `overlay/*` entry serialises
  through `WindowRenderer::lock_present`.
- **`permissions/{pending, respond}`** — pending list for the addressed
  instance + decision write. Mirrors what the desktop overlay does
  through `permission_reply` Tauri command. After `respond`, an
  `acp:permission-resolved` event fires on the broadcast so any
  second subscriber clears their row.
- **`prompts/{send, cancel}`** — per-instance scripting surface.
  `send` accepts `{ instanceId | name, text }`; the request adopts the
  client-minted instance id verbatim. `cancel` interrupts the active
  turn.
- **`status/*`** — `get` (one-shot), `subscribe` (registers connection;
  server pushes `status/changed` notifications). `StatusResult`:
  `{ state: "idle" | "streaming" | "awaiting" | "error", visible, active_session }`.
- **`tauri/<command>`** — proxy for every Tauri webview command
  (`session_submit`, `session_load`, `session_list`, `models_set`,
  `modes_set`, `permission_reply`, `completion_query`,
  `boot_snapshot`, `agents_list`, `profiles_list`, `mcps_list`,
  `skills_list/get/reload`, `paths_resolve`, `read_file_for_attachment`,
  `instance_snapshot_{meta,chat,terminals}`, `instance_meta`,
  `instances_focus`, `window_toggle`, etc.). One namespace covers
  every action verb the SPA uses, so a second frontend has full
  parity. Source of truth: `rpc/handlers/tauri_proxy.rs`.

**Namespace convention**: every method on the wire uses
`namespace/name`, matching ACP's own methods (`session/prompt`,
`session/new`). Bare method names are dead — they receive
`-32601 method not found`. Methods without params omit the `params` key.
`status/changed` and `events/changed` are server-push notifications
(no `id`). Request ids are per-call UUID v4 strings; server echoes
them verbatim.

### Error codes

JSON-RPC 2.0 standard codes: `-32700` parse error (`id` echoes as
`null`), `-32600` invalid request, `-32601` method not found, `-32602`
invalid params, `-32603` internal error. `-32000..=-32099` is reserved
for hyprpilot-specific errors; none defined yet.

### Design notes

- **Framing**: NDJSON on top of `tokio::io::BufReader::lines`.
- **Dispatcher**: hand-rolled on `serde_json`. Each handler implements
  `RpcHandler` and parses its own `params: Value` into a typed struct.
  Extending = one `RpcHandler` impl + one line in
  `RpcDispatcher::with_defaults`.
- **No auth**: single-user assumption.
- **`ctl` is one-shot** for most commands. Connection failure prints
  `"hyprpilot daemon is not running"` and exits `1`.
- **`ctl status --watch` is persistent**: reconnects with back-off
  (1s → 2s → 5s) on socket loss, emitting an offline payload between
  attempts.
- **`StatusBroadcast`** wraps a capacity-32 broadcast channel + a
  `Mutex<StatusResult>` snapshot. Slow consumers drop messages — waybar
  re-renders from the next tick.

### Mirror — daemon-side state cache

Every live `AcpInstance` owns an `Arc<RwLock<InstanceMirror>>`
(`src-tauri/src/adapters/mirror.rs`, ~1500 lines). The mirror is a
**write-through cache**: every line that emits an
`InstanceEvent::*` onto the registry's broadcast also calls
`mirror.write().apply(&evt)` in the same actor tick. The
`publish()` helper (in `acp::instance`) enforces the apply-then-
broadcast ordering so a snapshot fetched mid-burst never sees
events the broadcast already shipped.

Captured axes:

- `transcript: Vec<TranscriptItem>` — user / agent / thought
  messages, tool call records, turn markers, mode-update markers,
  system-prompt-injection markers. Bounded ring buffer (capped per
  instance to keep daemon memory bounded; older entries fall off
  the front; consumers re-fetch via backward pagination).
- `tool_calls: HashMap<ToolCallId, ToolCallRecord>` — current
  merged state per running/completed call. Sourced from the
  per-instance `ToolCallCache` (lifted out of the actor's local
  vars in PR #26). `formatted` is authoritative — recomputed by
  the formatter registry on every `tool_call_update`.
- `terminals: HashMap<TerminalId, TerminalSnapshot>` — current
  scrollback + running flag + exit code/signal.
- `meta` — same shape `instance_meta` returns (mode, model,
  available_*, cwd, mcps_count, profile id) plus `pendingPermissions`
  + `usage` so a meta snapshot is one round-trip.
- `last_turn_event: { TurnStarted | TurnEnded | None }` — enough
  for the UI's phase derivation.

The mirror is the single hydration source for **every** snapshot RPC
(`instance/snapshot/{meta, chat, terminals}`) AND every Tauri
command of the same shape. Captain's invariant: any new
`InstanceEvent` variant must have a corresponding
`InstanceMirror::apply` arm — a `match` on the enum keeps the
compiler honest.

### External clients — the second-frontend contract

The daemon's design separates **transport** (unix socket, WS
remote bridge, Tauri webview) from **the dispatcher**. Every
transport reuses one `RpcDispatcher` and one event broadcast, so
a second frontend gets full parity by speaking the wire
contract — no daemon code changes.

**Hydrate-then-subscribe** is the model. A new connection:

1. Calls `instance/snapshot/meta { instanceId }` for header chrome
   (mode, model, pending permissions, usage). One RPC.
2. Calls `instance/snapshot/chat { instanceId, limit }` for the
   most-recent transcript page. Backward-paginates via
   `before: <oldestSeq>` cursors when the captain scrolls up.
3. Calls `instance/snapshot/terminals { instanceId }` only when
   needed (the chat snapshot mentions a terminal id).
4. Calls `events/subscribe { instanceId? }` to start receiving
   live `events/changed` notifications. Filter scopes per-instance
   events; daemon-global events (`daemon:reloaded`,
   `acp:instances-changed`, `acp:instances-focused`) always pass.

After the brim-sync, the client patches its in-memory model with
each notification's payload — same shape the Tauri webview
processes through its accumulator stores. On `events/lagged`
(broadcast capacity exceeded), the client re-fetches via
`instance/snapshot/chat` to bridge the gap.

**Action surface** (writes from the client back to the daemon)
rides existing JSON-RPC verbs on the **same connection** — no
separate write socket, no auth handshake:

- Submit a prompt: `prompts/send { instanceId | name, text }` or
  `tauri/session_submit { ... }` (full attachment surface).
- Cancel an in-flight turn: `prompts/cancel { instanceId }` or
  `tauri/session_cancel`.
- Answer a permission prompt: `permissions/respond {
  instanceId, requestId, optionId }` or `tauri/permission_reply`.
- Switch mode / model: `tauri/modes_set` / `tauri/models_set`.
- Resume a stored session: `tauri/session_load`.
- Spawn / focus / restart / rename / shutdown an instance:
  `instances/*`.
- Read everything else (profiles, agents, skills, mcps, paths)
  via the matching `tauri/<command>`.

**Multiplexing**: each accepted unix-socket connection runs its
own per-connection task; concurrent dispatch on a per-request
`JoinSet`. No auth (single-user assumption). Waybar's `ctl
status --watch`, the desktop overlay's WS bridge, a Neovim
plugin, and a one-off `socat` pipe can all coexist.

**Tool-call presentation**: `FormattedToolCall` rides on every
`acp:transcript` and `acp:permission-request` event with
pre-rendered `title` / `stats[]` / `description` / `output` /
`fields[]`. The client renders these verbatim — the only
per-frontend logic is mapping the daemon's `IconKey` enum onto
the local icon system (FontAwesome on the SPA; nerd-fonts on a
Neovim plugin; no icons in `ctl`). See
`src-tauri/src/tools/formatter/types.rs::FormattedToolCall` for
the schema.

**Existing client implementations**:

- `src-tauri/src/ctl/` — operator CLI. One `CtlHandler` impl per
  subcommand, all sharing a `CtlClient` factory (so handlers
  reconnect with back-off without holding a live connection).
  `ctl status --watch` is the canonical `status/subscribe`
  consumer; the same shape with `events/subscribe` is the entry
  for `ctl events --watch` and any second frontend.
- `src-tauri/src/remote/ws.rs` — TLS WebSocket bridge for the
  phone / browser remote. Wraps the same dispatcher; subscribes
  to events at handshake time (so a peer-issued
  `events/subscribe` over WS returns -32600 — there's already a
  stream open).

**Adding a new client** = no daemon work. Open a unix socket,
NDJSON-frame the JSON-RPC envelope, dispatch by `id` (responses)
or `method` (notifications). The captain's `nvim-hyprpilot.lua`
PoC plan lives at `docs/plans/2026-05-09-nvim-plugin-handoff.md`.

### Client-side handler pattern (`ctl`)

The `ctl` CLI mirrors the server's `RpcHandler` split: one struct per
subcommand, one `CtlHandler` trait, a shared `CtlClient` factory.

**Rule**: handlers take `&CtlClient` (the path), not a live
`CtlConnection`. **Why**: `StatusHandler --watch` reconnects in a loop
with back-off when the socket drops; passing the factory satisfies both
streaming and one-shot handlers without branching in the trait.

**Status is the only non-plain handler.** `StatusHandler` never exits
non-zero — waybar's `exec` expects valid JSON even when the daemon is
down, so transport / RPC errors fall through to the `offline()`
sentinel and exit 0.

## ACP bridge

The daemon fronts one or more ACP-speaking agent subprocesses.
`session/submit` resolves the addressed profile (or falls back through
`[agent] default_profile` → first `[[agents]]` entry), spawns the
configured vendor on first hit, wires a `Client.builder().connect_with`
pipeline against its stdio, and streams `SessionUpdate`s through to the
webview (`acp:transcript` Tauri events) + the `ctl status` broadcast.

Follow-up prompts against the same `(agent_id, profile_id)` reuse the
live session; a different profile against the same agent spawns its own
actor so system-prompt + model overlays stay deterministic.

### Module layout (`src-tauri/src/adapters/`)

The generic adapter layer + the ACP impl as one transport among many.
`rpc::` / `ctl::` / `daemon::` talk to `dyn Adapter` or to the concrete
`AcpAdapter` re-exported from `adapters::*`; they never `use
crate::adapters::acp::*` directly (enforced by the
`no_acp_imports_outside_adapters` test).

**Generic layer** (`src-tauri/src/adapters/`) — `Adapter` trait,
`AdapterRegistry<H: InstanceActor>` (instance map + insertion-order vec
+ focused-id pointer + event broadcast), `InstanceKey` (UUID newtype),
`InstanceState`, `InstanceEvent` (with dot-separated `topic()`),
`TranscriptItem`, `TurnRecord`, `ToolCallRecord`, `UserTurnInput`,
`Attachment`, `PermissionPrompt`, `ToolCall`, `ResolvedInstance`. Tauri
`#[command]`s in `commands.rs` dispatch through `Arc<dyn Adapter>`.

**ACP impl** (`src-tauri/src/adapters/acp/`):

- `agents/{claude_code,codex,opencode}.rs` — `AcpAgent` trait + vendor
  unit structs. `match_provider_agent(provider)` resolves a
  `Box<dyn AcpAgent>` off the closed `AgentProvider` enum.
- `spawn.rs` — wraps `AcpAgent::spawn` + `inject_system_prompt`.
- `client.rs` — `AcpClient` — `on_receive_*` handlers for
  `Client.builder()`. `SessionNotification`s fan out onto a
  per-instance mpsc.
- `runtime.rs` — one `tokio::spawn`ed actor per instance. Drives
  `initialize` → `session/new` → `session/prompt` for the first
  prompt, then loops on an mpsc of `InstanceCommand::{Prompt, Cancel,
  ListSessions, Shutdown}`.
- `instance.rs` — `AcpInstance` per-actor handle.
- `instances.rs` — `AcpAdapter`. Composes
  `Arc<AdapterRegistry<AcpInstance>>`. Owns config, the permission
  controller, and the runtime-events → generic-events bridge.
- `mapping.rs` — `From` / `TryFrom` bridges between ACP wire DTOs and
  generic `adapters::*` vocabulary.

### Composable registry

The generic `AdapterRegistry<H: InstanceActor>` is the single owner of
per-transport instance state. Every adapter facade composes
`Arc<AdapterRegistry<TheirInstance>>` and implements generic methods
(`list` / `focus` / `shutdown_one` / `restart` / `info_for` /
`subscribe`) as one-line delegations. The facade owns transport-specific
bits (resolve / spawn / submit / cancel / load_session); the registry
owns the shared machinery.

**Auto-focus policy:**

- **Empty-registry → first-spawn** auto-focuses and emits
  `InstancesFocused` alongside `InstancesChanged`.
- **Shutdown of focused → oldest survivor** (insertion-order
  `order.first()`). Empty registry → focus clears to `None`.
- **Restart preserves slot.** `drop_preserving_slot` →
  `insert_at_slot(slot, same_key, new_handle)`. The `InstanceKey` (UUID)
  is preserved.

**Documented races**: `shutdown_one` releases all locks before awaiting
the actor's shutdown ack (2s timeout); a concurrent `insert` between
drop and auto-focus can land on `order.first()`. Callers reconcile via
the `InstancesFocused` event stream. `focus` holds `instances` +
`focused` locks across the check + write (TOCTOU-safe).

**Broadcast contract**: `AdapterRegistry::subscribe` returns a
`broadcast::Receiver<InstanceEvent>` over a capacity-256 channel. Every
consumer MUST handle `broadcast::error::RecvError::Lagged`.

**Topic naming — two axes**: Tauri event names use `:` (`acp:transcript`,
`acp:instances-changed`); `InstanceEvent::topic()` returns dot-separated
strings (`instance.transcript`, `instances.changed`).

### Per-vendor system-prompt injection

`AcpAgent::inject_system_prompt(cmd, prompt) -> SystemPromptInjection`
runs at spawn time and either:

- mutates `cmd` directly (CLI flag, `-c` override, env var) and returns
  `SystemPromptInjection::Handled`; or
- returns `SystemPromptInjection::FirstMessage(text)`, in which case the
  runtime prepends `text` onto the first `session/prompt` text block
  (`\n\n` separated) and clears the slot.

`acp-codex` uses `Handled` (mutates `cmd` with `-c instructions=<json>`,
which the native binary merges into TOML config). `acp-claude-code` and
`acp-opencode` use `FirstMessage` because neither vendor exposes a
launch-time injection hook.

### `agent-client-protocol` 0.11 runtime notes

The 0.11 crate exposes a builder API — `Client.builder()
.on_receive_notification(…) .on_receive_request(…)
.connect_with(transport, main_fn)` — whose futures are all `Send`. No
`LocalSet` or `current_thread` runtime is required; the daemon stays on
the default Tauri-managed multi-thread runtime. Transport is
`ByteStreams::new(stdin.compat_write(), stdout.compat())`.

### Agents + profiles config (flattened at TOML root)

```toml
[agent]                          # singleton: global agent-scope config
default = "claude-code"
default_profile = "ask"

[[agents]]                       # registry: per-agent entries
id = "claude-code"
provider = "acp-claude-code"     # closed AgentProvider enum
model = "claude-sonnet-4-5"
command = "bunx"
args = ["--bun", "@zed-industries/claude-code-acp"]

[agents.env]                     # optional per-agent env overlay

[[profiles]]                     # registry: per-profile presets
id = "strict"
agent = "claude-code"            # must reference a real [[agents]] id
model = "claude-opus-4-5"        # profile > agent > vendor
system_prompt = ["~/.config/hyprpilot/prompts/base.md", "~/.config/hyprpilot/prompts/strict.md"]
```

Singular `[agent]` parallels plural `[[agents]]` / `[[profiles]]` —
TOML's single-table vs array-of-tables distinction carries the "global
config vs registry" split.

**Merge semantics**: user entries with an existing `id` override the
whole default entry; new `id`s append. Whole-entry replace, no
field-level merge inside an entry.

**Cross-field rules:**

- `agent.default` → `[[agents]].id`.
- `agent.default_profile` → `[[profiles]].id`.
- `profile.agent` → `[[agents]].id`.
- `profile.system_prompt` is a single field — array of file paths, no
  inline-string variant. Empty array is the explicit "no prompt"
  off-switch.

`AgentProvider` is a **closed enum** keyed by wire name
(`acp-claude-code` / `acp-codex` / `acp-opencode`); adding a provider
means a new enum variant + `AcpAgent` impl + match arm in
`match_provider_agent`.

### Shutdown orchestration

Process lifecycle lives in `daemon`, not `rpc`. `daemon::shutdown(app,
adapter)` is the one orchestrator; it drains adapter instances via
`AcpAdapter::shutdown_all`, then calls `app.exit(0)`.

Four call sites funnel through it: `daemon/kill` RPC (returns
`{"killed": true}`; dispatcher inspects the payload after flush),
`daemon/shutdown` RPC (graceful; refuses with `-32603` when any
instance is busy unless `force = true`), SIGINT, SIGTERM. First signal
triggers the orchestrator; a second falls through to the default
handler (force-kill).

Socket file is not explicitly removed — next-start probes stale sockets
via `ECONNREFUSED`.

### Permissions are the vendor's concern

ACP delivers a `PermissionOption[]` per `session/request_permission`
and expects the client to pick one option id. Hyprpilot does **not**
ship a policy layer on top: vendor-side modes (plan / approval / tool
filters) already give users granular control. The daemon forwards every
permission request straight to the webview as `acp:permission-request`;
the user picks via dialog and replies with `permission_reply`.

Client-side auto-accept / auto-reject lives on the
`PermissionController` as a two-lane pipeline:

1. **Runtime trust store** — `(instance_id, tool_name) → Allow|Deny`,
   populated by "always allow" / "always deny" buttons via
   `permission_reply { remember }`. Cleared on instance shutdown /
   restart. In-memory only.
2. **Per-server hyprpilot extension globs** — each MCP JSON entry's
   optional `hyprpilot.autoAcceptTools` / `autoRejectTools`. Tool→server
   attribution by `mcp__<server>__<tool>` prefix.

Reject beats accept inside each lane. Vendor-native tools (Bash, Read,
…) skip lane 2 entirely. Misses on both lanes fall through to AskUser.

### Tauri commands + events (live)

**Commands**: `session_submit`, `session_cancel`, `permission_reply`,
`agents_list`, `profiles_list`, `session_list`, `session_load`. Argument
shapes are typed in `interfaces/ipc/invoke.ts` keyed by `TauriCommand`;
see `src-tauri/src/adapters/commands.rs` for the authoritative Rust side.
Notable: `session_submit.instance_id` is a client-minted UUID
(`crypto.randomUUID()`); subsequent submits pass the same id. `session_load`
is gated on `agent_capabilities.load_session`.

**Events**:

- `acp:transcript` `{ agent_id, instance_id, session_id, update }` — every
  `SessionNotification`; `update` is the raw `SessionUpdate` JSON.
- `acp:permission-request` `{ agent_id, instance_id, session_id, options }`
  — every `session/request_permission`.
- `acp:instance-state` `{ agent_id, instance_id, session_id?, state }` —
  every lifecycle transition (`starting` / `running` / `ended` / `error`).

Event names use `:` (Tauri convention); the JSON-RPC wire keeps `/`;
config uses `.`; CSS uses `-`.

### User-turn input + attachments

`UserTurnInput::Prompt { text, attachments }` is the only variant.
`Attachment` is the generic palette-picked-context shape:

```rust
pub struct Attachment {
    pub slug: String,        // "git-commit"
    pub path: PathBuf,       // /home/.../skills/git-commit/SKILL.md
    pub body: String,        // snapshot at pick time
    pub title: Option<String>,
}
```

ACP mapping in `adapters/acp/mapping.rs::build_prompt_blocks` projects
each attachment onto a `ContentBlock::Resource` carrying an
`EmbeddedResource { resource: TextResourceContents { uri:
"file://<path>", mime_type: Some("text/markdown"), text: <body> } }`,
prepended before the trailing `ContentBlock::Text` — agent reads context
first, then user instructions. Body snapshots at palette-pick time.

### Glossary

- **session** — the ACP wire session id (issued by the agent via
  `session/new`). Only meaningful inside `adapters::acp`.
- **instance** — our owner/record of a running agent process + its ACP
  session + its channels. Keyed by `InstanceKey` (a `Uuid` newtype).
  Outlives any single `session/new` cycle.
- **profile** — user-config bundle of agent + model + cwd + system
  prompt + mode.
- **mode** — per-instance operational mode (e.g. claude-code's `plan` /
  `edit`). Threaded through `SpawnSpec → ResolvedInstance →
  AcpInstance → InstanceInfo`.
- **agent** — the vendor process/binary (claude-code, codex, opencode).
- **adapter** — the transport trait (`adapters::Adapter`). ACP is one
  impl; HTTP-based agents will be another.
- **registry** — `AdapterRegistry<H: InstanceActor>` — the generic
  per-adapter instance map + insertion-order vec + focus pointer +
  event broadcast.

## What is not in the scaffold

- Persistent disk-backed trust store (today's runtime store is
  in-memory).
- Real branding icon — `src-tauri/icons/icon.png` is a placeholder.
- Release bundling (`bundle.active = false` in `tauri.conf.json`).
- CI / `.gitlab-ci.yml`.

## Upstream migration runway

Pending upstream moves that will drive a hyprpilot bump. Whenever an
upstream ships a tracked migration, follow the linked checklist in the
same commit that bumps the dep, and **delete the row from this section
when the work lands**.

### wry / Tauri → GTK4 + webkit2gtk-6.0

- **Tracking**: [`tauri-apps/wry#1474`](https://github.com/tauri-apps/wry/issues/1474) (open, prioritized).
- **Port PR**: [`tauri-apps/wry#1530`](https://github.com/tauri-apps/wry/pull/1530) (open; unmerged).
- **Current binding**: GTK3 via `gtk = "0.18"` / `gdk = "0.18"` /
  `gtk-layer-shell = "0.8"`, webview via `webkit2gtk` 4.1.
- **Why it matters**: gtk-rs GTK3 crates are archived
  (RUSTSEC-2024-0411..0420) and `glib < 0.2` carries a known unsoundness.

When wry#1530 merges and Tauri publishes a release consuming it,
migrate in one PR:

1. Bump `tauri` in `src-tauri/Cargo.toml`.
2. Swap Linux deps: `gtk` → `gtk4`, `gdk` → `gdk4`, `gtk-layer-shell` →
   `gtk4-layer-shell`. Drop the `v0_6` feature.
3. Update `apply_anchor_mode`: GTK3 prelude → GTK4 prelude;
   `gtk_window.show_all()` → `set_visible(true)`; `gtk_window.hide()` →
   `set_visible(false)`; `present()` stays.
4. Revisit `WEBKIT_DISABLE_DMABUF_RENDERER=1` in `main.rs` — drop if 6.0
   handles DMABUF cleanly on NVIDIA.
5. Swap the system-library note to `gtk4-layer-shell`.
6. Paste pre/post `hyprctl layers` output into the PR.

**Do not preempt upstream.** Vendoring wry's fork or cherry-picking
trades compile-time pain for a feature already prioritized.

### Other open debt worth tracking

- **Playwright `tauri` mode against WebKitGTK.** `webview.eval`
  callbacks stall on `webkit2gtk-4.1`. E2E runs in `browser` mode by
  default; `HYPRPILOT_E2E_MODE=tauri` flips over once the stall clears
  (likely with the GTK4 + webkit2gtk-6.0 migration above).
- **Release bundling.** `bundle.active = false`. Lifting it needs real
  icons + the pipelines issue.
- **CI / `.gitlab-ci.yml`.** Not yet created.
- **Real branding icon.** Programmatic 32×32 placeholder.

## Manual verification patterns

`task test`, `task lint`, `task format` are the automated bar. Beyond
that, **every feature that changes runtime behavior lands with a manual
smoke-test block in its PR description** — concrete commands + literal
observed output so a reviewer can re-run against the branch. "Should
pass" is not evidence; paste the actual response.

### Baseline smokes (extend per feature)

These cover the scaffold's surface and should stay green on every PR:

- `task install && task build` produces `target/debug/hyprpilot`.
- `./target/debug/hyprpilot {--help, daemon --help, ctl --help}` render
  via clap.
- `./target/debug/hyprpilot daemon` opens a window and
  `$XDG_RUNTIME_DIR/hyprpilot.sock` is bound. Second `daemon` exits `0`
  via single-instance without spawning a second window.
- `ctl <cmd>` round-trips JSON-RPC; daemon-not-running → exit 1, stderr
  `"hyprpilot daemon is not running"`.
- A deliberately broken `config.toml` aborts startup with a readable
  garde error naming the field path.
- Partial config overrides compose: setting only one nested theme token
  keeps every sibling falling through to `defaults.toml`.

### Layer-shell / anchor mode

- `hyprctl layers` (or `swaymsg -t get_tree` on Sway) lists a layer with
  `namespace: hyprpilot` and the configured `xywh`.
- `[daemon.window.anchor] edge = "left"` via `--config` moves the
  surface without a rebuild.
- `[daemon.window] mode = "center"` yields a regular top-level — **no**
  entry under `hyprctl layers`.
- `[daemon.window.anchor] margin = 20` shifts the surface 20px from the
  anchored edge.

### JSON-RPC / ctl

- All `ctl` subcommands round-trip; stdout is the pretty-printed JSON
  `result`, exit 0.
- Raw socket probes (via `socat`, `ncat`, or python `UnixStream`): a
  valid request returns a `result` envelope; `not json` returns
  `-32700`; missing `jsonrpc` returns `-32600`; unknown method returns
  `-32601`.

### When a check needs a Wayland session

Most layer-shell / window checks require running on Hyprland or Sway.
Call that out in the PR's verification block so a non-Wayland reviewer
knows why it isn't reproducible from CI.

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

`rust-toolchain.toml` covers toolchain pinning for `cargo` invocations outside
mise.

## Tasks

Every `task` target orchestrates both Rust and the frontend where applicable.
Exactly the targets listed below exist — no others should be added without
updating this file.

| Task | Purpose |
| ---- | ------- |
| `task install` | `cargo fetch` + `pnpm --dir ui install`. |
| `task dev` | `./ui/node_modules/.bin/tauri dev` — full dev cycle with Vite + Tauri (CLI is a Node devDep of `ui/`). |
| `task test` | `cargo test --all-targets` + `pnpm --dir ui test` + `pnpm --dir ui test:e2e`. |
| `task format` | `cargo fmt --all` + `pnpm --dir ui format` (Prettier + eslint --fix). |
| `task lint` | `cargo fmt -- --check` + `cargo clippy --all-targets -- -D warnings` + eslint + `vue-tsc --noEmit`. |
| `task build` | Debug build via `./ui/node_modules/.bin/tauri build --debug`. |
| `task "build:release"` | Release build via `./ui/node_modules/.bin/tauri build`. |

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
3. Per-profile config — `$XDG_CONFIG_HOME/hyprpilot/profiles/<name>.toml` when
   `--profile <name>` / `HYPRPILOT_PROFILE` is supplied.
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

## Theming

**The palette lives in Rust, not CSS.** Flow:

1. `src-tauri/src/config/defaults.toml` seeds every theme token under
   `[ui.theme.*]`.
2. User TOMLs override any subset; `merge_theme` walks the tree field-by-field
   using `.or()` over `Option<String>` leaves.
3. `config::Theme` is a typed tree. Groups: `font`, `window`
   (`default` + `edge`), `surface` (`card.{user,assistant}`, `compose`,
   `text`), `fg` (`default`/`dim`/`muted`), `border`
   (`default`/`soft`/`focus`), `accent` (`default`/`user`/`assistant`),
   `state` (`idle`/`stream`/`pending`/`awaiting`).
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
- CSS must not declare literal theme values on `:root`. Rust is the single
  source of truth. Only exceptions: three `var(--token, literal)` fallbacks
  on the body / app / window-edge rules — the tokens that affect the first
  visible frame, to avoid FOUC before `applyTheme()` resolves. Keep those
  literals in sync with `defaults.toml`.
- The Tauri window's native `backgroundColor` (in `src-tauri/tauri.conf.json`)
  is painted before the webview loads; keep it equal to
  `[ui.theme.window] default`.
- **Do not introduce new `--pilot-*` vars.** All theme tokens are `--theme-*`.
- Cards are keyed by speaker: `surface.card.user`, `surface.card.assistant`.
  Each is an object (`bg` today; `accent` / `border` / `fg` later). Do not
  name surfaces by elevation (`card_hi`, `card_alt`); name them by role.

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

## Rust conventions

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

### Style conventions

- **No `__` in class names.** Use `-` as the separator — `.placeholder-header`,
  not `.placeholder__header`.
- **No `--pilot-*` CSS variables.** All theme tokens are `--theme-*`.
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
| `submit` | `{ "text": "..." }` | `{ "accepted": true, "text": "..." }` | Stub. No ACP session yet. |
| `cancel` | *(none)* | `{ "cancelled": false, "reason": "no active session" }` | Stub. |
| `toggle` | *(none)* | `{ "visible": bool }` | Flips main window visibility; updates `StatusBroadcast` visible bit. |
| `kill` | *(none)* | `{ "exiting": true }` | Calls `app.exit(0)` after write + flush. Delivery is best-effort: the process may tear down before the peer finishes reading. |
| `session-info` | *(none)* | `{ "sessions": [] }` | Stub. |
| `status/get` | *(none)* | `StatusResult` | One-shot status snapshot. |
| `status/subscribe` | *(none)* | `StatusResult` (initial) | Registers connection as subscriber; server pushes `status/changed` notifications. |
| `status/changed` | `StatusResult` | *(notification, no id)* | Server-push on every state transition. Clients receive this after `status/subscribe`. |

`StatusResult` shape: `{ "state": "idle" | "streaming" | "awaiting" | "error", "visible": bool, "active_session": string | null }`.

Method names are kebab-case on the wire (`session-info`). Status methods use
`namespace/name` notation (`status/get`, `status/subscribe`). Methods without
params (`cancel` / `toggle` / `kill` / `session-info`) omit the `params`
key entirely — the server accepts `{"method":"toggle"}` with no `params`
and responds normally. `status/changed` is a server-push notification — it
carries no `id` and is not a response to a request.

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
- `-32603` internal error (handler failed — `toggle` against a missing
  window, serializer failures, etc.).

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

**Stub handlers `unimplemented!()` instead of calling the server**
(see "Stubs panic, they don't pretend" in Rust conventions). Today
`SubmitHandler` / `CancelHandler` / `SessionInfoHandler` all panic
with `"ctl <verb>: ACP bridge not yet implemented (K-239)"`.

## What is not in the scaffold

The following deliberately land in their own issues — do not bolt them onto
scaffold work:

- ACP adapter, MCP server(s), skills loader, permissions store, markdown
  rendering, waybar, profile switcher UI.
- Playwright e2e wiring (`tauri-driver` + WebKitGTK WebDriver shim) — the
  current e2e is `test.skip` only.
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

- **Playwright e2e wiring.** `ui/e2e/placeholder.spec.ts` is `test.skip`
  only; lands when we wire `tauri-driver` + WebKitGTK's WebDriver shim.
  After the GTK4 migration above, the shim likely becomes `webkit6gtk`
  equivalent — the two deltas are adjacent and may fall out of one PR.
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

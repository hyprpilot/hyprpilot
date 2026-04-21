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

# CLI client (socket wire protocol not wired yet; logs + exits 0)
./target/release/hyprpilot ctl submit "hello there"
./target/release/hyprpilot ctl toggle
./target/release/hyprpilot ctl --help
```

Second `hyprpilot daemon` forwards argv through `tauri-plugin-single-instance`
and exits `0` without opening a second window.

## Config layering

Sources resolve in this order; later layers override earlier ones for the
fields they set.

1. Compiled defaults — `src-tauri/src/config/defaults.toml` embedded via
   `include_str!`.
2. Global config — `$XDG_CONFIG_HOME/hyprpilot/config.toml` or `--config <path>`.
3. Per-profile config — `$XDG_CONFIG_HOME/hyprpilot/profiles/<name>.toml` when
   `--profile <name>` / `HYPRPILOT_PROFILE` is supplied.
4. `clap` flags — override-per-invocation, never persisted.

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

## Logging

`tracing` is bootstrapped once via `logging::init`. Both the dev stderr layer
and the release file layer tag every event with its `file:line` callsite +
module target. Helpers:

- `dev_fmt_layer` — ANSI on, stderr writer.
- `file_fmt_layer` — ANSI stripped, rolling file under
  `$XDG_STATE_HOME/hyprpilot/logs/hyprpilot.log.*` via `tracing-appender`.

Filter precedence: `--log-level` → `RUST_LOG` → `info` fallback.

## Rust conventions

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

## What is not in the scaffold

The following deliberately land in their own issues — do not bolt them onto
scaffold work:

- Layer-shell / right-edge compositor anchoring.
- JSON-RPC method set and server/client dispatch tables.
- ACP adapter, MCP server(s), skills loader, permissions store, markdown
  rendering, waybar, profile switcher UI.
- Playwright e2e wiring (`tauri-driver` + WebKitGTK WebDriver shim) — the
  current e2e is `test.skip` only.
- Real branding icon — `src-tauri/icons/icon.png` is a generated 32×32
  placeholder.
- Release bundling (`bundle.active = false` in `tauri.conf.json`).
- CI / `.gitlab-ci.yml`.

## Verification checklist (used by the scaffold issue)

- `task install && task build` succeeds on a clean clone.
- `./target/debug/hyprpilot daemon` opens a window and
  `ls $XDG_RUNTIME_DIR/hyprpilot.sock` confirms the socket is bound.
- Second `hyprpilot daemon` invocation exits `0` without spawning a window.
- `./target/debug/hyprpilot ctl submit hello` logs and exits `0`.
- `task test`, `task lint`, `task format` all pass.
- A deliberately broken `config.toml` aborts startup with a readable error.
- A partial `[ui.theme.surface.card.user] bg = "#..."` override merges cleanly
  over defaults; untouched tokens still fall through.

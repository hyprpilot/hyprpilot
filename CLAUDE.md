# CLAUDE.md

Agent operating manual for `hyprpilot`. Read this first; the Linear
project description is the authoritative design snapshot.

## Overview

`hyprpilot` is a **config-driven, fire-and-exec launcher for terminal
coding agents** — a single Rust binary. It resolves a session
*profile* from layered config, projects that profile onto the chosen
vendor's **native** CLI flags / env, optionally renames the
tmux/zellij window, and `exec()`s into the vendor CLI
(`claude` / `codex` / `opencode`) — replacing its own process. There
is **no background daemon, no unix socket, no overlay window, no
desktop UI, no in-process agent bridge**. Those were removed in the
strip refactor (K-725→734); do not reintroduce that vocabulary.

The one long-lived thing hyprpilot ships is an in-tree **MCP server**
(`hyprpilot mcp serve`) the launcher auto-injects into the vendor's
MCP config so the captain's *skills* catalogue reaches the agent over
stdio. The vendor spawns and owns that sidecar's lifetime.

## Toolchain (mise-pinned)

`mise install` at the repo root drops: `rust` (stable + `rustfmt` +
`clippy`), `task` 3 (go-task), `usage` 3, `cargo-nextest`, plus
`node` 24 + `pnpm` 10 **for the `docs/` VitePress site only** — that
is the sole Node consumer in the repo. `rust-toolchain.toml` pins the
toolchain for `cargo` invocations outside mise.

## Repo layout

Single crate at the repo **ROOT** (`Cargo.toml` + `src/`) — no nested
backend/frontend crates (`ui/`, `tests/e2e/`, and the old backend
crate dir are gone).

- `icons/` — branding assets (LFS-tracked).
- `docs/` — VitePress site; the only Node package. **`docs/` prose is
  owned by a separate issue (K-733) — do not rewrite it here.**
- `packaging/`, `.github/workflows/` — AUR packaging + CI.

Key `src/` modules:

- `main.rs` — `clap`-derive CLI. Bare invocation launches; `mcp` /
  `profiles` are the only subcommands.
- `config/` — layered load (`mod.rs`), `merge` crate strategies,
  `garde` validation, `agents.rs` (`[[agents]]` / `[profile]` /
  `[[profiles]]`), `mcp.rs` (`[mcp]`), `extensions.rs`
  (`McpFile` / `SkillEntry`), `patch.rs` (strategic merge),
  `with_config.rs` (`--with-config`), `system_prompt.rs`,
  `defaults.toml` (compiled defaults).
- `resolve/mod.rs` — pure `Config` → resolution core: profile pick,
  patch folding, per-launch MCP + skills registry construction.
- `spawn/` — `launch.rs` (`LaunchArgs` + the bare-launch entry
  `run`), `mod.rs` (orchestration), `providers.rs` (per-vendor
  native-flag projection + `exec`), `picker.rs` (interactive
  profile picker), `multiplexer.rs` (tmux/zellij rename).
- `profile.rs` — `ResolvedProfile` (flat runtime view).
- `mcp/` — MCP catalogue (`mod.rs`, `loader.rs`), `auto_inject.rs`
  (the in-tree `hyprpilot` server), `server/` (`mcp serve`).
- `skills/` — `SkillsRegistry` + `SKILL.md` loader.
- `profiles.rs` — the `profiles` subcommand.
- `logging.rs`, `paths.rs`.

## Tasks

Exactly the targets in `Taskfile.yml` — no others without updating
this file.

| Task | Purpose |
| ---- | ------- |
| `task install` | `cargo fetch` + `pnpm install` (workspace root). |
| `task run` | `cargo run -- {{.CLI_ARGS}}`. |
| `task cli` | Invoke the built debug binary (`./target/debug/hyprpilot`) with `{{.CLI_ARGS}}`. |
| `task build` | `cargo build` (debug launcher). |
| `task release` | `cargo build --release`. |
| `task test` | `task test:rust`. |
| `task test:rust` | `cargo nextest run --all-targets --no-fail-fast`. |
| `task format` | `format:rust` (`cargo fmt --all`) + `format:node`. |
| `task lint` | `lint:rust` (`cargo fmt --check` + `cargo clippy --all-targets -D warnings`) + `lint:node`. |
| `task format:node` / `task lint:node` | `pnpm -r --parallel --if-present run format` / `lint` — picks up the `docs/` package's `prettier` + `markdownlint-cli2` scripts. |
| `task docs:dev` / `docs:build` / `docs:preview` | VitePress docs site dev / build / preview. |

**Pre-push bar:** `task build && task lint && task test` all exit 0.
CI runs lint + test + build as separate jobs; any one red rejects.

## CLI surface

```sh
# Launch (bare invocation IS the launch)
hyprpilot                       # pick a profile interactively, then exec
hyprpilot engineer              # launch the `engineer` profile directly (positional)
hyprpilot engineer --cwd ~/code/foo
hyprpilot review -- --resume    # everything after `--` is forwarded verbatim

# Subcommands
hyprpilot profiles              # table of configured profiles
hyprpilot profiles --json       # machine-readable
hyprpilot mcp serve --skill-dir '{"dir":"/abs/path","ignore":[]}'
```

- **Bare launch** picks the profile via the optional positional
  `[PROFILE]` argument, falling back to an interactive `nucleo` picker
  when omitted (with `[profile] default` pre-selected under the
  cursor), then resolves and `exec()`s into the vendor CLI. Subcommands
  resolve before the positional, so a profile named `profiles`/`mcp`
  isn't positionally addressable.
- **Launch flags** (bare invocation): positional `[PROFILE]`,
  `--cwd <dir>`, `--mode`, `--with-config` / `--with-config-format`,
  and a trailing `-- <provider args>` forwarded verbatim. The profile
  is the single source of truth for its agent + model — there are **no**
  `--agent` / `--model` launch flags; use `--with-config` (e.g.
  `--with-config '@{"model":"..."}'`) for a one-off override.
- **Global flags** (every subcommand): `--config <path>`
  (`HYPRPILOT_CONFIG`), `--config-profile <name>`
  (`HYPRPILOT_CONFIG_PROFILE`), `--log-level`
  (`HYPRPILOT_LOG_LEVEL`).
- **`mcp serve`** is spawned by the agent vendor over stdio (via the
  auto-injected MCP entry), not run by hand.

## Config layering

Sources resolve in order; later layers override earlier ones for the
fields they set (`merge` crate derive).

1. **Compiled defaults** — `src/config/defaults.toml`, embedded via
   `include_str!`.
2. **Global config** — `$XDG_CONFIG_HOME/hyprpilot/config.{toml,json,yaml,yml}`
   or `--config <path>`. Extensions are searched in priority order
   (`.toml` → `.json` → `.yaml` → `.yml`); multiple coexisting files
   error at load. `--config <path>` infers format from the extension.
3. **Named config-layer profile** —
   `$XDG_CONFIG_HOME/hyprpilot/profiles/<name>.{ext}` when
   `--config-profile <name>` / `HYPRPILOT_CONFIG_PROFILE` is set. Same
   extension search + multi-format rejection. **Distinct** from the
   session `[[profiles]]` registry (addressed per-launch via the
   positional `[PROFILE]` argument).

`Config::validate()` runs `garde` after merge and fails startup with a
readable field-path error. `#[serde(deny_unknown_fields)]` + closed
enums reject typos at parse time.

**Rule:** `defaults.toml` is the **single source of truth** for
defaults. Consumers use `.expect("... seeded by defaults.toml")`
rather than duplicating fallbacks; a paired test pins every
`.expect()`-ed leaf to a seeded field.

### Merge (the `merge` crate)

Layers fold via `#[derive(merge::Merge)]` with per-field strategies:
`overwrite_some` for `Option` scalar leaves (later `Some` wins),
`merge_agents_by_id` / `merge_profiles_by_id` for the keyed
`[[agents]]` / `[[profiles]]` lists (override by `id`, append new
ids — whole-entry replace, no field-level merge inside an entry), and
`append_layers` for the root `[[patches]]` list (concatenate later
layers onto earlier — see "Root-level `[[patches]]`").

### Validation strategy (garde)

Per-type invariants live on the type (`impl garde::Validate` +
`#[garde(dive)]`). String-backed closed sets are `#[derive(Deserialize)]`
enums (`#[serde(rename_all = "...")]`) so unknown values reject at
parse time. Cross-field references use higher-order
`custom(fn(&self.sibling))` hooks; collection-level checks are free fns
via `#[garde(custom(fn))]`. `Config::validate()` wraps the garde report
in `anyhow!`.

## Agents + profiles (flattened at the TOML root)

```toml
[[agents]]                    # vendor registry (AgentsConfig, flattened)
id = "claude-code"
provider = "claude-code"      # closed AgentProvider enum
command = "claude"            # NATIVE vendor CLI the launcher exec()s
args = []                     # bare → the vendor's interactive TUI

[profile]                     # singleton: picks the default profile
default = "engineer"

[[profiles]]                  # captain-supplied session presets
id = "engineer"
agent = "claude-code"         # must reference a real [[agents]] id
model = "claude-opus-4-5"     # profile > agent > vendor default
system_prompt = [
  { file = "~/.config/hyprpilot/prompts/base.md" },
]
```

- **`[[agents]]`** (`AgentConfig`): `id`, `provider`, `model?`,
  `effort?`, `command` (mandatory native binary), `args`, `cwd?`,
  `env`. `defaults.toml` seeds the three built-ins (`claude`, `codex`,
  `opencode`, all `args = []`). There is no `[agent]` singleton.
- **`AgentProvider`** — closed enum keyed by wire name:
  `claude-code` / `codex` / `opencode` (per-vendor native-CLI
  projection). There is no generic/`custom` escape hatch — every agent
  is one of these three, so every profile gets the full native
  projection; a hand-rolled CLI declares its own `command` / `args`
  under one of these providers (or per-profile via the flat
  `command`/`args`/`env` override). New named vendor = new variant +
  a `providers.rs` `build_*` arm.
- **`[[profiles]]`** (`ProfileConfig`): `id`, `agent`, `model?`,
  `effort?`, `system_prompt?` (array of `{ file, inject? }`), `mcps?`
  (per-profile MCP catalogue), `mcp?` (per-profile `[mcp]` block),
  `mode?`, `cwd?`, `headless?` (force non-interactive launch — see
  Headless below), `env`. **At least one entry is required** —
  `validate_profiles_non_empty` rejects an empty list at load.
  `defaults.toml` seeds **zero** profiles (captains supply their own,
  so the profile list is never polluted with a default-pretender).
- **Profile override surface:** at resolve time the profile's
  `model` / `effort` / `mode` / `cwd` override the agent entry (profile
  is the more specific scope). The profile is the single source of
  truth for its agent + model — there is no per-launch `--agent` /
  `--model` override; `--with-config` is the ad-hoc escape hatch for a
  one-off swap. A `[[profiles]]` entry also carries flat
  top-level `command: Option<String>`, `args: Option<Vec<String>>`,
  and `env: BTreeMap<String, String>` fields — no nested override
  block. When set, `command` / `args` each REPLACE the base agent's
  value wholesale (no append/merge — flags have no stable key to
  merge by); `env` OVERLAYS the base agent's `env` per-key, with the
  profile's key winning on collision and absent keys left untouched.

### Resolution (single source of truth)

`resolve::resolve_effective_profile` / `resolve_into_instance_and_profile`
are the one path every spawn flows through so the `ResolvedProfile`
and the MCP / skills registries can't drift:

1. Pick the base profile: the positional `[PROFILE]` id first, then
   `[profile] default`; **error** when neither addresses a real
   `[[profiles]]` entry — there is no bare-agent fallback.
2. Fold root `[[patches]]` (filtered by each patch's optional
   `$match.profile` glob).
3. Fold `--with-config` patches in declaration order.
4. Deserialize the merged `Value` back into `ProfileConfig` + re-run
   `garde`.

Resolution flows purely profile → patches → `--with-config`; there is
no post-hoc agent/model override applied on top.

## Root-level `[[patches]]`

`patches: Option<Vec<serde_json::Value>>` — partial `ProfileConfig`
overlays folded onto whichever profile is picked. An optional `$match`
sibling (`$match.profile = "<glob>"`, `globset` — crosses `/`) filters
where a patch applies and is stripped before merge. Folding uses the
`config::patch::merge_values` strategic engine (object-field merge,
`$patch: replace`, keyed-array merge by `id`, primitive-array
append+dedupe, `$deleteFromPrimitiveList/<field>` removal) — the same
engine `--with-config` uses. This is the
single mechanism for profile-shared knobs; there is **no** root-level
`system_prompt` / `mcps` / `mcp` field.

**Additive across config layers** (`append_layers` merge strategy, not
`overwrite_some`): the `patches` list **concatenates** across layers
(defaults → global config → config-profile) in declaration order —
earlier-layer patches first, then later. A user config layer's
`[[patches]]` **extends** the compiled-default seed rather than
replacing the whole list; the seeded skills-dir patch always survives.
Because the resolve-time fold applies every patch in order, a later
patch still overrides or wipes an earlier one's fields — captains
express replacement/deletion with `$patch: replace` (or
`$deleteFromPrimitiveList/<field>`) **inside a patch body**, never by
clobbering the layer list. This closes the footgun where a partial
`[patches.mcp]` in a user layer silently dropped the seeded skills dir.

`defaults.toml` seeds one unscoped patch pointing the in-tree
`hyprpilot` MCP server at the XDG skills dir. The seed carries **only**
`mcp.skills` (the load-bearing value that must survive layer merge);
`enabled = true` / `autoAcceptTools = ["*"]` / `autoRejectTools = []`
are the typed `McpConfig::default()` the resolver backfills per-leaf in
`resolve::effective_mcp_with`, so those values are single-sourced in
Rust — not duplicated in `defaults.toml`.

```toml
[[patches]]
"$match" = { profile = "work/*" }   # unset $match applies to every profile
[[patches.mcps]]
file = "~/.config/hyprpilot/mcps/work.json"
```

## `--with-config`

Repeatable overlay flag on the launch path. Each value is a file path
(extension drives format), `@<inline body>`, or `-` (stdin, usable
**at most once**). `--with-config-format toml|json|yaml` (default
`json`) drives stdin / inline / extension-less inputs. Values parse to
`serde_json::Value` and fold **after** root `[[patches]]`. `$patch`
directive semantics live in `config/patch.rs`.

## MCP catalogue (`mcps`) + tool policy

`McpFile` entries carry **either** a `file` path **or** an inline
`mcp_servers` map (exactly one; garde rejects both/neither). File
paths follow the standard `{ "mcpServers": { … } }` shape used by
Claude Code / Codex / Cursor; hyprpilot extends each server entry with
an optional `hyprpilot` namespace key:

```json
{ "mcpServers": { "filesystem": {
  "command": "npx", "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
  "hyprpilot": {
    "includeTools": ["read_*"], "excludeTools": ["delete_*"],
    "autoAcceptTools": ["read_*"], "autoRejectTools": ["delete_*"]
  }
}}}
```

- **Per-profile override:** `[[profiles]] mcps = [...]` wholesale-
  replaces; `mcps = []` is the explicit off-switch. Root `[[patches]]`
  `mcps` merge onto the picked profile.
- **Merge:** files iterate in order, later wins on name collision; one
  malformed file warns + skips. Everything except the typed
  `hyprpilot` block stays opaque `serde_json::Value`.
- **Tool policy:** `includeTools` / `excludeTools` (visibility) +
  `autoAcceptTools` / `autoRejectTools` (approval). Globs are
  **server-relative** (`read_*`, prefix `mcp__<server>__` implicit).
  Exclude beats include; reject beats accept. `[mcp].auto_accept_tools`
  (default `["*"]`) is copied onto servers with no per-server override.
- **Vendor projection** (`spawn/providers.rs`): claude gets
  `--mcp-config` (inline JSON) + `--allowedTools` / `--disallowedTools`;
  codex gets `-c mcp_servers.<name>.*` config overrides; opencode gets
  `OPENCODE_CONFIG_CONTENT` + `OPENCODE_PERMISSION` env. Transport is
  inferred by field presence (`command` → stdio, `url` → http/sse).
  Provider args the captain passed suppress the generated equivalents.
  Reserved name `hyprpilot` — see auto-inject below.

## Skills + the in-tree MCP server (the skills channel)

Skills reach the agent **only** through the hyprpilot MCP server.

- **`[mcp]` block** (`McpConfig`): `enabled` (default `true`),
  `skills` (`Vec<SkillEntry { dir, ignore }>`), `autoAcceptTools`
  (`["*"]`), `autoRejectTools` (`[]`). Per-profile `[profiles.mcp]`
  wholesale-replaces the global; folded via patches. `[mcp].skills`
  default seed is `~/.config/hyprpilot/skills`.
- Each skill root is a flat directory of `<slug>/SKILL.md` bundles
  plus an optional per-root `ignore` glob list. `SkillsRegistry`
  scans + first-slug-wins on collision; missing roots warn + skip.
- **Auto-inject** (`resolve::build_mcp_registry_with` +
  `mcp::auto_inject`): when `[mcp].enabled` **and** the resolved
  skills registry is non-empty, the launcher prepends a stdio MCP
  entry named **`hyprpilot`** that spawns
  `hyprpilot mcp serve --skill-dir <json> …` (one `--skill-dir` per
  root, each carrying that root's ignore list as JSON). The reserved
  `hyprpilot` name replaces any same-named configured server.
  Auto-inject is independent of `mcps` — `mcps = []` does not suppress
  it; `[mcp].enabled = false` does.
- **`hyprpilot mcp serve`** (`mcp/server/serve.rs`): an `rmcp` stdio
  server. Resources: `hyprpilot://skills/<slug>` (body) and
  `.../references` (bundled frontmatter references). Tools:
  `list_skills`, `read_skill`, `load_skill_references`, `reload`
  (rescan dirs), `open` (OS-default handler via the `open` crate).
  Skills are discovered by directory scan — the same
  `SkillsRegistry` discovery the launcher uses — so editing a skill
  and calling `reload` refreshes without restarting the session.

## Launch / exec (`spawn`)

`spawn::launch_profile`: resolve the profile → build per-launch skills
+ MCP registries → `providers::build_command` (per-vendor native-flag
projection) → optional multiplexer rename → `providers::exec`. On unix
`exec()` **replaces** the process (no child); non-unix falls back to
spawn + propagate exit code. Model precedence is profile > agent >
vendor default. `system_prompt` files are read at **resolve** time so
a missing file fails loudly on the next launch. CLI `--cwd` / `--mode`
override the resolved profile after profile resolution (there is no
`--model` / `--agent` flag — use `--with-config`).

### Headless / stdin pass-through

Effective headless = `profile.headless == true` **OR** stdin is piped
(`!std::io::stdin().is_terminal()`). `headless: Option<bool>` on
`ProfileConfig` threads into `ResolvedProfile.headless`. When headless
is active AND hyprpilot generates the projection (**no** trailing `--
<provider args>`), `spawn::headless_prompt` buffers **all** of stdin
into a `String` and passes it as the vendor's prompt ARGUMENT — else it
returns `None` (interactive, or the escape-hatch path). Headless + a
TTY (no pipe) → error; empty piped prompt → error. **Profile
selection:** a headless launch never opens the picker
(`select_profile_without_positional`) — piped stdin OR a
`headless`-flagged `[profile] default` resolves the default directly,
erroring when no default is set; only an interactive TTY with a
non-headless default falls through to the picker. Per-vendor
projection (`providers.rs`, driven by `prompt: Option<&str>` on
`build_command` / `build_*`): claude `--print` + prompt positional;
codex `exec` subcommand + prompt positional (approval-policy `mode`
dropped — `codex exec` has no `--ask-for-approval`; sandbox modes still
project via `-s`); opencode `run` subcommand + prompt positional. All
share the interactive model/effort/mode/MCP/tool-policy projection +
arg-dedup — headless only changes prompt DELIVERY. **exec, not spawn**:
hyprpilot consumes stdin before `exec()`, so fd0 is at EOF and `codex
exec` does not block on it (verified live). **Escape hatch:** trailing
`-- <provider args>` (non-empty `provider_args`) makes hyprpilot skip
stdin entirely — fd0 stays inherited so the vendor gets the raw pipe as
input data, and the existing dedup suppresses the generated projection.

### Multiplexer title

`[multiplexer] set_title = true` (default, seeded by defaults.toml)
renames the current tmux window / zellij tab to
`hyprpilot@<cwd-basename>` right before `exec()`, via
`tmux rename-window` / `zellij action rename-tab` (shell-out, **not**
OSC escapes — those are gated by tmux/zellij settings). Best-effort:
every failure logs at `debug` and never aborts the launch; no-op
outside a multiplexer regardless of the flag.

## Logging

`tracing` bootstrapped via `logging::init`. **Always writes to
stderr** (debug + release), ANSI on. `LogLevel` is a closed enum
shared by the `--log-level` clap flag and the `[logging] level` config
field. Filter precedence: `--log-level` → `RUST_LOG` → `info`. The
launcher `exec()`s into the vendor, so hyprpilot's own tracing only
covers the brief resolve phase before hand-off.

## Paths (`paths.rs`)

XDG config dir resolution, config-file discovery (`CONFIG_EXTENSIONS`
priority `toml > json > yaml > yml`; multiple matches error),
`resolve_user` (`~` / `$VAR` expansion via `shellexpand`, then
`path-absolutize` for `./` / `../` collapse + cwd-relative joins).

## Rust conventions

- **Enums whenever feasible — never `String` for a closed set.** If a
  value can only be one of N known things at compile time, it is a
  `#[derive(...)] enum` with `#[serde(rename_all = "...")]` for wire
  types (agent provider, log level, config format, dimension flavour),
  NOT a `String`. **Why:** exhaustive matches, parse-time rejection of
  unknown values, mechanical renames. Free-form `String` is reserved
  for user-supplied content (ids, paths, prompts).
- **No backwards-compatibility layers — ever.** CLI, config file, and
  the theme/agent tree evolve in lockstep with the binary. Delete a
  design that stops making sense and rewire the call sites; no
  typed-shim enums or "legacy" wrappers.
- **Fix the root cause, not the symptom.** If one code path can handle
  every case, don't branch; if one type can hold the state, don't
  split it. Treat every "add a flag for this edge case" as a signal to
  find the right shape.
- **Stubs panic, they don't pretend** (`unimplemented!("<verb>: <why>")`).
- **Inline single-use helpers.** Prefer `fn main() -> Result<()>` over
  a `try_main` wrapper.
- **Compose behavior onto the owning type, not free fns.** Helpers
  that touch a primary type's state go as methods; free fns are for
  pure transformations.
- **Structs carry their invariants; don't re-pass context on every
  call** (e.g. a `Sandbox { root }` canonicalises once at
  construction).
- **Enum + match dispatch for families of related handlers**; reach
  for macros only when monomorphisation forces per-handler registration.
- **Traits for open extension points; closed enums for closed sets.**
- **Comment discipline — terse WHY, never WHAT.** Default to no
  comments.
- **Multiline fixtures use raw strings** (`r#"..."#`).
- **Config structs** use `#[derive(Debug, Clone, Default, Deserialize,
  Serialize, PartialEq)]` (+ `Validate` / `Merge` where relevant) with
  `#[serde(default, deny_unknown_fields)]`; leaves are `Option<T>` so
  partial user configs merge.
- **Tests** live next to their module.

## YAML conventions

**Block style only — never JSON-like flow mappings** in any YAML the
repo ships:

```yaml
# right
- uses: actions/checkout@v6
  with:
    lfs: true
```

GitHub Actions `${{ … }}` expressions stay as-is (string substitution,
not YAML structure).

## Workflow

- `.mcp.json` at the repo root is the repo-scoped MCP server registry.
  Add servers you need during a task; remove non-load-bearing ones at
  merge.
- Every issue is picked up on a dedicated branch — **never implement on
  `main` or `beta` directly.** PRs target **`beta`**, never `main`.
- **Feature changes and feature additions MUST include documentation
  updates (`docs/` + `CLAUDE.md`) in the same PR.** A user-observable
  behavior change that ships without the matching docs edit is
  incomplete — the doc update is part of the change, not a follow-up.
- Commit style: conventional commits with a `refs K-<id>` /
  `closes K-<id>` trailer.
- Prefer the MCP tools over CLIs for git / GitLab / Linear / GitHub;
  fall back to CLI only when the MCP server lacks the operation.

## Manual verification patterns

`task build && task lint && task test` is the automated bar. Beyond
that, **every runtime-behavior change lands with a smoke-test block in
its PR** — concrete commands + literal observed output.

Baseline smokes:

- `task build` produces `target/debug/hyprpilot`.
- `hyprpilot --help`, `hyprpilot profiles --help`,
  `hyprpilot mcp serve --help` render via clap.
- `hyprpilot profiles` lists configured profiles (empty config →
  validation error naming the empty `[[profiles]]` list).
- A deliberately broken `config.toml` aborts with a readable garde
  error naming the field path.
- Partial config overrides compose: setting one nested field keeps
  every sibling falling through to `defaults.toml`.
- `hyprpilot <id>` (positional profile) resolves the profile and
  `exec()`s the vendor
  CLI. Verify without an external agent by pointing a real-provider
  agent's `command` at a stub on `$PATH` — e.g. `[[agents]] provider =
  "claude-code"`, `command = "echo"` — so the launch execs the stub and
  the projected argv is observable. (`provider` must still be one of
  `claude-code` / `codex` / `opencode`; there is no `custom` provider.)

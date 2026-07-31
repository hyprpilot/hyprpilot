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

The one long-lived thing hyprpilot ships is a set of in-tree **MCP
servers** the launcher auto-injects into the vendor's MCP config —
`mcp serve` (general tools), `mcp skills` (the captain's *skills*
catalogue), `mcp harness` (agent sessions). One subcommand, one
process, one catalogue entry each. The vendor spawns and owns those
sidecars' lifetimes.

**One deliberate exception to "no in-process bridge":** the opt-in
`mcp harness` sidecar owns agent sessions it
spawned, in an in-process table (see "The agent harness"). That is not
the daemon the strip refactor removed — the vendor still owns the
sidecar's lifetime, nothing survives it, and no socket or control plane
exists. The launcher itself is unchanged: still fire-and-exec, and the
harness stays off unless `[mcp.harness].enabled` says otherwise.

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
  `run`), `mod.rs` (orchestration), `providers/` (per-vendor
  native-flag projection + `exec`: `mod.rs` = dispatch / `exec` /
  `base_command` / redaction / shared helpers, `argv.rs` =
  flag-detection, `claude.rs` / `codex.rs` / `opencode.rs` = the
  three vendor builders, `temp.rs` = the 0600 temp-config lifecycle +
  reaper), `picker.rs` (interactive profile picker), `multiplexer.rs`
  (tmux/zellij rename).
- `profile.rs` — `ResolvedProfile` (flat runtime view).
- `mcp/` — MCP catalogue (`mod.rs`, `loader.rs`), `auto_inject.rs`
  (one builder per in-tree server), `server/` = the three servers,
  one `ServerHandler` each: `tools.rs` (`mcp serve` — `open`;
  stateless), `serve.rs` (`mcp skills` — protocol + skills tools;
  also owns the shared schema/result helpers), `harness_server.rs`
  (`mcp harness` — protocol + tool dispatch) over `harness.rs` (the
  session-driving logic) and `sessions/` (the owned-session store).
  `skills/` = metadata + references.
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
hyprpilot mcp serve             # general tools (`open`)
hyprpilot mcp skills --skill-dir '{"dir":"/abs/path","ignore":[]}'
hyprpilot mcp harness --max-sessions 64
```

- **Bare launch** picks the profile via the optional positional
  `[PROFILE]` argument, falling back to an interactive `nucleo` picker
  when omitted (with `[profile] default` pre-selected under the
  cursor), then resolves and `exec()`s into the vendor CLI. Subcommands
  resolve before the positional, so a profile named `profiles`/`mcp`
  isn't positionally addressable.
- **Launch flags** (bare invocation): positional `[PROFILE]`,
  `-p/--prompt <PROMPT>` (inline headless prompt), `-f/--file <PATH>`
  (headless prompt read from a file; `conflicts_with` `--prompt`),
  `--cwd <dir>`, `--mode`, `--with-config` / `--with-config-format`,
  and a trailing `-- <provider args>` forwarded verbatim. The profile
  is the single source of truth for its agent + model — there are **no**
  `--agent` / `--model` launch flags; use `--with-config` (e.g.
  `--with-config '@{"model":"..."}'`) for a one-off override. `-p` is
  free (K-747 made the profile a positional, so `-p` no longer means
  `--profile`).
- **Global flags** (every subcommand): `--config <path>`
  (`HYPRPILOT_CONFIG`), `--config-profile <name>`
  (`HYPRPILOT_CONFIG_PROFILE`), `--log-level`
  (`HYPRPILOT_LOG_LEVEL`).
- **The `mcp` servers** are spawned by the agent vendor over stdio
  (via the auto-injected MCP entries), not run by hand.

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
  a `providers/<vendor>.rs` `build_*` arm.
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

`defaults.toml` seeds one unscoped patch pointing the skills server at
the XDG skills dir. The seed carries **only** `mcp.skills.dirs` (the
load-bearing value that must survive layer merge);
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
- **Vendor projection** (`spawn/providers/`): claude gets
  `--mcp-config` (inline JSON) + `--allowedTools` / `--disallowedTools`;
  codex gets `-c mcp_servers.<name>.*` config overrides; opencode gets
  `OPENCODE_CONFIG_CONTENT` + `OPENCODE_PERMISSION` env. Transport is
  inferred by field presence (`command` → stdio, `url` → http/sse).
  Provider args the captain passed suppress the generated equivalents.
  Reserved names: each in-tree server's resolved name — see
  auto-inject below.

## The three in-tree MCP servers

One subcommand, one process, one `ServerHandler`, one catalogue entry
each. The split is the GATE: the skills server cannot serve `spawn`
because it does not implement it. An earlier design hung the harness
off the skills server behind `--with-harness`, which meant gating both
`list_tools` and `call_tool` by name — and a reviewer caught one half
missing. Do not re-merge them.

| Subcommand | Default name | Module | Serves | Default |
| ---------- | ------------ | ------ | ------ | ------- |
| `mcp serve` | `hyprpilot` | `server/tools.rs` | `open` | on |
| `mcp skills` | `hyprpilot_skills` | `server/serve.rs` | skills tools + resources | on |
| `mcp harness` | `hyprpilot_harness` | `server/harness_server.rs` | `list_profiles` / `spawn` / `session_*` | **off** |

`serve.rs` also owns the helpers the other two import
(`object_schema`, `structured_with_text`, `tool_error`,
`require_string`, `wait_for_shutdown`).

Skills reach the agent **only** through the skills server.

- **`[mcp]` block** (`McpConfig`): `enabled` (default `true` — the
  MASTER gate over all three servers), `serve` / `skills` / `harness`
  (per-server blocks), `autoAcceptTools` (`["*"]`), `autoRejectTools`
  (`[]`). Per-profile `[profiles.mcp]` wholesale-replaces the global;
  folded via patches.
- **Per-server blocks** each carry `enabled`, `name`,
  `autoAcceptTools`, `autoRejectTools`, plus their own fields:
  `[mcp.skills].dirs` (`Vec<SkillEntry { dir, ignore }>`, default
  seed `~/.config/hyprpilot/skills`) and `[mcp.harness].maxSessions`.
  A per-server tool-policy glob list OVERRIDES the `[mcp]`-level one
  rather than merging. `[mcp.harness].enabled` defaults to **false**
  and that is a security property, not a preference — a profile's
  `command` is an arbitrary binary, so `spawn` executes as the user.
- Each skill root is a flat directory of `<slug>/SKILL.md` bundles
  plus an optional per-root `ignore` glob list. `SkillsRegistry`
  scans + first-slug-wins on collision; missing roots warn + skip.
- **Auto-inject** (`resolve::build_mcp_registry_with` +
  `mcp::auto_inject`, one `build_*_definition` per server): under the
  `[mcp].enabled` master gate, each server injects a stdio entry when
  its own block is enabled. The reserved name replaces any same-named
  configured server. Auto-inject is independent of `mcps` — `mcps = []`
  does not suppress it. **Skills is the only one also gated on
  content**: an empty registry means nothing to serve, so nothing is
  injected. Its entry spawns `hyprpilot mcp skills --skill-dir <json> …`
  (one `--skill-dir` per root, each carrying that root's ignore list as
  JSON).
- **`hyprpilot mcp skills`** (`mcp/server/serve.rs`): an `rmcp` stdio
  server. Resources: `hyprpilot://skills/<slug>` (body) and
  `hyprpilot://references/<slug>` (bundled frontmatter references — a
  parallel top-level scheme, NOT a `/references` segment nested under
  the slug; the nested form broke client URI autocomplete). Tools:
  `list_skills`, `read_skill`, `load_skill_references`, `reload`
  (rescan dirs). Skills are discovered by directory scan — the same
  `SkillsRegistry` discovery the launcher uses — so editing a skill
  and calling `reload` refreshes without restarting the session.
  **Every tool result carries BOTH a `content` text block AND
  `structured_content`** (`serve::structured_with_text` overwrites
  `CallToolResult::structured`'s `.content` with an explicit readable
  summary — `list_skills` a one-line-per-skill catalogue, `read_skill`
  the body): clients that render only `content` (opencode) show the
  text instead of "Unknown"; structured-aware clients (Claude Code)
  still get the JSON. A structured-only result renders as "Unknown" in
  opencode — never return one.

## The agent harness (`mcp harness`)

`hyprpilot mcp harness` serves six tools that let a connected agent
drive hyprpilot profiles: `list_profiles` (discovery), `spawn`,
`session_send`, `session_list`, `session_read`, `session_kill`.
`[mcp.harness].enabled` defaults to false.

- **The gate bounds DISCOVERY, not capability — do not overstate it.**
  A profile's `command` is an arbitrary binary, so anything that
  reaches `spawn` executes commands as this user. `[mcp.harness]
  .enabled` decides only whether the launcher AUTO-INJECTS the entry:
  `hyprpilot mcp harness` is an ordinary subcommand and serves `spawn`
  whenever invoked, regardless of config (deliberate — a hand-written
  MCP entry must work without a config to consult, and the `mcp` branch
  skips validation so a broken config can't kill a respawned sidecar).
  Against an agent with shell access it buys nothing; it is a real
  boundary only for an MCP-only client. What the split DOES guarantee
  is structural within the served surface: the skills server cannot
  serve `spawn` because it does not implement it. (It used to be a name
  check inside a shared server, which had to cover `call_tool` as well
  as `list_tools` because dispatch is by name; that is exactly the
  failure mode the split removes.) `HYPRPILOT_SPAWN_DEPTH` bounds
  nesting; a session-count ceiling bounds breadth.
- **Enabling the harness inherits `autoAcceptTools = ["*"]`** from the
  `[mcp]` level unless `[mcp.harness].autoAcceptTools` is set — so
  `spawn` lands auto-approved. The per-server policy the split bought
  is only worth something if it is actually used; a test pins this
  default so tightening it stays a deliberate change.
- **The sidecar OWNS every session** (`mcp/server/sessions/`). Sessions
  are direct children waited on via `tokio::process::Child`, so exit
  codes are recoverable, no zombie defeats a liveness check, and no PID
  is reused underneath us. They **die with the sidecar** and do not
  survive a restart — that trade buys the correctness above.
- **Orphan prevention is layered; only the last layer is a guarantee.**
  `async shutdown()` (SIGTERM → grace → SIGKILL, raced against
  SIGTERM/SIGHUP handlers) and `kill_on_drop(true)` are userspace
  courtesy — without `kill_on_drop` tokio *orphans* a live child on drop
  rather than killing it. **`PR_SET_PDEATHSIG`** is the real guarantee:
  the kernel kills the child even under `SIGKILL` or the release
  profile's `panic = "abort"`, both of which run no destructor. Linux
  only; elsewhere it degrades to the userspace paths. Sessions run in
  their **own process group** so a kill reaches the vendor's own MCP
  sidecars and tool subprocesses. A **startup sweep** reclaims what a
  crashed predecessor left behind, skipping any directory whose owning
  sidecar is still alive (two sidecars at once is ordinary).
- **A conversation REPLAYS its launch.** `session_send` carries the
  original `cwd` / `mode` / `with_config` / `args` forward from the
  spawn (recorded as `sessions::LaunchShape`); an explicit per-turn
  value overrides. How a conversation was launched is part of its
  IDENTITY, not a per-turn option — claude keys its conversation store
  by project directory, so a resume from a different cwd failed with a
  bare "No conversation found with session ID" for a healthy session.
- **cwd reaches each vendor differently.** claude inherits the process
  cwd; codex takes `--cd`; opencode takes `--dir`. hyprpilot sets
  `current_dir` AND emits the flag for the two that need one —
  opencode does not derive its tool sandbox from the process cwd, so
  without `--dir` the agent silently worked in the wrong tree while
  every surface reported the requested path.
- **`[profiles.harness]` is OPT-IN.** A profile with no block is not
  available: absent from `list_profiles` AND refused by
  `spawn`/`session_send`. Within a declared block `enabled` defaults
  true; it is the block's ABSENCE that keeps a profile off. Both halves
  matter — `launch` is the shared body of both tools, so one check
  covers them; gating only the listing would leave it reachable by id.
  An unknown id stays "allowed" so the resolver keeps owning that error.
- **A conversation is ONE session.** `session_send` reuses its handle and
  appends to the same transcript, so an N-turn conversation costs one
  table entry, not N. Its check-and-spawn happens under the table lock —
  `Command::spawn` is synchronous, so "one turn at a time" is an
  invariant, not a racy check.
- **Bounded retention.** `--max-sessions` (default 64) evicts the oldest
  *finished* sessions; a running one is never touched. `session_kill` is
  state-aware — it terminates a running session (keeping the transcript)
  and reaps an already-finished one.
- **`with_config` is an ALLOW-list** (`model` / `effort` / `mode`). A
  deny-list of `command`/`args`/`env` was not the reachable surface:
  `mcps` carries inline `mcp_servers` with their own `command`, and
  `$deleteFromPrimitiveList/<field>` reaches a field without naming it.
- **One resolution path.** Every harness launch goes through
  `spawn::prepare` — the same function `hyprpilot <profile>` uses — so
  prompt-source priority, the `-- <args>` escape hatch, and cwd
  precedence cannot drift between the CLI and MCP. `LaunchOrigin`
  carries the differences: the harness never reads real stdin (the
  sidecar's fd0 **is** the MCP transport), never opens the picker, and
  never renames the multiplexer window.
- **Harness-only JSON projection.** claude gets
  `--output-format stream-json` plus a **mandatory** `--verbose` (claude
  refuses the launch without it), codex `--json`, opencode
  `--format json`. The CLI's `-p` output stays human-readable. Vendors
  report their session id under three different keys — `session_id`,
  `thread_id`, `sessionID` — all verified against the installed CLIs.
- **Streaming** rides `notifications/progress` when the caller supplies
  a progressToken; a follow ends on session exit, client cancellation,
  or a caller-set limit. MCP tool results are single-shot, so the result
  still carries everything the notifications streamed.

- **Skill metadata — ONE block, spec fields canonical**
  (`mcp/server/skills/metadata.rs`): the MCP spec's `_meta` is a single
  field keyed by reverse-DNS names, so every skill surface carries
  exactly ONE namespaced key — **`io.hyprpilot/skill`** in resource
  `_meta`, **`metadata`** in tool output (`list_skills` / `read_skill` /
  `load_skill_references`) — and nothing in it repeats a spec-compliant
  `Resource` field. The block = the WHOLE frontmatter map **verbatim**
  (`skill_block`) **minus** `title` + `description` (byte-for-byte equal
  to `Resource.title` / `Resource.description`) **plus** the
  runtime-derived `path` + `bundleDir` (not in the frontmatter).
  Frontmatter `name` is **kept** — `Resource.name` is the SLUG, an
  author's frontmatter `name` may differ, so it's not a spec duplicate.
  Only `title`/`description` are dropped. There is **no**
  `io.hyprpilot/frontmatter` key and **no** curated camelCase
  re-projection anymore — a new/custom frontmatter key rides through
  the one block losslessly. `list_skills` keeps the headline
  `slug`/`title`/`description`/`uri` scan view alongside the single
  `metadata` block. Built ONCE per skill into `LoadedSkill.meta_block`,
  not per request.

## Launch / exec (`spawn`)

`spawn::launch_profile`: resolve the profile → build per-launch skills
+ MCP registries → `providers::build_command` (per-vendor native-flag
projection) → optional multiplexer rename → `providers::exec`. On unix
`exec()` **replaces** the process (no child); non-unix falls back to
spawn + propagate exit code. **Exception:** when `SpawnCommand.
stdin_prompt` is `Some` (claude/codex headless) `exec` SPAWNS instead,
writes the prompt to the child's stdin, closes it, and propagates the
exit code — see Headless below. Model precedence is profile > agent >
vendor default. `system_prompt` files are read at **resolve** time so
a missing file fails loudly on the next launch. CLI `--cwd` / `--mode`
override the resolved profile after profile resolution (there is no
`--model` / `--agent` flag — use `--with-config`).

### Headless / prompt delivery

Effective headless = `--prompt`/`--file` given **OR**
`profile.headless == true` **OR** stdin is piped
(`!std::io::stdin().is_terminal()`). `headless: Option<bool>` on
`ProfileConfig` threads into `ResolvedProfile.headless`.
`spawn::headless_prompt(prompt_override, …)` resolves the prompt: an
explicit `--prompt`/`--file` value (`prompt_override`, resolved in
`LaunchArgs::prompt_override` — `--file` read via `paths::resolve_user`,
read error surfaced cleanly) is delivered FIRST — **even alongside**
trailing `-- <provider args>`, so `-p`/`-f` COMPOSE with the escape
hatch (prompt delivered + extra flags forwarded); else the escape hatch
(trailing `-- <provider args>` with no `--prompt`/`--file`) → `None`;
else, when effective, piped stdin wins and **all** of stdin is
buffered. Headless + a TTY with no
`--prompt`/`--file`/pipe → error; empty prompt → error. `--prompt` and
`--file` are `conflicts_with` at the clap layer. **Profile selection:**
a headless launch never opens the picker
(`select_profile_without_positional`, which takes `prompt_given`) —
piped stdin, `--prompt`/`--file`, OR a `headless`-flagged
`[profile] default` resolves the default directly, erroring when no
default is set; only an interactive TTY with a non-headless default and
no prompt flag falls through to the picker.

**Prompt DELIVERY per vendor** (driven by `prompt: Option<&str>` on
`build_command` / `build_*`, which set `SpawnCommand.stdin_prompt`):

- **claude** — `--print`, prompt on **stdin**. NOT a positional:
  claude's `--allowedTools`/`--disallowedTools` are variadic
  (`<tools...>`) and would swallow a trailing operand as a tool entry,
  so a positional prompt never reaches the model (this was the 3.0.0
  bug). `stdin_prompt = Some(prompt)`.
- **codex** — `exec` subcommand, prompt on **stdin** (approval-policy
  `mode` dropped — `codex exec` has no `--ask-for-approval`; sandbox
  modes still project via `-s`). `stdin_prompt = Some(prompt)`.
- **opencode** — `run` subcommand + prompt POSITIONAL (opencode's
  `run [message…]` is positional-only; no stdin support).

All share the interactive model/effort/mode/MCP/tool-policy projection
+ arg-dedup — headless only changes prompt DELIVERY.

**spawn vs exec** (`providers::exec`): when `stdin_prompt` is `Some`
(claude/codex headless) the launcher SPAWNS the vendor, writes the
prompt to the child's stdin, closes it (EOF), and propagates the exit
code — NOT `exec()`. stdout/stderr stay inherited so the child can
always drain them (no deadlock against the blocking stdin write). The
EOF close is what keeps `codex exec` from hanging on an idle pipe
(openai/codex#20919). Interactive and opencode-headless keep the unix
`exec()` handoff. **Escape hatch:** trailing `-- <provider args>`
(non-empty `provider_args`) **with no explicit `--prompt`/`--file`**
makes hyprpilot skip stdin entirely — fd0 stays inherited so the vendor
gets the raw pipe as input data, and the existing dedup suppresses the
generated projection. An explicit `--prompt`/`--file` overrides this: it
COMPOSES with the escape hatch — the prompt rides its usual delivery
path (stdin for claude/codex, positional for opencode) while the
`-- <args>` still append to argv (dedup lets a hand-passed flag suppress
the generated equivalent).

### Multiplexer title

`[multiplexer] set_title = true` (default, seeded by defaults.toml)
renames the current tmux window / zellij tab to
`hyprpilot@<cwd-basename>` right before `exec()`, via
`tmux rename-window` / `zellij action rename-tab` (shell-out, **not**
OSC escapes — those are gated by tmux/zellij settings). Best-effort:
every failure logs at `debug` and never aborts the launch; no-op
outside a multiplexer regardless of the flag.

**Skip conditions** (`multiplexer::title_rename_skip_reason`, checked
before `Multiplexer::detect`): the rename runs only when `set_title !=
false` AND `HYPRPILOT_NO_TITLE` is unset/falsey AND not under an editor
— any one skips (debug-logged with the reason). `HYPRPILOT_NO_TITLE`
(truthy = `1`/`true`/any non-empty ≠ `0`/`false`) is the authoritative
launcher override — used when the caller (e.g. `sidekick.nvim`'s
per-tool `env` block) owns the pane; it can't route through
`--with-config` because `[multiplexer]` is a root field, not a profile
field. Editor auto-detect keys off env markers (`NVIM` /
`NVIM_LISTEN_ADDRESS` / `INSIDE_EMACS` / `VSCODE_PID` /
`TERM_PROGRAM=vscode` / `VIM`) so it also "just works" under nvim
without the env var.

## Logging

`tracing` bootstrapped via `logging::init(cli_level, config_level)`.
**Always writes to stderr** (debug + release), ANSI on. `LogLevel` is a
closed enum shared by the `--log-level` clap flag and the
`[logging] level` config field. Filter precedence: `--log-level` →
`RUST_LOG` → `[logging] level` → the **`error`** default (a fresh run
is quiet — errors only). `main` loads the config QUIETLY (before any
subscriber exists), then `init` installs the subscriber ONCE with the
fully-resolved filter, then emits the `config: loaded` line — so it (and
every info line) honors `[logging] level`/`--log-level error`. No
early-init/late-reload dance (the old `apply_config_level` + reload
handle are gone). `defaults.toml` does NOT seed a level — the code
fallback owns the default (seeding would re-nullify the scoped
`[logging] level` filter, K-750). The launcher `exec()`s into the
vendor, so hyprpilot's own tracing only covers the brief resolve phase
before hand-off.

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
  `main` directly.** PRs target **`main`**. `beta` was the 3.0 staging
  branch; it merged into `main` (squashed, #192) and has had no commits
  since 2026-07-23. It is retired — do NOT target it. The 29 commits it
  carries that `main` lacks are the pre-squash originals of that merge,
  not unmerged work.
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
- `hyprpilot --help`, `hyprpilot profiles --help`, and
  `hyprpilot mcp {serve,skills,harness} --help` render via clap.
- Each `mcp` subcommand answers `initialize` + `tools/list` over stdio
  and reports the right `serverInfo.name` (`hyprpilot` /
  `hyprpilot_skills` / `hyprpilot_harness`) and tool set.
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

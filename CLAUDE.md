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
- `resolve/` — pure `Config` → resolution core: `mod.rs` (profile
  pick, patch folding, per-launch MCP + skills registry construction),
  `profile.rs` (`ResolvedProfile`, the flat runtime view it produces).
- `spawn/` — `launch.rs` (`LaunchArgs` + the bare-launch entry
  `run`), `mod.rs` (orchestration), `providers/` (per-vendor
  native-flag projection + `exec`: `mod.rs` = dispatch / `exec` /
  `base_command` / redaction / shared helpers, `argv.rs` =
  flag-detection, `claude.rs` / `codex.rs` / `opencode.rs` = the
  three vendor builders, `temp.rs` = the 0600 temp-config lifecycle +
  reaper), `picker.rs` (interactive profile picker), `multiplexer.rs`
  (tmux/zellij rename).
- `mcp/` — MCP catalogue (`mod.rs`, `loader.rs`), `auto_inject.rs`
  (one builder per in-tree server), `server/` = the three servers,
  one `ServerHandler` each: `tools.rs` (`mcp serve` — `open`;
  stateless), `skills_server.rs` (`mcp skills` — protocol + tools),
  `rpc.rs` (the JSON-RPC plumbing all three share — schema builders,
  result wrappers, argument decoders), `harness_server.rs`
  (`mcp harness` — protocol + tool dispatch) over `harness.rs` (the
  session-driving logic) and `sessions/` (the owned-session store).
  `skills/` = `SkillsRegistry` + the `SKILL.md` loader, plus
  `wire_metadata.rs` / `wire_references.rs` / `wire_time.rs` (the MCP
  wire-shape projection, beside the loader whose frontmatter they read
  and whose `split_frontmatter` `wire_references` reuses for a
  reference's own fence) — under `mcp/`
  because everything it feeds exists for the skills server. `resolve`
  builds one per launch solely to gate that server's injection (skills
  is the only server also gated on content).
- `profiles.rs` — the `profiles` subcommand.
- `watch.rs` — general directory watching: `WatchRoot` in, debounced
  `WatchSignal` out. Knows nothing about skills (no slugs, no
  `CatalogueDelta`, no rmcp), so the seam is the type: the skills server
  maps `ResolvedSkillEntry` into `WatchRoot` and owns everything
  downstream of the signal. Crate root, not under `mcp/`, because the
  skills sidecar is its first consumer rather than its shape.
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
hyprpilot mcp skills --skill-dir '{"dir":"/abs/path","ignore":[],"watch":true}'
hyprpilot mcp harness --max-sessions 64 --max-live-sessions 0
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
  `--cwd <dir>`, `--mode`, `--resume[=<session>]` / `--resume-last`
  (see Resume below), `--with-config` / `--with-config-format`,
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
the XDG skills dir (watched), naming each in-tree server, and carrying
the harness ceilings — the values that must survive layer merge, plus every
NESTED leaf the resolver never backfills.
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

**A server's name is CONFIG, not a constant.** `[mcp.<server>] name` is
what `auto_inject` writes into the vendor catalogue and the only thing
it reads, and `defaults.toml` seeds all three — so renaming one is an
edit, not a rebuild. `DEFAULT_*_SERVER_NAME` covers only a `Config`
carrying no patches (a programmatic one in a test, or a captain who
cleared the seed) and is what each SIDECAR reports as `serverInfo.name`,
since a sidecar cannot know which catalogue key spawned it.
`defaults_seed_the_server_names` pins the pair equal, the same way
`defaults_seed_the_harness_ceilings` does for the numbers.

| Subcommand | Default name | Module | Serves | Default |
| ---------- | ------------ | ------ | ------ | ------- |
| `mcp serve` | `hyprpilot` | `server/tools.rs` | `open` | on |
| `mcp skills` | `hyprpilot-skills` | `server/skills_server.rs` | skills tools + resources | on |
| `mcp harness` | `hyprpilot-harness` | `server/harness_server.rs` | `list_profiles` / `spawn` / `session_*` (7 tools) + session resources | **off** |

**Sessions are resources in FOUR views** — `hyprpilot://sessions/
<handle>` (status), `/result`, `/transcript` and `/stderr` — plus two
indexes, `hyprpilot://sessions` (what `session_list` returns) and
`hyprpilot://profiles` (what `list_profiles` returns, same delegate
scope). **Each turn owns a directory** — `turns/<n>/{turns.jsonl,stderr.log,
done.json}` under the session, with `session.json` session-scoped. That
layout is load-bearing three ways: a turn's output IS its file, so
reading turn N cannot reach turn N+1 and needs no byte offsets; its
stderr is its own, so "stderr is non-empty" means THIS turn wrote it
rather than an earlier one; and a fresh turn is a fresh directory, so no
completion marker has to be cleared before it starts. `sessionInfo.files`
names the session's paths plus the CURRENT turn's (`turn`, `turnDir`,
`turnsDir`); earlier turns are inferable from the turn number and are
deliberately not enumerated. `session_read`'s cursor carries its turn
(`turn.offset`, hex) — a bare offset would address the wrong file once
the next turn started.

Reading `hyprpilot://sessions/<handle>` also lists every turn with its
outcome and the URI that fetches it, so one read answers "which turns
exist and which is worth fetching" instead of walking
`…/turns/<n>/status` until one errors. The UN-TURNED forms stay the
shortcut to the current turn. Every view is also addressable PER TURN
(`…/turns/<n>/<view>`). Guessing the boundary from the events was
a live bug twice — a heuristic mis-attributed one turn's error to the
next, then an unbounded slice swallowed every later turn — which is what
the per-turn layout retires rather than patches. `resources/list` names the indexes and ONE entry per session,
never one per view — four views across 64 retained sessions is 256 rows
every client pays for on connect, the bloat the skills listing already
measured and cut. The views ride a resource TEMPLATE instead.
Both INDEXES carry `ttlMs: 0`: `hyprpilot://sessions` embeds live per-session status and nothing fires `resources/updated` for the index URI, and for profiles, config is
re-read per call and nothing watches that file, so there is no signal to
invalidate it with. `done.json` and the breadcrumb are deliberately NOT
resources — the status view answers the first, whose whole purpose is
being reachable without MCP, and the second is orphan plumbing. A caller
can
`subscriptions/listen` on a handle and be WOKEN when its turn ends
instead of polling `session_status` or watching `done.json`, and then
read only the part it wants.

`/result` is the load-bearing one: `server/transcript.rs` does the
per-vendor extraction the `jq` recipes did by hand, and the two silent
failures are structurally impossible there — it slices by EVENT so a
multi-line answer cannot be truncated to its last line (`tail -n1` did
exactly that), and an `error` event OUTRANKS text so an upstream 402
cannot report as "returned nothing". It never returns blank for a
finished session: the three no-answer shapes land in different places —
`error` event in the transcript, launch failure in `stderr.log` with the
transcript EMPTY, or neither — so it falls through transcript, stderr,
exit code and names which one happened. `/transcript` is capped and cut
from the front, since the answer is at the end; `session_read` stays
the pager because a resource read has no cursor.

**TTL is conditional here, unlike everywhere else**: a running session's
views carry `ttlMs: 0`, because they change under the caller and the
turn-end notification cannot retroactively correct a cache taken a
second before it. Only a finished session gets the 24h ttl. An unknown
view is refused rather than read as the status — the subscription filter
is built on the same parser, so accepting one would acknowledge a URI
that can never be served.

**All three serve from the connection's FIRST byte**
(`rpc::serve_from_first_byte`, wrapping rmcp's `serve_directly`), never
`ServiceExt::serve`. `serve` runs a pre-loop handshake that handles a
non-`initialize` opener INLINE — `handle_request().await` completes
before the serve loop is spawned — so a LONG-LIVED opener deadlocks the
process: `subscriptions/listen` acknowledges through
`Peer::send_notification`, which awaits a oneshot only the loop can
fire, and the loop does not exist yet. Nothing is read or written
again, ever. That is not hypothetical: Claude Code's v2 MCP runtime
probes `server/discover` on a DISPOSABLE second process, then opens the
real transport with `subscriptions/listen` as its first request — so
for a server implementing subscriptions this ordering is the NORMAL
path. It reported `connected` (the throwaway probe succeeded) and then
`tools fetch failed`, on one account only, because the runtime is
gated per-account. `mcp serve` was immune twice over: it advertises no
`listChanged`, so no listen is opened, and it does not override
`accepted_subscription_filter`, so rmcp answers `-32601` before
`establish`. Negotiation still runs against
`supported_protocol_versions`, but rmcp's in-loop `initialize` records
the version the client ASKED for rather than the negotiated one — so
every server overrides `initialize` to use
`rpc::initialize_negotiated`. Without it a client told `2025-11-25` is
still served `2026-07-28` result shapes, which is the `ttlMs` failure
again from the other side. Requests now also run CONCURRENTLY, so a
client that pipelines past `initialize` can be answered before that
version is recorded; the spec forbids it, and nothing is owed to a
client that does. Tests drive every opener
(`initialize`-first, `discover`-first, `listen`-first) because rmcp
gives the first request its own code path; a smoke that only opens
with `initialize` covers one of three.

`server/rpc.rs` owns the plumbing all three import (`object_schema`,
`structured_with_text`, `tool_error`, `require_string`,
`optional_*`, `wait_for_shutdown`, `supported_protocol_versions`). It
used to live in the skills server purely because that server was written
first — five of the helpers had no caller there at all.

**The negotiable protocol set is ONE declaration through
`2026-07-28`** (`rpc::supported_protocol_versions`, overridden on all
three `ServerHandler`s — one function, no per-server variant). rmcp's
default is `KNOWN_VERSIONS` and negotiation echoes back whatever the
client asks within it, so inheriting the default would let a vendor
CLI's own release change our wire shape. Declaring it keeps the set a
statement rather than an emergent property; a client below it is
unaffected (codex negotiates 2025-06-18) and one asking higher
negotiates down.

**Every cacheable result MUST carry `ttlMs` + `cacheScope`**
(`rpc::RESULT_TTL_MS` / `RESULT_CACHE_SCOPE`, stamped at all seven
sites: `tools/list` on each server, plus `resources/list`,
`resources/templates/list` and both `resources/read` arms on skills).
`2026-07-28` makes them REQUIRED — `ListToolsResult extends
PaginatedResult, CacheableResult`, and `CacheableResult` declares both
without `?` — while rmcp models them `Option` for back-compat and
defaults them to `None`. A server that just calls `with_all_items`
emits neither, and a client validating the revision it negotiated
rejects the listing: `ttlMs: expected number, received undefined`. That
is not partial breakage — the listing is the door, so the session
reports `connected` and has NO TOOLS AT ALL. Claude Code 2.2.x
negotiates `2026-07-28` and hit exactly this against the harness, which
was the one server that had opted in.

`ttlMs` is **24h — longer than any sidecar lives**, so a client that
honours it never re-fetches on a timer; every real change arrives as a
notification instead. That is only honest because the invalidation is
real, which is the next bullet. `private` because these results are
scoped to one captain's config. Emitting both at older revisions is
harmless — the spec's `Result` is an open map. The earlier per-server
split (harness opts in, the other two capped) was wrong in the
direction that matters: the revision's requirements land on
`tools/list`, which every server serves, so excluding two of them hid
the work rather than avoiding it. A new result type that forgets the
stamp is invisible until a client upgrades — tests pin the constants
and the set.

**Notifications go through `rpc::Subscriptions`, never `Peer::notify_*`
directly.** `Peer::notify_*` is an unconditional pipe send: it reaches
every client whatever it subscribed to, and carries no
`io.modelcontextprotocol/subscriptionId` — so a conforming `2026-07-28`
client, which correlates stream notifications by that id, never sees it
on the stream it opened. Only `SubscriptionSink` filters and stamps.
`Subscriptions` holds every open stream's sink in an id-keyed
`Registry` and picks: the streams when any are open, broadcast when none
are. **Concurrent streams are legal** — rmcp runs each request in its own
task, and `listen(B)` then `cancel(A)` is how a client changes its
filter — so a notification is offered to EVERY open stream and teardown
removes only the entry matching that stream's request id. An earlier
single-slot version cleared unconditionally, which left the surviving
stream acknowledged but sinkless and silently degraded every later
notification to an untagged broadcast.

Both servers also override `accepted_subscription_filter` (rmcp
defaults it to `None` = unimplemented) and filter
`resourceSubscriptions` to URIs they can actually fire for — the
acknowledgment is the client's contract, so accepting a scheme we never
notify leaves it waiting forever. The SDK then intersects with the
advertised capabilities, which is what refuses `toolsListChanged`.
Legacy `resources/subscribe` is answered `Ok` rather than rmcp's
`-32601`, because `resources.subscribe: true` would otherwise be a lie
to a `2025-11-25` client that does receive the broadcasts.

**Every mutable surface pairs the ttl with a signal.** Every RESCAN —
the watcher's on a debounced filesystem event, or `reload`'s on
demand — DIFFS the catalogue (`CatalogueDelta`) rather than firing
blind: any change emits `resources/list_changed` plus
`resources/updated` per changed slug and for the catalogue index, and a
rescan that changed nothing emits **nothing**. Firing spuriously would
make every rescan cost a full re-fetch and teach clients to ignore us.
A changed reference FINGERPRINT updates every citing skill but NOT the
index, which renders a reference count and never a reference's content.
Both callers reach the wire only through `announce()`, so the watcher
and the tool cannot drift into announcing different things for one
delta. On the harness, a turn
starting emits `resources/updated` for its session; a turn ending emits
`updated` AND `list_changed`, because the listing embeds live status;
`spawn` emits `list_changed`, and `session_kill` emits both. The session
listing mutates, so under this ttl it has to say so. The exit hook is installed
UNCONDITIONALLY: `notifyOnComplete` names the Claude channel push alone,
and gating the whole hook on it also skipped `seal_turn` and the session
`resources/updated`, which are correctness rather than noise.

**`list_changed` fires on ANY skills change, not only membership.** A
client that cannot subscribe — anything pre-`2026-07-28` — has no way to
act on `resources/updated`, so narrowing `list_changed` to membership
would make a body edit reach it as silence. The per-URI updates are the
precision for subscribers; `list_changed` is the signal everyone can
use. Any NEW mutable surface must fire a notification or lower its own
ttl; doing neither is invisible until a client caches it for a day.

`tool_error` / `structured_with_text` return rmcp 3's `CallToolResponse`
envelope, converting at that single point so no `call_tool` body deals
with the `Complete` / `InputRequired` / `Task` distinction — every
result these servers produce is `Complete`.

Skills reach the agent **only** through the skills server.

- **`[mcp]` block** (`McpConfig`): `enabled` (default `true` — the
  MASTER gate over all three servers), `serve` / `skills` / `harness`
  (per-server blocks), `autoAcceptTools` (`["*"]`), `autoRejectTools`
  (`[]`). Per-profile `[profiles.mcp]` wholesale-replaces the global;
  folded via patches.
- **Per-server blocks** each carry `enabled`, `name`,
  `autoAcceptTools`, `autoRejectTools`, plus their own fields:
  `[mcp.skills].dirs` (`Vec<SkillEntry { dir, ignore, watch }>`,
  default seed `~/.config/hyprpilot/skills` with `watch = true`. Like
  the harness ceilings, `watch` is SEEDED in `defaults.toml` rather than
  left to Rust: `[mcp.skills]` is nested, so the resolver never
  backfills its leaves, and the value a real launch reads belongs in the
  file the captain edits. `DEFAULT_SKILL_ROOT_WATCH` covers only a
  `Config` carrying no patches, and `defaults_seed_the_skills_watch`
  pins the pair equal. One word, so no casing alias and no
  duplicate-key hazard when a captain overrides the seed. A `dirs` entry
  is keyed by `dir` in the patch engine — without that the array fell to
  the primitive branch and a user layer naming the seeded root APPENDED
  a second entry for the same directory instead of overriding it, so the
  documented `watch = false` watched the root anyway and reported it
  off) and `[mcp.harness]`'s
  `maxDepth` / `maxSessions` / `maxLiveSessions` / `notifyOnComplete` /
  `includeProfiles` / `excludeProfiles` / `mcp`.
  A per-server tool-policy glob list OVERRIDES the `[mcp]`-level one
  rather than merging. `[mcp.harness].enabled` defaults to **false**
  and that is a security property, not a preference — a profile's
  `command` is an arbitrary binary, so `spawn` executes as the user.
- **The nested server blocks merge PER LEAF** (`merge_nested`), not
  wholesale. `overwrite_some` is right for a leaf and wrong for a
  sub-block: a delegate overlay naming only `skills.enabled` would
  otherwise replace the whole `skills` block and take `dirs` with it.
  Two call sites: `effective_mcp_with`, whose left operand is
  `McpConfig::default()` with all three `None` (so the change is inert
  there), and the delegate fold in `spawn::prepare`, which is the one
  that needs it. `overwrite_some` is RIGHT-wins, so the overlay is the
  right operand — reversed, an inherited `Some` clobbers it and the
  feature no-ops against exactly the config that motivated it.
- **Both casings parse**, at the serde layer only: `[mcp.*]` serializes
  camelCase while the rest of the tree is snake_case, so every
  multi-word field carries a `#[serde(alias)]` for the other spelling.
  There is deliberately NO key rewriting in the patch engine — `mcps`
  and `env` are keyed by vendor server name and env var, which must
  survive verbatim. Consequence: patches merge by KEY STRING before
  anything is typed, so writing a `defaults.toml`-seeded key
  (`maxDepth` / `maxSessions` / `maxLiveSessions` /
  `notifyOnComplete`) in the OTHER
  spelling reaches serde as a duplicate field and fails config load.
  Loud, pinned by a test, and the reason to write a seeded key the way
  the seed writes it.
- **`McpConfig`'s serde default is `sparse()`, not `Default`.** Its
  `Default` seeds the resolver floor (`enabled = true`,
  `autoAcceptTools = ["*"]`), and container `#[serde(default)]` would
  fill missing fields from it — so a partial block on disk came back
  carrying values its author never wrote. Invisible where
  `effective_mcp_with` backfills the same numbers, and wrong for
  `[mcp.harness.mcp]`, where an unwritten leaf must INHERIT the
  delegate's own rather than override it.
- **`[mcp.harness].maxDepth`** (default `1`, seeded in `defaults.toml`)
  is read in ONE place — the `[mcp.harness]` block a gate is deciding
  on — and answers two questions with `depth < maxDepth`: whether a
  session at that depth gets a harness INJECTED, and whether a sidecar
  at that depth may `spawn`. The injection half is absolute: `enabled =
  true` does not re-open it, because a harness at the cap could only
  refuse `spawn`. Self-adjusting — raising it reopens the next level
  with nothing else to change. No upper bound; see "The agent harness"
  for why that is a resource trade, not a security one.
- **`[mcp.harness].mcp`** (`Option<Box<McpConfig>>` — boxed because the
  type is mutually recursive) is the `[mcp]` block every delegate this
  harness spawns receives, folded per-leaf over the delegate profile's
  own resolved block: a key set wins, a key unset inherits. It rides
  argv as `--delegate-mcp=<json>` for the reason `notifyOnComplete`
  does, and the sidecar re-parses AND re-validates it, failing at
  startup on either error — this block is what NARROWS a delegate's
  reach, so dropping it silently widens. `maxDepth` is checked first, so
  nothing here can give a delegate a harness.
- **`defaults.toml` seeds the harness ceilings**, not
  `McpConfig::default()`. `[mcp]`'s own leaves cannot move there —
  their accessors `.expect()` a value, so they need the per-leaf
  backfill a programmatic `Config` also gets — but `[mcp.harness]` is
  nested and never backfilled, so its numbers live in the file the
  captain edits. The Rust constants remain only for a `Config` carrying
  no patches, and `defaults_seed_the_harness_ceilings` pins the pair
  equal so they cannot drift.
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
  (one `--skill-dir` per root, each carrying that root's ignore list and
  `watch` flag as JSON; `watch` defaults ON when absent, so a
  hand-written catalogue entry predating the flag still gets a watched
  root).
- **`hyprpilot mcp skills`** (`mcp/server/skills_server.rs`): an `rmcp` stdio
  server. Resources: `hyprpilot://skills` (the catalogue index —
  markdown; the bare form cannot collide with a slug because every slug
  URI carries a `skills/` prefix) and `hyprpilot://skills/<slug>` (body
  **plus a manifest footer**). That is the WHOLE resource surface —
  there is no reference URI; see the `resources/list` bullet below.
  Tools: `list_skills`, `read_skill`, `list_skill_references`,
  `read_skill_references`, `reload` (force a rescan — the FALLBACK for
  a root the watcher reports degraded or off, and for a reference file
  outside every root).
- **A reference is addressed by its canonical PATH**, not a slug or a
  name. A path is what the citation IS; a slug-and-name is one of many
  addresses for one shared file, which is exactly what makes double
  loading invisible. Addressing by path de-duplicates (two citations of
  one file are one path, and a repeated path is served once), spans
  skills in a single call, and needs no collision rule — paths are
  unique by construction, so two references sharing a LABEL inside one
  skill are both fully addressable. There is no shadowing.
- **A caller-supplied path is CHECKED, never joined.** `SkillsCache
  .declared` maps every canonical path some skill declares to its
  CITERS and its `FileStat` fingerprint, built once per rescan because
  it is structural; a load request is validated
  against it, so the surface reaches exactly the files the skills
  already reference. The DECLARED spelling
  (`../references/output-diff.md`) never reaches the wire — it is
  meaningless outside its bundle dir, and publishing both would offer a
  caller two addresses of which only one works. That is why
  `skill_block` drops the raw `references` array, the same way it drops
  `title`/`description`: another field carries it better.
- **Reference BODIES are opt-in; the MANIFEST is not.** `read_skill`
  defaults to body + manifest (`path` / `name` / `size` / `modified` /
  `created` / `metadata` per row); `bundle: true` adds every body.
  `read_skill_references { references: [path] }` fetches by path.
  The reason is duplication, not size: references are shared (479
  citations across 103 skills resolve to 60 shared files), so the old
  always-bundle default re-sent conventions already in context —
  `read_skill git-commit` was ~34 KB, now 12.6 KB. The manifest is what
  keeps the flipped default from being a silent gap, which is why it
  also rides the RESOURCE path as a text FOOTER: a resource read returns
  text plus `_meta`, and many clients never surface `_meta` to the
  model, so an attached skill would otherwise lose its references with
  no in-context signal at all.
- **`list_skill_references { slug }` takes a REQUIRED slug.** A
  whole-catalogue scan was a six-figure payload — the single largest
  thing this server could produce — and per-skill listing answers the
  same question incrementally, since comparing paths needs only the
  skill in hand. `list_skills` does NOT resolve references either (it
  reports `referenceCount`, served purely from cache); resolving there
  would read every declared file of every skill on every catalogue call.
- **`resources/list` is the catalogue and skill bodies, NOTHING else.**
  There is no reference URI at all — not per-skill, not per-reference.
  This is measured, not stylistic: on a 127-skill root the listing is
  128 entries / ~105 KB (~26k tokens); adding one bundle entry per skill
  took it to 231 / ~170 KB, 48% of it `_meta`, each bundle entry
  repeating its OWN skill's block verbatim. Enumerating all 479
  references would reach ~607 entries and ~500 KB — over 120k tokens
  before a single skill is read.
- **A declared-but-unreadable file** keeps its declared position as a
  `status: not-found` marker in the manifest and any bundle (a trailing
  summary would say a reference is missing without saying where it
  belonged) and carries no path, so it cannot be fetched.
- **A reference may carry its own frontmatter**, parsed by the loader's
  `split_frontmatter` rather than a second implementation, and served
  with the fence STRIPPED — leaving it would put a bare `---` directly
  under our own `reference:` header, where it reads as a delimiter
  rather than as data. Its keys project into the manifest row's
  `metadata` VERBATIM; nothing is defaulted in. In particular there is
  no `disableModelInvocation` default: hyprpilot enforces no invocation
  gate anywhere (the key is honoured by the AGENT, per its own
  `AGENTS.md` tiers), so stamping it would imply a restriction that does
  not exist.
- **A FETCHED reference carries FULL metadata.** The bundle header is
  built from the same `manifest_row` the listing advertises, so the two
  cannot drift. Full detail is affordable there and not in a listing: it
  is emitted once per reference deliberately requested, where
  `resources/list` pays for the whole catalogue. Header values that
  would break the YAML block (a `:`, a newline, a leading `-`) are
  quoted, so a hand-authored frontmatter value cannot forge a delimiter.
- **Timestamps are `modified` + `created`, never atime**
  (`skills/wire_time.rs`, RFC 3339 UTC via `chrono` — already compiled
  in transitively via `rmcp`, so making it direct costs nothing). atime
  records READS and updates lazily under `relatime`, so it answers
  neither "when did this change" nor "when was this added". `created`
  is the filesystem birth time and is OMITTED where unsupported rather
  than back-filled from `modified`, which would answer a different
  question than the key names.
- Reference BODIES and manifests are resolved per call, not cached with
  the skill — a reference changes far more often than its declaring
  skill, and caching would serve a stale convention (and a stale mtime,
  the one thing these fields exist to report) until an unrelated
  rescan. Cached are the declared-path allow-list and each declared
  file's size + mtime FINGERPRINT: the fingerprint is what lets a
  rescan tell a reference edit from silence, and because `modified` is
  a served manifest field a fingerprint change IS a served-content
  change. The comparison is on the RAW mtime, not the served string:
  `modified` is truncated to seconds for readability, and comparing on
  that made two same-length edits inside one second identical — a rescan
  that diffed to nothing while every citer went stale for the full ttl.
  Display basis and comparison basis are deliberately different.
  `between` likewise compares `title` / `description` / `refs` DIRECTLY,
  because `skill_block` strips all three, so a frontmatter-only edit
  otherwise rode on that same second-granularity stat. One `metadata()`
  per unique declared file per rescan, so a file 60 skills share costs
  one stat.
- Bundles delimit each file with a `reference:` YAML block carrying its
  full manifest row, under a `skill_references:` banner naming the skill
  and count, so an appended bundle can never be mistaken for more skill
  body.
- Skills are discovered by directory scan — the same
  `SkillsRegistry` discovery the launcher uses — and every root is
  WATCHED, so an edit is rescanned and announced without a tool call.
  **The watcher contract:** armed BEFORE the startup scan (so an edit
  between the scan and the first drain is queued, not lost); the relay
  is spawned AFTER `serve_from_first_byte` with
  `running.peer().clone()`, mirroring the harness exit hook, and is
  never an opener and never on a request path — so it cannot
  reintroduce the pre-loop deadlock. Rescans are serialised by a
  `reload_gate` mutex (two concurrent `reload` calls could already
  interleave scan/swap and regress the cache). Posture is
  warn-and-DEGRADE, never fatal: unlike `--delegate-mcp`, losing the
  watcher widens nothing and leaves the catalogue exactly as stale as
  before one existed, so `ENOSPC`, a missing root or a dead watcher
  thread mark the affected root degraded and keep serving. A
  `WatchSignal::Degraded` carries the dirs the backend attributed the
  error to; an EMPTY list means every root, and that is the only honest
  reason to blame roots that may be fine — otherwise one root's
  mid-session failure reported every other one unwatched, permanently. Coverage is reported
  in `list_skills` (`watch: { active, roots }`) AND appended to its text
  summary when partial, so an opencode-style text-only client learns it
  needs the fallback. Two cases a watch cannot cover, which is why
  `reload` stays: a root on a filesystem that never fires (NFS/SSHFS/
  FUSE accept the watch silently — no error to degrade on, hence
  per-root `watch = false`), and a reference declared ABOVE every root.
  **Every tool result carries BOTH a `content` text block AND
  `structured_content`** (`serve::structured_with_text` overwrites
  `CallToolResult::structured`'s `.content` with an explicit readable
  summary — `list_skills` a one-line-per-skill catalogue, `read_skill`
  the body): clients that render only `content` (opencode) show the
  text instead of "Unknown"; structured-aware clients (Claude Code)
  still get the JSON. A structured-only result renders as "Unknown" in
  opencode — never return one.

## The agent harness (`mcp harness`)

`hyprpilot mcp harness` serves seven tools that let a connected agent
drive hyprpilot profiles: `list_profiles` (discovery), `spawn`,
`session_send`, `session_list`, `session_status`, `session_read`,
`session_kill`.
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
  failure mode the split removes.) `HYPRPILOT_SPAWN_DEPTH` carries the
  depth and `[mcp.harness].maxDepth` bounds it, **default 1** — a
  spawned agent gets no harness injected AND its `spawn` is refused, so
  the lead delegates and the delegate works. `spawn::prepare` writes the
  stamp, so the depth that gated a session's catalogue and the depth
  that session reports are one number. Resource shape, not security:
  each sidecar's concurrency ceiling covers only its own table, so a
  second level would make N delegates × N sessions a fan-out no single
  ceiling catches and the lead cannot see. A session-count ceiling
  bounds breadth.
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
  spawn's `cwd` / `mode` / `with_config` / `args` forward (recorded as
  `sessions::LaunchShape`) and its tool schema does NOT accept
  `cwd`/`args`/`with_config` — passing one is an error, not a silent
  drop. Only `prompt`/`file`, `mode`, `wait`, `timeout_seconds` are
  per-turn. How a conversation was launched is part of its IDENTITY:
  claude keys its conversation store by project directory, so a resume
  from a different cwd failed with a bare "No conversation found with
  session ID" for a healthy session.
- **Reads paginate the MCP way.** `cursor` in, `nextCursor` out, opaque
  (hex-encoded byte offset) so a caller cannot synthesise a position it
  never read. **Absent `nextCursor` = finished AND fully read**; a
  running session always gets one. There is no `truncated` flag — the
  cursor's presence is the signal. An unrecognised cursor errors.
- **cwd reaches each vendor differently.** claude inherits the process
  cwd; codex takes a global `--cd`; opencode takes `--dir` **on `run`
  only**. hyprpilot sets `current_dir` AND emits the flag for the two
  that need one — opencode does not derive its tool sandbox from the
  process cwd, so without `--dir` the agent silently worked in the
  wrong tree while every surface reported the requested path. `--dir`
  is gated on the headless path: the bare TUI command takes a
  positional `project` and parses strictly, so passing it there exits 1
  with a usage dump and no session at all.
- **`[profiles.harness]` is OPT-IN.** A profile with no block is not
  available: absent from `list_profiles` AND refused by
  `spawn`/`session_send`. Within a declared block `enabled` defaults
  true; it is the block's ABSENCE that keeps a profile off. Both halves
  matter — `launch` is the shared body of both tools, so one check
  covers them; gating only the listing would leave it reachable by id.
  An unknown id stays "allowed" so the resolver keeps owning that error.
- **`[mcp.harness].includeProfiles` / `excludeProfiles` scope delegation
  PER LAUNCH.** `[profiles.harness]` is the target's own opt-in and is
  global — it says a profile may be driven, not by whom. These two globs
  are the LAUNCHER's scope, so a `$match`ed patch gives `personal/*` a
  harness reaching only `personal/*`. `globset`, so `*` crosses `/` like
  `$match.profile`. Exclude beats include. The two gates **AND**: a glob
  can never promote a profile that never declared `[profiles.harness]`.
  Both halves again — `list_profiles` filters, `launch` refuses — with
  distinct messages, since the two gates have different fixes. Unknown
  ids clear both, keeping "unknown profile" the resolver's error.
  The filter runs on the id ALONE, before resolution, so unlike
  `harness_enabled` it cannot be wrong about a profile whose patches
  broke. **`--no-delegates` carries `includeProfiles: []`**: zero
  `--include-profile` occurrences is indistinguishable from unset on the
  wire, and unset means unrestricted — the empty list must not decay
  into its opposite. An empty scope still INJECTS the server; it just
  has no candidates. A malformed glob fails the sidecar at startup
  rather than being skipped, because a dropped `exclude` silently
  widens. Same class as `enabled`: bounds discovery, not capability.
- **A conversation is ONE session.** `session_send` reuses its handle and
  appends to the same transcript, so an N-turn conversation costs one
  table entry, not N. Its check-and-spawn happens under the table lock —
  `Command::spawn` is synchronous, so "one turn at a time" is an
  invariant, not a racy check.
- **`done.json` is the vendor-neutral completion signal.** The waiter
  task writes it beside `turns.jsonl` after `child.wait()` returns, and
  `launch_child` DELETES it before every turn — `session_send` reuses
  the directory, so a watcher armed for turn N+1 would otherwise fire
  on turn N's leftover. Surfaced as `sessionInfo.files.done`; `files` names every
  file the session owns (`dir` / `transcript` / `stderr` / `done` /
  `breadcrumb`) so a caller can `jq` the transcript instead of paging
  it. Advisory:
  reap/evict/shutdown remove the directory, so the watcher contract is
  `[ ! -d "$DIR" ] || [ -f "$DIR/done.json" ]`. Never panic in that
  task — `panic = "abort"` would take every running agent down with it.
- **Completion fires a Claude channel.** `[mcp.harness].notifyOnComplete`
  (default TRUE, resolved by the LAUNCHER and passed down as
  `--no-notify-on-complete` — a sidecar cannot know which profile
  spawned it, so it cannot read a per-profile key itself) pushes `notifications/claude/channel` when a turn's
  process exits — Claude Code renders it as a `<channel>` block in the
  lead's next turn. Safe on by default: an unregistered channel is
  dropped silently and unknown capabilities are ignored per spec, so
  the knob is for NOISE (a session is `exited` every turn). The peer
  only exists after `serve()` returns, so the hook is installed there,
  and `sessions/` stays rmcp-free — it takes a bare `ExitHook` closure
  built in `harness.rs`. **The content is a fixed template**: never
  interpolate agent output, or a spawned agent writes into its parent's
  context through a path the parent never called.
- **Bounded retention counts FINISHED sessions only.** `maxSessions`
  (default 64, `0` retains everything) evicts the oldest *finished*
  ones; a running session is never evicted AND never counted. Counting
  one would spend the history budget on work still in flight, so with
  the concurrency ceiling off, enough concurrent agents would evict
  every transcript the cap exists to keep. `session_kill` is
  state-aware — it terminates a running session (keeping the
  transcript) and reaps an already-finished one.
- **The concurrency ceiling is OFF by default.**
  `[mcp.harness].maxLiveSessions` (default `0` = unlimited) refuses
  `spawn` past N *running* sessions. It bounds breadth where `maxDepth`
  bounds recursion, and it is off because how many agents a host can
  carry is the captain's fact, not ours — a resource knob for a shared
  or tight machine, not a safety boundary (`spawn` runs an arbitrary
  binary either way). Rides argv as `--max-live-sessions`, the same way
  `--max-sessions` does.
- **Listings lead with what is RUNNING**, then most-recent turn first
  (`SessionTable::map_all`). The table is keyed by UUID, so its own
  order is noise — which scattered the sessions a caller is waiting on
  among dozens of retained transcripts, in a listing whose whole job is
  answering what is happening now.
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
- **One id, minted by us.** The `session` handle exists from `spawn`,
  never changes, and is the only identifier any tool accepts. The
  harvested vendor id is stored as `Session.resume_token` and is
  strictly the argument `session_send` hands back to the vendor — it is
  NOT on the wire anywhere (`describe` / `sessionInfo` / `session_status`
  / `session_list`). Exposing it published a second id for the same
  thing that behaved worse: `None` for the whole first turn, and
  addressing nothing. A test pins it out of every metadata payload —
  but NOT out of `describe`'s `text`, which is the vendor's own event
  stream verbatim and must stay unedited.
- **SEP-2663 Tasks ride alongside, never instead.** `spawn` /
  `session_send` return a `CallToolResponse::Task` **only** when the peer
  declared `io.modelcontextprotocol/tasks`; every other client gets the
  exact result it got before, and rmcp independently rejects a task sent
  to a non-declaring peer. Only the `Ok` arm can become a task — a
  refused launch stays a `tool_error`, because a task id for work that
  never started can never resolve. The one ungated part is the
  `extensions` key in `initialize`, which is required for `tasks/*` to
  route at all and is ignored by clients that do not know it.
- **A task names a TURN, not a session** (`task_id` = `<handle>:<turn>`).
  The spec makes `completed`/`failed`/`cancelled` terminal, while a
  session handle is reused across turns and cycles `exited → running →
  exited` — keyed by handle alone, turn 2 starting would mutate turn 1's
  finished task. That forces `Session.turns: Vec<TurnRecord>`: `respawn`
  replaces the `done` watch wholesale, so a previous turn's exit code is
  otherwise unreachable. The session handle rides `CreateTaskResult._meta`
  (`io.hyprpilot/session`) so a caller never has to PARSE the task id.
- **A terminal task's payload must not MOVE**, so every field of
  `sessionInfo` is read off the turn's own `TurnRecord` — its
  `provenance` (model / effort / mode / argv), `pid`, `turnStartedAt`
  and its file paths. The session's copies of all of those are
  overwritten by the next turn, which is what made a re-polled finished
  task hand back a later turn's answer. Any NEW `sessionInfo` field has
  to come off the record too; sourcing one from the live session is
  invisible until a caller re-polls.
- **`TurnOutcome::Killed` is stamped in `SessionTable::kill`, not
  derived.** The waiter stores `status.code().unwrap_or(-1)` and `code()`
  is `None` for signal death, so a kill, an external signal and a wait
  error are indistinguishable after the fact. Stamped only past the
  already-exited early return — reaping a session that finished normally
  must not report its turn cancelled.
- **`notifications/tasks` is DOUBLE-GATED**: the peer declared tasks AND
  a task exists for that turn. The exit hook fires for every turn of
  every session, so an ungated push would reach a client that opted into
  nothing. It rides `Peer::send_notification` directly because rmcp
  refuses to route task notifications through `subscriptions/listen`
  (`SubscriptionFilter` has no `taskIds` field) — only
  `resources/list_changed` and `resources/updated` are routable there.
- **Launches are DETACHED by default.** `wait` defaults to **false** on
  `spawn` / `session_send` (`wait_flag`), so both return as soon as the
  turn starts. Waiting never guaranteed a finished answer — a turn past
  `timeout_seconds` comes back `running` regardless — so it only cost
  the caller its ability to do anything meanwhile; `session_status` is
  the cheap poll that replaces it. `timeout_seconds` is inert unless
  `wait: true`. Consequence: `session_send`'s lazy `harvest` is
  load-bearing on the DEFAULT path now, not just an opt-in one — a
  detached first turn never runs the waiting path, so without it no
  session could ever be resumed.
- **Streaming** rides `notifications/progress` when the caller supplies
  a progressToken; a follow ends on session exit, client cancellation,
  or a caller-set limit. MCP tool results are single-shot, so the result
  still carries everything the notifications streamed.

- **Skill metadata — ONE block, spec fields canonical**
  (`mcp/skills/wire_metadata.rs`): the MCP spec's `_meta` is a single
  field keyed by reverse-DNS names, so every skill surface carries
  exactly ONE namespaced key — **`io.hyprpilot/skill`** in resource
  `_meta`, **`metadata`** in tool output (`list_skills` / `read_skill` /
  `read_skill_references`) — and nothing in it repeats a spec-compliant
  `Resource` field. The block = the WHOLE frontmatter map **verbatim**
  (`skill_block`) **minus** the keys another field already carries
  — `title` + `description` (byte-for-byte equal to `Resource.title` /
  `Resource.description`) and `references` (superseded by the resolved
  manifest, which addresses each entry by name instead of publishing its
  path) — **plus** the runtime-derived `path`, `bundleDir`, `size`,
  `modified`, `created` (not in the frontmatter).
  Frontmatter `name` is **kept** — `Resource.name` is the SLUG, an
  author's frontmatter `name` may differ, so it's not a spec duplicate.
  There is **no** `io.hyprpilot/frontmatter` key and **no** curated
  camelCase re-projection anymore — a new/custom frontmatter key rides
  through the one block losslessly. `list_skills` keeps the headline
  `slug`/`title`/`description`/`uri` scan view alongside the single
  `metadata` block. Built ONCE per skill into `LoadedSkill.meta_block`,
  not per request — except the timestamps, which are stat'd there and so
  refresh on every rescan.

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

### Resume

`--resume[=<session>]` / `--resume-last` are ONE vendor-neutral intent
(`providers::Resume` — `Picker` / `Last` / `Session(id)`) that each
builder spells in its own CLI, the same shape `--mode` already has. The
harness rides the same enum: `Resume::Session(resume_token)` replaced
`HarnessProjection.resume`, so the by-id path has one implementation
rather than a CLI copy and a harness copy.

| Intent | claude | codex | opencode |
| ------ | ------ | ----- | -------- |
| `Picker` | `--resume` | `resume` | **refused** |
| `Last` | `--continue` | `resume --last` | `--continue` |
| `Session` | `--resume <id>` | `resume <id>` | `--session <id>` |

`--resume` takes `require_equals` so `hyprpilot --resume engineer`
cannot read the positional `[PROFILE]` as a session id — the optional
value and the positional would otherwise compete for the same token.

Two refusals, in the two places that own them. **opencode registers no
picker at all** (only `--continue` / `--session <id>`), so its builder
bails — a fall back to `--continue` would resume a session the captain
never chose. **No vendor offers a picker headless** (claude answers
"requires a valid session ID … when used with `--print`",
`codex exec resume` takes only an id or `--last`), so `spawn::prepare`
bails while the launch still knows why it was asked for. Codex's
`resume` is a SUBCOMMAND — of `codex` and again of `codex exec` — so it
is INSERTED at index 0, or 1 behind `exec`, ahead of every generated
option; claude's and opencode's flags are order-free and append.

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
  `hyprpilot-skills` / `hyprpilot-harness`) and tool set.
- `mcp skills` over a `subscriptions/listen` stream announces a disk
  edit with no `reload`: editing a `SKILL.md` fires
  `resources/updated` for that slug plus `resources/list_changed`;
  editing a declared reference fires `updated` for every CITING slug
  and NOT for the catalogue index; an editor temp file fires nothing.
  A root pointed at a missing directory reports `watch.active: false`
  with `state: degraded` and still answers `tools/list`.
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

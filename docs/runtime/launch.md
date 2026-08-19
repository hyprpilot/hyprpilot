---
title: Launching
order: 30
---

# {{ $frontmatter.title }}

The bare invocation **is** the launch — hyprpilot is one binary, there is no `run` subcommand, and the `profiles` and `mcp` subcommands round out the surface:

```sh
hyprpilot [PROFILE] [flags] [-- <provider args>]
hyprpilot profiles [--json]
hyprpilot mcp serve                          # general tools (`open`)
hyprpilot mcp skills [--skill-dir <json>]…   # the skill catalogue
hyprpilot mcp harness [--max-sessions <n>]   # spawn/drive agent sessions
```

<!-- more -->

## Picking a profile

The profile is an **optional positional argument** — no `--profile`/`-p` flag:

```sh
hyprpilot                       # interactive picker over configured profiles
hyprpilot engineer              # launch the `engineer` profile directly
```

Omit the positional and an interactive fuzzy picker (powered by `nucleo`) opens over your configured profiles — each row shows the default marker, id, agent, model, and cwd. The `profile.default` entry starts **pre-selected under the cursor**, so a bare `hyprpilot` followed by <kbd>Enter</kbd> launches your default. Cancelling the picker aborts the launch; a non-interactive terminal errors instead of hanging.

Because subcommands resolve before the positional, `hyprpilot profiles` and `hyprpilot mcp` always run the subcommand — a profile literally named `profiles` or `mcp` is therefore not positionally addressable (rename it, or reach it through `profile.default` + the picker).

## Launch flags

| Flag                                      | Purpose                                                                                                      |
| ----------------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| `[PROFILE]`                               | Positional session-profile id to resolve and launch. Omit to pick interactively.                             |
| `-p, --prompt <PROMPT>`                   | Inline headless prompt — the non-pipe alternative to `echo … \| hyprpilot`. Forces headless.                 |
| `-f, --file <PATH>`                       | Read the headless prompt from a file (`~` / `$VAR` / relative expanded). Mutually exclusive with `--prompt`. |
| `--cwd <dir>`                             | Working directory for the vendor process.                                                                    |
| `--mode <mode>`                           | Mode override, projected onto the vendor where supported.                                                    |
| `--resume[=<session>]`                    | Continue a conversation — bare opens the vendor's session picker, with a value it resumes that session.      |
| `--resume-last`                           | Continue the most recent conversation, no picker. Mutually exclusive with `--resume`.                        |
| `--with-config <path\|@inline\|->`        | Profile overlay patch (repeatable). See [Ad-hoc Overlays](./with-config).                                    |
| `--with-config-format <toml\|json\|yaml>` | Format for stdin / inline / extension-less overlays (default `json`).                                        |
| `-- <args>`                               | Everything after `--` is forwarded verbatim to the vendor CLI; generated equivalents are suppressed.         |

## Overriding per launch

The profile is the single source of truth for its agent and model — there are no `--agent` / `--model` launch flags. The two per-launch knobs that remain touch only where the vendor runs and how, not what the profile _is_:

```sh
hyprpilot engineer --cwd ~/code/foo   # run somewhere else
hyprpilot engineer --mode plan        # vendor-specific mode
```

- `--cwd <dir>` beats the profile's (or agent's) configured `cwd`; when neither speaks, the current directory is used.
- `--mode` is projected onto the vendor CLI where supported.

For a one-off model, agent, MCP set, or system prompt — anything that changes what the profile resolves to — reach for [`--with-config`](./with-config), the ad-hoc overlay escape hatch:

```sh
hyprpilot engineer --with-config '@{"model":"claude-opus-4-5"}'  # different model, one launch
hyprpilot engineer --with-config '@{"agent":"codex"}'            # different agent, one launch
```

## Resuming a conversation

`--resume` and `--resume-last` are vendor-neutral: hyprpilot maps the intent onto whatever the resolved profile's vendor calls it, so one keybinding works across every profile.

```sh
hyprpilot engineer --resume              # pick a session in the vendor's own picker
hyprpilot engineer --resume-last         # straight back into the most recent one
hyprpilot engineer --resume=<session-id> # a specific session
```

| Intent        | `claude`        | `codex`         | `opencode`       |
| ------------- | --------------- | --------------- | ---------------- |
| Picker        | `--resume`      | `resume`        | **unsupported**  |
| Most recent   | `--continue`    | `resume --last` | `--continue`     |
| By session id | `--resume <id>` | `resume <id>`   | `--session <id>` |

Two refusals, both loud rather than silent:

- **opencode has no session picker.** Its CLI registers only `--continue` and `--session <id>`, so a bare `--resume` errors instead of quietly resuming something you did not choose.
- **No picker survives a headless launch.** A picker needs a terminal to answer it, so combining `--resume` with `--prompt` / `--file` / piped stdin errors. Use `--resume-last` or `--resume=<session-id>` there.

Passing the vendor's own flag through `-- <args>` still wins — the generated projection is suppressed exactly as it is for every other flag.

## Forwarding native arguments

Everything after a `--` separator is forwarded verbatim to the vendor CLI — use it for provider-native flags and resume flows:

```sh
hyprpilot engineer -- --resume
hyprpilot review -- --dangerously-skip-permissions
```

Any provider-native argument you pass this way suppresses hyprpilot's generated equivalent, so your hand-written flag always wins over the projection.

## Headless / prompt delivery

Give hyprpilot a prompt and it launches the vendor **non-interactively** — one shot, then exit, like `claude --print`. There are three ways to supply the prompt:

```sh
echo "fix the failing test" | hyprpilot engineer   # piped stdin
git diff | hyprpilot review                         # the diff becomes the prompt
hyprpilot engineer --prompt "fix the failing test"  # inline flag, no pipe needed
hyprpilot engineer --file ./task.md                 # prompt read from a file
```

Headless is **effective** when any of these is true: `--prompt` / `--file` is given, the profile sets [`headless = true`](../config/profiles#headless), or stdin is a pipe (not a TTY). The **prompt source** resolves in priority order — an explicit `--prompt` / `--file` value wins over piped stdin (`--prompt` and `--file` are mutually exclusive; passing both errors).

The full model / effort / mode / MCP / tool-policy projection — plus `--cwd`, `--mode`, and `--with-config` — still applies; headless only changes _how the prompt is delivered_, not _what the profile resolves to_.

### How the prompt reaches the vendor

hyprpilot delivers the resolved prompt on the vendor's **stdin** where the vendor supports it, and as a positional argument otherwise:

| Vendor     | Projected invocation               | Prompt delivery               |
| ---------- | ---------------------------------- | ----------------------------- |
| `claude`   | `claude --print` (prompt on stdin) | **stdin** (spawned, then EOF) |
| `codex`    | `codex exec` (prompt on stdin)     | **stdin** (spawned, then EOF) |
| `opencode` | `opencode run "<prompt>"`          | positional argument           |

For **claude** and **codex**, hyprpilot spawns the vendor, writes the prompt to its stdin, and closes the pipe (EOF). This is deliberate: claude's `--allowedTools` / `--disallowedTools` are **variadic** flags that would greedily swallow a trailing positional prompt as a tool entry, and a positional never reaches the model; stdin has no such ambiguity. **opencode** has no stdin prompt support (its `run [message…]` is positional-only), so the prompt stays a positional argument there. The interactive (non-headless) path always `exec()`s, unchanged.

- **Profile selection.** A headless launch never opens the interactive picker (there may be no TTY, and stdin may be a consumed pipe). With no positional profile it resolves [`profile.default`](../config/profiles#picking-the-default) directly, and errors cleanly when no default is configured — pass a positional profile or set a default.
- **Headless without a prompt.** If headless is forced (profile `headless = true`, or `--prompt`/`--file` — though those always carry a prompt) but no prompt resolves — e.g. `headless = true` on an interactive TTY with no pipe and no `--prompt`/`--file` — the launch **errors** rather than opening a picker it can't drive. An empty prompt (empty pipe, or empty `--prompt`/`--file`) errors too.
- **`--with-config -` already drains stdin.** `--with-config -` reads the pipe to build the overlay, so the same pipe can't also be the headless prompt. Piping into a headless launch that also passes `--with-config -` **errors** with a targeted message (rather than misreporting an "empty prompt") — pass the prompt via `--prompt` / `--file` instead, or forward it through a trailing `-- <provider args>`.
- **Escape hatch — bring your own invocation.** When you pass the vendor's headless flags yourself via `-- …` **without** a `--prompt`/`--file`, hyprpilot does **not** read stdin — fd0 stays inherited so the vendor gets the raw pipe as input data, and the trailing args suppress hyprpilot's generated projection:

  ```sh
  cat data.json | hyprpilot engineer -- -p "summarize this"
  # → claude gets data.json on stdin AND "summarize this" as the prompt arg
  ```

  Only the automatic path (no trailing `--` args) buffers stdin.

- **`-p`/`-f` compose with `-- <provider args>`.** An explicit `--prompt` / `--file` is a deliberate prompt, so it is **delivered even when you also pass trailing `-- <provider args>`** — the two compose rather than being mutually exclusive. hyprpilot delivers the prompt on its usual vendor path (stdin for `claude` / `codex`, positional for `opencode`) **and** appends your `-- <args>` to the vendor argv, where the existing dedup lets a hand-passed flag suppress hyprpilot's generated equivalent:

  ```sh
  hyprpilot engineer -p "fix the bug" -- --allowedTools Read
  # → claude gets "fix the bug" on stdin AND `--allowedTools Read` on argv
  ```

  Only the escape hatch **without** an explicit `--prompt`/`--file` skips stdin entirely.

## Global flags

Available on every invocation, each with an environment twin, so you can pin them per shell or per session:

| Flag                      | Env                        | Purpose                                                                      |
| ------------------------- | -------------------------- | ---------------------------------------------------------------------------- |
| `--config <path>`         | `HYPRPILOT_CONFIG`         | Override the global config path (format inferred from the extension).        |
| `--config-profile <name>` | `HYPRPILOT_CONFIG_PROFILE` | Layer a named config-layer overlay (`profiles/<name>.{ext}`).                |
| `--log-level <level>`     | `HYPRPILOT_LOG_LEVEL`      | Override the tracing filter (`trace` / `debug` / `info` / `warn` / `error`). |

```sh
HYPRPILOT_CONFIG_PROFILE=work hyprpilot engineer
```

`--config-profile` layers a named config file (`~/.config/hyprpilot/profiles/<name>.yaml`) on top of your global config — a **config-layer** profile, distinct from the session `profiles` you address with the positional `[PROFILE]`. See [Config → Layering](../config/layering).

Log filter precedence is `--log-level` → `RUST_LOG` → `logging.level` → the built-in `error` default (a fresh run is quiet — errors only — unless a level is explicitly requested); tracing always writes to stderr. See [Config → Logging](../config/logging).

**cwd reaches each vendor differently.** claude inherits the process working directory; codex takes `--cd`; opencode takes `--dir`. hyprpilot sets the process cwd _and_ emits the flag for the two vendors that need one — opencode does not derive its tool sandbox from the process cwd, so without `--dir` an agent given a `cwd` silently worked in the wrong tree while every surface reported the requested path. A `--dir` / `--cd` you pass yourself after `--` suppresses the generated one.

## What a launch does

1. Load + validate layered config ([Config → Layering](../config/layering)).
2. Pick the profile (positional `[PROFILE]` → `profile.default` → picker) and fold [`patches`](../config/patches) + [`--with-config`](./with-config) overlays.
3. Build the per-launch MCP + skills registries, auto-injecting each in-tree server your `mcp` config enables ([Skills](./skills), [Agent Harness](./harness)).
4. Project everything onto the vendor's native flags/env ([Config → Agents](../config/agents)).
5. Optionally rename the tmux window / zellij tab ([Config → Multiplexer](../config/multiplexer)).
6. `exec()` — the vendor CLI replaces the hyprpilot process.

### cwd precedence

The working directory the vendor launches in resolves as: explicit `--cwd` flag → the profile's (or agent's) configured `cwd` → the current directory. A profile pinned to a repo therefore launches there by default, and `--cwd` overrides it per invocation.

## Inspecting without launching

```sh
hyprpilot profiles              # table: default marker, profile, agent, model
hyprpilot profiles --json       # machine-readable
hyprpilot --with-config '@{"model":"claude-opus-4-5"}' profiles   # preview an overlay
```

The listing resolves config the same way a launch does — including [`patches`](../config/patches) **and any [`--with-config`](./with-config) overlay you pass** — but stops before exec, so the displayed summaries reflect what a launch _would_ use. `--json` keeps stdout pure (all tracing goes to stderr), safe to pipe into `jq`.

If a `patches` / `--with-config` overlay fails to resolve for a profile, that row is flagged with a `!` marker and the error message (in the table, the JSON gains an `error` field) instead of silently showing the un-overlaid base values — so a broken patch is never mistaken for the resolved shape.

An empty `profiles` list is a validation error, not an empty table — fresh installs refuse to run until you configure at least one profile ([Quickstart](./quickstart)). A config typo aborts with an error naming the offending field path.

### Subcommands are not launches

`profiles` and the `mcp` servers are subcommands, not launches, so **launch-only arguments do not apply to them** — the positional `[PROFILE]`, `--cwd`, `--mode`, and a trailing `-- <provider args>` are all rejected with a clear error rather than silently dropped:

```sh
hyprpilot engineer profiles     # error: positional <PROFILE> does not apply to `profiles`
hyprpilot --cwd /tmp profiles   # error: --cwd does not apply to `profiles`
```

The one exception is `--with-config`: `profiles` honors it (the overlay preview above), while the `mcp` servers — which read none of the launch config — reject it too. Run the launch and the subcommand as separate invocations.

## Exit behavior

Because a successful launch replaces the process, hyprpilot's own exit code is the vendor CLI's on unix (non-unix platforms fall back to spawn-and-propagate-exit-code). Config load failures, an empty `profiles` list, an unresolvable profile, or a missing `system_prompt` file abort before exec with a readable error naming the problem.

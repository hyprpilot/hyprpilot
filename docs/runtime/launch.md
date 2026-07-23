---
title: Launching
order: 30
---

# {{ $frontmatter.title }}

The bare invocation **is** the launch — hyprpilot is one binary, there is no `run` subcommand, and two subcommands (`profiles`, `mcp serve`) round out the surface:

```sh
hyprpilot [PROFILE] [flags] [-- <provider args>]
hyprpilot profiles [--json]
hyprpilot mcp serve [--skill-dir <json>]…
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

| Flag                                      | Purpose                                                                                              |
| ----------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| `[PROFILE]`                               | Positional session-profile id to resolve and launch. Omit to pick interactively.                     |
| `--cwd <dir>`                             | Working directory for the vendor process.                                                            |
| `--mode <mode>`                           | Mode override, projected onto the vendor where supported.                                            |
| `--with-config <path\|@inline\|->`        | Profile overlay patch (repeatable). See [Ad-hoc Overlays](./with-config).                            |
| `--with-config-format <toml\|json\|yaml>` | Format for stdin / inline / extension-less overlays (default `json`).                                |
| `-- <args>`                               | Everything after `--` is forwarded verbatim to the vendor CLI; generated equivalents are suppressed. |

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

## Forwarding native arguments

Everything after a `--` separator is forwarded verbatim to the vendor CLI — use it for provider-native flags and resume flows:

```sh
hyprpilot engineer -- --resume
hyprpilot review -- --dangerously-skip-permissions
```

Any provider-native argument you pass this way suppresses hyprpilot's generated equivalent, so your hand-written flag always wins over the projection.

## Headless / stdin pass-through

Pipe a prompt in and hyprpilot launches the vendor **non-interactively** — one shot, then exit, like `claude --print`:

```sh
echo "fix the failing test" | hyprpilot engineer
git diff | hyprpilot review        # the diff becomes the prompt
```

Headless is **effective** when either the piped stdin is detected (stdin is not a TTY) **or** the profile sets [`headless = true`](../config/profiles#headless). hyprpilot buffers **all** of stdin into a string and projects it as each vendor's prompt argument:

| Vendor     | Projected invocation        |
| ---------- | --------------------------- |
| `claude`   | `claude --print "<prompt>"` |
| `codex`    | `codex exec "<prompt>"`     |
| `opencode` | `opencode run "<prompt>"`   |

The full model / effort / mode / MCP / tool-policy projection — plus `--cwd`, `--mode`, and `--with-config` — still applies; headless only changes _how the prompt is delivered_, not _what the profile resolves to_.

- **Profile selection.** A headless launch never opens the interactive picker (there may be no TTY, and stdin may be a consumed pipe). With no positional profile it resolves [`profile.default`](../config/profiles#picking-the-default) directly, and errors cleanly when no default is configured — pass a positional profile or set a default.
- **`headless = true` without a pipe.** If a profile forces headless but stdin is an interactive TTY (no prompt to read), the launch **errors** rather than opening a picker it can't drive. An empty piped prompt errors too.
- **Escape hatch — bring your own invocation.** When you pass the vendor's headless flags yourself via `-- …`, hyprpilot does **not** read stdin — fd0 stays inherited so the vendor gets the raw pipe as input data, and the trailing args suppress hyprpilot's generated projection:

  ```sh
  cat data.json | hyprpilot engineer -- -p "summarize this"
  # → claude gets data.json on stdin AND "summarize this" as the prompt arg
  ```

  Only the automatic path (no trailing `--` args) buffers stdin.

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

Log filter precedence is `--log-level` → `RUST_LOG` → `logging.level` → the built-in `warn,hyprpilot=info` default; tracing always writes to stderr. See [Config → Logging](../config/logging).

## What a launch does

1. Load + validate layered config ([Config → Layering](../config/layering)).
2. Pick the profile (positional `[PROFILE]` → `profile.default` → picker) and fold [`patches`](../config/patches) + [`--with-config`](./with-config) overlays.
3. Build the per-launch MCP + skills registries, auto-injecting the `hyprpilot` server when skills resolve ([Skills](./skills)).
4. Project everything onto the vendor's native flags/env ([Config → Agents](../config/agents)).
5. Optionally rename the tmux window / zellij tab ([Config → Multiplexer](../config/multiplexer)).
6. `exec()` — the vendor CLI replaces the hyprpilot process.

### cwd precedence

The working directory the vendor launches in resolves as: explicit `--cwd` flag → the profile's (or agent's) configured `cwd` → the current directory. A profile pinned to a repo therefore launches there by default, and `--cwd` overrides it per invocation.

## Inspecting without launching

```sh
hyprpilot profiles              # table: default marker, profile, agent, model
hyprpilot profiles --json       # machine-readable
```

The listing resolves config the same way a launch does — including [`patches`](../config/patches) — but stops before exec, so the displayed summaries reflect what a launch _would_ use. `--json` keeps stdout pure (all tracing goes to stderr), safe to pipe into `jq`.

An empty `profiles` list is a validation error, not an empty table — fresh installs refuse to run until you configure at least one profile ([Quickstart](./quickstart)). A config typo aborts with an error naming the offending field path.

## Exit behavior

Because a successful launch replaces the process, hyprpilot's own exit code is the vendor CLI's on unix (non-unix platforms fall back to spawn-and-propagate-exit-code). Config load failures, an empty `profiles` list, an unresolvable profile, or a missing `system_prompt` file abort before exec with a readable error naming the problem.

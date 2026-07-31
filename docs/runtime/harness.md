---
title: Agent Harness
order: 60
next: false
---

# {{ $frontmatter.title }}

`hyprpilot mcp serve --with-harness` turns the in-tree MCP server into a control plane for hyprpilot itself: a connected agent can list your configured profiles, launch one as its own agent session, talk to it across turns, watch it work, and kill it. It's the same sidecar process and stdio transport [Skills](./skills) uses — the harness is six extra tools on top, gated separately.

<!-- more -->

## What it solves

Without the harness, an agent connected to hyprpilot's MCP server can only read skills — it has no way to act as an orchestrator that spins up other hyprpilot sessions. `--with-harness` adds that: discover profiles, start one, send it follow-up turns, follow its output live, stop it. Every launch flows through the same `spawn::prepare` path `hyprpilot <profile>` itself uses, so profile resolution, the `-- <args>` escape hatch, and cwd precedence can't drift between a CLI launch and a harness-driven one — see [Launching](./launch).

## Gating the harness

```sh
hyprpilot mcp serve --with-harness --skill-dir '{"dir":"/abs/path","ignore":[]}'
```

Off by default, deliberately. `mcp.enabled` auto-injects this same sidecar into **every** launch (see [Skills → Auto-injection](./skills#auto-injection)), so an ungated spawn surface would let any claude session spawn nested claude sessions with no bound. It's also, independently, a **security boundary**: a profile's `command` is an arbitrary binary — its `provider` only picks which native-flag projection applies, not a sandbox — so anything that can call `spawn` can execute commands as whoever is running the sidecar. Enable it only where that's intended, e.g. a gateway host whose MCP config opts in explicitly.

The gate covers both halves of the surface: `list_tools` omits the six harness tools when the flag is off, **and** `call_tool` refuses them too. That second half matters on its own — `call_tool` dispatches on the tool name alone, so a client that already knew a harness tool's name (a stale listing, this very page) would otherwise still be able to call it even with an unlisted tool.

::: warning The auto-injected `hyprpilot` entry never carries `--with-harness`
hyprpilot's own `[mcp]` config has no field for it — the launcher-built entry (see [Skills → Auto-injection](./skills#auto-injection)) always omits the flag, however you configure `mcp`. To expose the harness, configure your **own** MCP server entry that runs `hyprpilot mcp serve --with-harness …`, under a name other than the reserved `hyprpilot` — otherwise the auto-injected entry (without the flag, whenever skills resolve) replaces it. See [Example config](#example-config).
:::

## The tools

| Tool            | Purpose                                                                                                |
| --------------- | ------------------------------------------------------------------------------------------------------ |
| `list_profiles` | Discover the profiles you can launch — vendor, model, effort, mode, cwd, MCP/skill counts. Start here. |
| `spawn`         | Start a new session from a profile and send it a prompt.                                               |
| `session_send`  | Send another message to an existing session, resuming it first if it's finished.                       |
| `session_list`  | List this server's sessions — handle, profile, status, exit code, timestamps.                          |
| `session_read`  | Read, and optionally follow live, a session's transcript.                                              |
| `session_kill`  | Terminate a running session and everything it started.                                                 |

### Workflow

1. **`list_profiles`** to find an `id` — a row marked `!` failed to resolve; don't launch it.
2. **`spawn { profile, prompt }`** to start a session. With `wait` true (the default) it blocks and returns the transcript; if the turn outlives `timeout_seconds` the result comes back with status `running`, a `nextOffset` to resume reading from, and the agent **keeps working**.
3. If status is `running`, poll or follow **`session_read { session, wait: true }`** — do **not** call `spawn` again for the same conversation.
4. **`session_send { session, prompt }`** for every follow-up turn, once the session has finished its previous one.
5. **`session_kill { session }`** to stop a runaway agent, or to free a slot when `spawn` reports the concurrency limit.
6. **`session_list`** any time you need to recover a handle you lost.

### `spawn` / `session_send` parameters

The two tools share one parameter set:

| Parameter         | Type             | Default       | What it does                                                                                                  |
| ----------------- | ---------------- | ------------- | ------------------------------------------------------------------------------------------------------------- |
| `prompt`          | string           | —             | The instruction to send. Mutually exclusive with `file`.                                                      |
| `file`            | string           | —             | Path to a file whose contents become the prompt (`~` / `$VAR` expanded). Mutually exclusive with `prompt`.    |
| `cwd`             | string           | profile's cwd | Working directory for the agent.                                                                              |
| `mode`            | string           | —             | Vendor mode override (e.g. claude's `plan`). Overrides the profile.                                           |
| `with_config`     | array of objects | `[]`          | Ad-hoc profile overlays. **Restricted to `model`, `effort` and `mode`** — see below.                          |
| `args`            | string[]         | `[]`          | Extra arguments forwarded verbatim to the vendor CLI — the tool equivalent of the CLI's trailing `-- <args>`. |
| `wait`            | bool             | `true`        | Block until the turn finishes. When `false`, returns immediately with the handle — poll `session_read`.       |
| `timeout_seconds` | integer          | `300`         | Seconds to wait when `wait` is true. On timeout the agent keeps running; the result reports status `running`. |

Exactly one of `prompt` / `file` is required on both — the same mutual exclusion the CLI's `-p`/`-f` enforce. `spawn` additionally requires `profile` (an id from `list_profiles`). `session_send` additionally requires `session` (a handle from `spawn` or `session_list`) and has **no** `profile` parameter — the profile is inherited from the original spawn, so a conversation can't switch profiles mid-stream.

`session_send` inherits only the **profile**. `cwd`, `mode`, `with_config` and `args` are not carried forward from the original `spawn` — pass them again on each turn if the conversation needs them.

::: warning `with_config` is restricted to `model`, `effort` and `mode`
Unlike the CLI's `--with-config`, the harness accepts only those three keys — an allow-list, not a block-list. A profile overlay can otherwise reach `command`, `args` and `env` (which replace the binary outright), `mcps` (whose inline `mcp_servers` entries carry their own `command`/`args`, which the vendor then spawns), `$deleteFromPrimitiveList/<field>` directives (which mutate a field without ever naming it, e.g. stripping a profile's `--sandbox`), and `system_prompt` (which reads an arbitrary file into the agent's context). Any of those turns `spawn` into arbitrary command execution as the sidecar's user.

Enumerating the ways *in* is a losing game against a config tree that grows; enumerating what's allowed is not. To run something else, add a profile for it in the hyprpilot config — that is the captain's decision to make, not the calling agent's.
:::

### `session_read` parameters

| Parameter         | Type    | Default | What it does                                                                                                                        |
| ----------------- | ------- | ------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| `session`         | string  | —       | Required. Handle from `spawn` or `session_list`.                                                                                    |
| `tail`            | integer | `200`   | Trailing lines to return when `offset` is omitted.                                                                                  |
| `offset`          | integer | —       | Byte offset to read forward from — pass a previous result's `nextOffset` to stream new output only.                                 |
| `wait`            | bool    | `false` | Follow the session live from `offset` instead of returning immediately — the same knob, with the same meaning, as `spawn`'s `wait`. |
| `timeout_seconds` | integer | —       | Caps a `wait` follow, in seconds. Inert without `wait: true`. Omit to follow until the agent finishes or you cancel.                |

A follow streams each new chunk as an MCP `notifications/progress` message when the caller's request carries a `progressToken`; without one it degrades to a plain long poll and the caller still gets everything in the final result. It ends on whichever comes first: the agent finishing, the caller cancelling the request, or `timeout_seconds` elapsing — there's no other server-side time limit.

## `session_send` semantics

`session_send` doesn't require the target session to still be alive — it inspects the handle's status and does whatever's needed:

- **Refused** if the session is still `running` — no vendor supports two concurrent turns on one conversation. Wait for it (poll `session_read`) or `session_kill` it first.
- **Refused** if the session never reported a vendor session id — its first turn likely failed before the agent even started; check `session_read`.
- Otherwise it **resumes**: the vendor's own session store continues the conversation, but in a **new process**, which means a **new handle**. The result's `delivery` field reports `"resumed"`, `resumedFrom` carries the old handle, and `session` carries the one to use from now on — the old handle's process has already exited, so continuing to address it just reads a dead session's transcript.

## Session lifetime

Every session is a **direct child** of `hyprpilot mcp serve`, not a daemon of its own: a `tokio::process::Child` waited on in-process, with its transcript streamed into a per-session, owner-only (`0700`) temp directory. That has one hard consequence — **sessions die with the server, and their transcripts die with them.** Restarting the sidecar, for any reason (the vendor restarting it, the host process exiting, a crash), doesn't preserve a single running or finished session: there's no persistence and no state across launches. Treat a `spawn`/`session_send` chain as living only as long as the MCP connection that started it; if a result needs to survive that boundary, capture it before the connection ends.

On a graceful transport close, or on `SIGTERM`/`SIGHUP`, the server kills every live session (SIGTERM the process group, a grace period, then SIGKILL if it didn't listen) and removes its temp directory before exiting — a clean shutdown never leaves an orphan behind.

## Orphan prevention

A crashed or forcibly-killed sidecar is a different story from a clean shutdown, so orphan prevention is layered — only the last layer is a guarantee:

1. **Graceful shutdown** — the path above. Userspace, so it only runs if the process gets a chance to run destructors at all.
2. **tokio's drop guard** (`kill_on_drop`) — without it, tokio's default behavior for a dropped, still-running child is to push it onto a global orphan queue rather than kill it, which is precisely the failure this exists to prevent. Still userspace.
3. **`PR_SET_PDEATHSIG`** (Linux only) — the kernel kills the child when the sidecar dies, _however_ it dies. This is the only layer that survives a `SIGKILL` of the sidecar, or the release build's `panic = "abort"`, both of which run no destructor at all.

Each session also runs in its **own process group**, so a kill from `session_kill` or graceful shutdown signals the whole group — reaching everything the vendor itself spawned (its own MCP subprocesses, tool calls), not just the direct child.

**PDEATHSIG is the exception, and it matters.** It signals only the *direct* child, and is cleared across that child's own forks. So in exactly the case layer 3 exists for — the sidecar `SIGKILL`ed or aborted, with no chance to signal anything — the vendor dies but **its grandchildren can survive** until the next `--with-harness` sidecar sweeps them. Layers 1 and 2 cover the group; layer 3 covers only the child.

Because PDEATHSIG is Linux-only, the guarantee degrades elsewhere to the first two layers, both of which a `SIGKILL` of the sidecar defeats.

A **startup sweep** (run once, only under `--with-harness`, before the server starts serving) covers what none of the three layers can: a machine crash, or the surviving grandchildren described above. It scans the temp directory for leftover session directories, kills any process group still alive (recorded in a crash-recovery breadcrumb written at spawn time), and removes the directory — logging a warning whenever it reclaims something, since a non-empty sweep means a previous sidecar died badly.

The sweep only reclaims sessions whose **owning sidecar is gone**. Each breadcrumb records the pid of the sidecar that created it, and the sweep skips any directory whose owner is still alive — or whose ownership it cannot establish. Running two harness sidecars at once is an ordinary setup, and without that check the newcomer's "recovery" would kill the other's live agents and delete transcripts still being written.

## Limits

| Limit                       | Value        | Enforced by                                                                                         |
| --------------------------- | ------------ | --------------------------------------------------------------------------------------------------- |
| Concurrent running sessions | 8            | `spawn` refused past the ceiling; `session_kill` a finished or runaway one to free a slot.          |
| Spawn nesting depth         | 2            | `HYPRPILOT_SPAWN_DEPTH` env, stamped `depth + 1` on every spawned session; `spawn` refused past it. |
| Transcript read per call    | 60,000 bytes | Caps `session_read` and an inline `spawn`/`session_send` result.                                    |
| Default tail                | 200 lines    | `session_read`'s default when `offset` is omitted.                                                  |
| Default turn timeout        | 300 seconds  | `spawn`/`session_send`'s `wait: true` default before the result reports status `running`.           |

The depth ceiling exists because a session started through the harness could itself be another `hyprpilot mcp serve --with-harness` sidecar — the env stamp bounds that chain regardless of how deep the concurrency ceiling alone would otherwise allow it to go. The concurrency ceiling bounds breadth at any single depth: since a profile's `command` can be any binary, an agent that could spawn without limit could exhaust the host.

## Example config

Because the auto-injected `hyprpilot` entry never carries `--with-harness`, enable it through your own `mcps` entry, under a different server name:

```json
{
  "mcpServers": {
    "hyprpilot-harness": {
      "command": "hyprpilot",
      "args": ["mcp", "serve", "--with-harness", "--skill-dir", "{\"dir\":\"/home/you/.config/hyprpilot/skills\",\"ignore\":[]}"]
    }
  }
}
```

```yaml
profiles:
  - id: gateway
    agent: claude-code
    mcps:
      - file: ~/.config/hyprpilot/mcps/harness.json
```

Only give a profile this MCP entry when you actually want it driving other hyprpilot sessions — see [gating the harness](#gating-the-harness) above for why.

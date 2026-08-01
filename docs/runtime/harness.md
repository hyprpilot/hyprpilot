---
title: Agent Harness
order: 60
next: false
---

# {{ $frontmatter.title }}

`hyprpilot mcp harness` is a control plane for hyprpilot itself: a connected agent can list your configured profiles, launch one as its own agent session, talk to it across turns, watch it work, and kill it. It is its own stdio server, alongside [Skills](./skills) and the general-tools server — separate process, separate catalogue entry, separate tool policy.

<!-- more -->

## What it solves

Without the harness, an agent connected to hyprpilot's MCP servers can only read skills — it has no way to act as an orchestrator that spins up other hyprpilot sessions. The harness adds that: discover profiles, start one, send it follow-up turns, follow its output live, stop it. Every launch flows through the same `spawn::prepare` path `hyprpilot <profile>` itself uses, so profile resolution, the `-- <args>` escape hatch, and cwd precedence can't drift between a CLI launch and a harness-driven one — see [Launching](./launch).

## Gating the harness

```yaml
mcp:
  harness:
    enabled: true
```

**Off by default, deliberately.** hyprpilot auto-injects its in-tree servers into **every** launch (see [Skills → Auto-injection](./skills#auto-injection)), so an ungated spawn surface would let any agent session spawn nested sessions with no bound. A profile's `command` is an arbitrary binary — its `provider` only picks which native-flag projection applies, not a sandbox — so anything that reaches `spawn` executes commands as whoever runs the sidecar. Enable it only where that's intended, e.g. a gateway host that opts in explicitly.

The gate is **structural** within the served surface. The harness is its own subcommand and its own `ServerHandler`, so the skills server has no `spawn` implementation to reach even if a client guesses the name. (An earlier design put these tools on the skills server behind a `--with-harness` flag, which meant remembering to gate both `list_tools` and `call_tool` — dispatch is by name, so gating only the listing would have left every tool callable. A reviewer caught exactly that half-missing gate.)

::: warning What the gate is not

`mcp.harness.enabled` controls whether **hyprpilot** injects the entry — it bounds what a connected agent can _discover_, not what it can _do_. `hyprpilot mcp harness` is an ordinary subcommand: run it and it serves `spawn`, whatever your config says. That is deliberate — a hand-configured MCP entry (a gateway managing its own catalogue) must work without a hyprpilot config to consult, and `mcp` deliberately skips config validation so a broken config can't kill a sidecar the vendor keeps respawning.

So this is **exposure reduction, not a capability boundary**. Against an agent that can run shell commands it buys nothing: such an agent can start the harness itself, or skip it entirely and run `hyprpilot <profile>`. It is a real boundary only against a client whose sole reach is MCP — which is the case worth protecting, since that is what an MCP-only gateway looks like.

:::

Because it is a separate catalogue entry, it also gets its own tool policy — worth tightening, since `spawn` is the tool that runs arbitrary binaries:

```yaml
mcp:
  harness:
    enabled: true
    autoAcceptTools:
      - list_profiles
      - session_read
    autoRejectTools:
      - spawn
```

## Which profiles it can drive

The harness runs **only the profiles you nominate.** A profile is available when it declares a `harness` block; without one it is absent from `list_profiles` and refused by `spawn` / `session_send`:

```yaml
profiles:
  - id: personal/engineer
    agent: claude-code
    harness:
      enabled: true
```

Opt in a whole family with a `$match`ed [patch](../config/patches) rather than repeating it:

```yaml
patches:
  - $match:
      profile: 'personal/*'
    harness:
      enabled: true
```

Default-deny because `spawn` runs a profile's `command` as you. See [Profiles → Putting a profile on the harness](../config/profiles#putting-a-profile-on-the-harness).

## The tools

| Tool             | Purpose                                                                                   |
| ---------------- | ----------------------------------------------------------------------------------------- |
| `list_profiles`  | Discover the profiles you can launch — vendor, model, effort, mode, cwd. Start here.      |
| `spawn`          | Start a new session from a profile and send it a prompt.                                  |
| `session_send`   | Send another message to an existing session, resuming it first if it's finished.          |
| `session_list`   | List this server's sessions — handle, profile, status, exit code, timestamps.             |
| `session_status` | One session's state without its transcript — the cheap poll.                              |
| `session_read`   | Read, and optionally follow live, a session's transcript.                                 |
| `session_kill`   | Stop a running session and everything it started — or reap one that has already finished. |

### Workflow

1. **`list_profiles`** to find an `id` — a row marked `!` failed to resolve; don't launch it.
2. **`spawn { profile, prompt }`** to start a session. With `wait` true (the default) it blocks and returns the transcript; if the turn outlives `timeout_seconds` the result comes back with status `running`, a `nextCursor` to resume reading from, and the agent **keeps working**.
3. If status is `running`, poll or follow **`session_read { session, wait: true }`** — do **not** call `spawn` again for the same conversation.
4. **`session_send { session, prompt }`** for every follow-up turn, once the session has finished its previous one.
5. **`session_kill { session }`** to stop a runaway agent, or to free a slot when `spawn` reports the concurrency limit. It is state-aware, like `session_send`: on a **running** session it terminates the agent and keeps the transcript, so you can still read why; on an **already-finished** one it reaps the session and its transcript. Calling it twice is the natural stop-then-clean-up, and the result's `action` says which happened.
6. **`session_status { session }`** is the cheap way to answer "is it done yet" — it reads no transcript, and `transcriptBytes` tells you whether a running agent is progressing or wedged, which `status` alone cannot.
7. **`session_list`** any time you need to recover a handle you lost.

### `session_status`

| Field             | Type   | When           | What it means                                                                           |
| ----------------- | ------ | -------------- | --------------------------------------------------------------------------------------- |
| `status`          | string | always         | `running` or `exited`. A session is `exited` after every **turn**, not only at the end. |
| `exitCode`        | int    | once exited    | Omitted while running.                                                                  |
| `transcriptBytes` | int    | always         | Bytes written so far. A number that stops moving is a wedged agent.                     |
| `hasResult`       | bool   | always         | Whether the agent's final answer has landed — see below.                                |
| `vendorSessionId` | string | once harvested | Omitted until the vendor emits it.                                                      |

`hasResult` is `false` for any running session, and only then scanned per vendor. Both halves matter:

- opencode emits a `text` part for **every** completed sentence, not just the final answer, so content alone cannot say "done".
- `session_send` **appends** to one transcript, so a scan from the start keeps finding the first turn's marker and every later turn would read as finished the moment it began. The scan reads the tail, scoping it to the latest turn.

The three vendors mark completion differently — all verified against the installed CLIs:

- **claude** — a terminal `{"type":"result"}` carrying the answer.
- **codex** — `{"type":"turn.completed"}` closes the turn; the text rode the `item.completed` before it, whose `item.type` is `agent_message`.
- **opencode** — emits **no** terminal marker at all. Its stream ends `step_finish(reason=stop)`, so the last `{"type":"text"}` part is the signal.

### Watching from a shell

Every session directory gets a `done.json` when its turn's process exits, written by the same `child.wait()` task that owns the truth — so no recycled PID and no zombie can produce a false reading. Its path rides on `spawn` / `session_send` / `session_read` results as `sessionInfo.donePath`.

This is the vendor-neutral completion signal, and the one a **shell** watcher can use, since a bash loop cannot call an MCP tool:

```bash
[ ! -d "$SESSION_DIR" ] || [ -f "$SESSION_DIR/done.json" ]
```

Both halves are required. The marker is advisory: reaping, eviction and shutdown all remove the directory, so a watcher that only tests for the file waits forever on a session that was cleaned up.

`{"handle": "…", "exitCode": 0, "finishedAt": 1785584247}`

It is **cleared when a turn starts**, not only written when one ends — `session_send` reuses the handle and directory, so a watcher armed for the next turn would otherwise fire instantly on the previous turn's leftover.

### Completion notifications (Claude Code channels)

When a turn's process exits the harness pushes a `notifications/claude/channel` event, which Claude Code turns into a `<channel source="hyprpilot_harness">` block in the lead agent's next turn:

```txt
content: hyprpilot harness session 4670d5aa… finished (exit 0). Read its output with session_read.
meta:    { session: "4670d5aa…", exit_code: "0" }
```

On by default. It is safe to leave on — a client that has not registered the channel drops the notification silently, and unknown capabilities are ignored per the MCP spec, so nothing errors anywhere. The knob exists for **noise**: a session is `exited` after every _turn_, so a ten-turn conversation emits ten events.

```yaml
mcp:
  harness:
    notifyOnComplete: false
```

Resolved by the **launcher**, from the profile it picked, and passed to the sidecar as a flag — the same way `maxSessions` arrives. A sidecar cannot work out which profile spawned it, so it cannot read this from config itself.

Two things worth knowing:

- **Registering the channel is the client's job, not hyprpilot's.** Claude Code only listens for channels it was launched with; that is your own launch configuration. hyprpilot declares the capability and pushes the event — where channels are unavailable, the push is dropped.
- **The content is a fixed template.** Transcript bytes and agent output are never interpolated into it — that would let a spawned agent write into its parent's context through a path the parent never called. Everything variable rides `meta`, whose keys must be `[A-Za-z0-9_]` (a hyphen is silently dropped, which is why it is `exit_code`).

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

`session_send` **replays the original launch** and does not let you change it. Only `prompt` / `file`, `mode`, `wait` and `timeout_seconds` are per-turn; `cwd`, `args` and `with_config` are inherited from the `spawn` and are **rejected** if passed — start a new session to launch differently.

`mode` is the exception because a per-turn permission change is a real workflow (`mode: plan` for a read-only follow-up) and it does not affect how the vendor looks the conversation up.

How a conversation was launched is part of its **identity**, not a per-turn option — re-deriving a follow-up turn from defaults launched it differently from the first, silently. The visible failure was `cwd`: claude keys its conversation store by project directory, so a resume from elsewhere came back with a bare `No conversation found with session ID: …` for a perfectly healthy session, because it was looked up in the wrong place. A dropped `mode` or `args` is quieter and worse — it changes the agent's permissions or flags mid-conversation without saying anything.

::: warning `with_config` is restricted to `model`, `effort` and `mode`
Unlike the CLI's `--with-config`, the harness accepts only those three keys — an allow-list, not a block-list. A profile overlay can otherwise reach `command`, `args` and `env` (which replace the binary outright), `mcps` (whose inline `mcp_servers` entries carry their own `command`/`args`, which the vendor then spawns), `$deleteFromPrimitiveList/<field>` directives (which mutate a field without ever naming it, e.g. stripping a profile's `--sandbox`), and `system_prompt` (which reads an arbitrary file into the agent's context). Any of those turns `spawn` into arbitrary command execution as the sidecar's user.

Enumerating the ways _in_ is a losing game against a config tree that grows; enumerating what's allowed is not. To run something else, add a profile for it in the hyprpilot config — that is the captain's decision to make, not the calling agent's.
:::

### `session_read` parameters

| Parameter         | Type    | Default | What it does                                                                                                                        |
| ----------------- | ------- | ------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| `session`         | string  | —       | Required. Handle from `spawn` or `session_list`.                                                                                    |
| `tail`            | integer | `200`   | Trailing lines to return when `cursor` is omitted.                                                                                  |
| `cursor`          | string  | —       | Opaque pagination cursor — pass a previous result's `nextCursor` verbatim to continue where it stopped.                             |
| `wait`            | bool    | `false` | Follow the session live from `cursor` instead of returning immediately — the same knob, with the same meaning, as `spawn`'s `wait`. |
| `timeout_seconds` | integer | —       | Caps a `wait` follow, in seconds. Inert without `wait: true`. Omit to follow until the agent finishes or you cancel.                |

**Pagination follows the MCP idiom.** `cursor` in, `nextCursor` out, opaque both ways — pass one back verbatim, never parse or construct one. **An absent `nextCursor` means the session is finished and you have all of it**; a running session always returns one, so a poller never loses its place. An unrecognised cursor is an error rather than a silent reset. There is no `truncated` flag: the cursor's presence is the signal.

A follow streams each new chunk as an MCP `notifications/progress` message when the caller's request carries a `progressToken`; without one it degrades to a plain long poll and the caller still gets everything in the final result. It ends on whichever comes first: the agent finishing, the caller cancelling the request, or `timeout_seconds` elapsing — there's no other server-side time limit.

## `session_send` semantics

`session_send` doesn't require the target session to still be alive — it inspects the handle's status and does whatever's needed:

- **Refused** if the session is still `running` — no vendor supports two concurrent turns on one conversation. Wait for it (poll `session_read`) or `session_kill` it first.
- **Refused** if the session never reported a vendor session id — its first turn likely failed before the agent even started; check `session_read`.
- Otherwise it **resumes**: the vendor's own session store continues the conversation in a new process, and the result's `delivery` field reports `"resumed"`. **The handle does not change.** It stays valid for the whole conversation however many turns you send, and each turn appends to the same transcript — so `session_read` offsets stay meaningful across turns, and an N-turn conversation costs one session, not N.

## Session lifetime

Every session is a **direct child** of `hyprpilot mcp harness`, not a daemon of its own: a `tokio::process::Child` waited on in-process, with its transcript streamed into a per-session, owner-only (`0700`) temp directory. That has one hard consequence — **sessions die with the server, and their transcripts die with them.** Restarting the sidecar, for any reason (the vendor restarting it, the host process exiting, a crash), doesn't preserve a single running or finished session: there's no persistence and no state across launches. Treat a `spawn`/`session_send` chain as living only as long as the MCP connection that started it; if a result needs to survive that boundary, capture it before the connection ends.

On a graceful transport close, or on `SIGTERM`/`SIGHUP`, the server kills every live session (SIGTERM the process group, a grace period, then SIGKILL if it didn't listen) and removes its temp directory before exiting — a clean shutdown never leaves an orphan behind.

## Orphan prevention

A crashed or forcibly-killed sidecar is a different story from a clean shutdown, so orphan prevention is layered — only the last layer is a guarantee:

1. **Graceful shutdown** — the path above. Userspace, so it only runs if the process gets a chance to run destructors at all.
2. **tokio's drop guard** (`kill_on_drop`) — without it, tokio's default behavior for a dropped, still-running child is to push it onto a global orphan queue rather than kill it, which is precisely the failure this exists to prevent. Still userspace.
3. **`PR_SET_PDEATHSIG`** (Linux only) — the kernel kills the child when the sidecar dies, _however_ it dies. This is the only layer that survives a `SIGKILL` of the sidecar, or the release build's `panic = "abort"`, both of which run no destructor at all.

Each session also runs in its **own process group**, so a kill from `session_kill` or graceful shutdown signals the whole group — reaching everything the vendor itself spawned (its own MCP subprocesses, tool calls), not just the direct child.

**PDEATHSIG is the exception, and it matters.** It signals only the _direct_ child, and is cleared across that child's own forks. So in exactly the case layer 3 exists for — the sidecar `SIGKILL`ed or aborted, with no chance to signal anything — the vendor dies but **its grandchildren can survive** until the next harness sidecar sweeps them. Layers 1 and 2 cover the group; layer 3 covers only the child.

Because PDEATHSIG is Linux-only, the guarantee degrades elsewhere to the first two layers, both of which a `SIGKILL` of the sidecar defeats.

A **startup sweep** (run once by `mcp harness` before it starts serving) covers what none of the three layers can: a machine crash, or the surviving grandchildren described above. It scans the temp directory for leftover session directories, kills any process group still alive (recorded in a crash-recovery breadcrumb written at spawn time), and removes the directory — logging a warning whenever it reclaims something, since a non-empty sweep means a previous sidecar died badly.

The sweep only reclaims sessions whose **owning sidecar is gone**. Each breadcrumb records the pid of the sidecar that created it, and the sweep skips any directory whose owner is still alive — or whose ownership it cannot establish. Running two harness sidecars at once is an ordinary setup, and without that check the newcomer's "recovery" would kill the other's live agents and delete transcripts still being written.

## Limits

| Limit                       | Value                 | Enforced by                                                                                                                      |
| --------------------------- | --------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| Concurrent running sessions | 8                     | `spawn` refused past the ceiling; `session_kill` a finished or runaway one to free a slot.                                       |
| Spawn nesting depth         | 2                     | `HYPRPILOT_SPAWN_DEPTH` env, stamped `depth + 1` on every spawned session; `spawn` refused past it.                              |
| Transcript read per call    | 60,000 bytes          | Caps `session_read` and an inline `spawn`/`session_send` result.                                                                 |
| Default tail                | 200 lines             | `session_read`'s default when `cursor` is omitted.                                                                               |
| Default turn timeout        | 300 seconds           | `spawn`/`session_send`'s `wait: true` default before the result reports status `running`.                                        |
| Retained sessions           | 64 (`--max-sessions`) | Past this, the oldest **finished** sessions are evicted (with their transcripts) and logged. A running session is never evicted. |

Only distinct `spawn`s grow the table — a conversation reuses its session however many turns it runs — so the retention limit bounds a long-lived server's memory and temp directories without a tool you have to remember to call. Raise `--max-sessions` on a busy gateway that wants deeper history; lower it where temp space is tight.

To free a session earlier than the limit would, call `session_kill` on it: on a finished session that reaps it and its transcript immediately.

The depth ceiling exists because a session started through the harness could itself be another `hyprpilot mcp harness` sidecar — the env stamp bounds that chain regardless of how deep the concurrency ceiling alone would otherwise allow it to go. The concurrency ceiling bounds breadth at any single depth: since a profile's `command` can be any binary, an agent that could spawn without limit could exhaust the host.

## Example config

Two independent switches, and you need **both**:

1. `mcp.harness.enabled` — whether the _server_ is injected at all, for the profile doing the orchestrating.
2. `profiles.harness.enabled` — which profiles that server is allowed to _drive_.

```yaml
patches:
  # The gateway profile gets the harness server injected.
  - $match:
      profile: gateway
    mcp:
      harness:
        enabled: true
        maxSessions: 128

  # …and these are the profiles it may launch.
  - $match:
      profile: 'worker/*'
    harness:
      enabled: true

profiles:
  - id: gateway
    agent: claude-code
  - id: worker/engineer
    agent: claude-code
  - id: deploy # neither switch — invisible to the harness
    agent: claude-code
```

Setting only the first gives an agent the tools and an **empty** `list_profiles`; setting only the second nominates profiles nothing can reach. A profile's own `mcp` block works in place of the first patch, but it wholesale-replaces the global one, so you would have to restate `skills` alongside it.

To run it by hand (debugging, or a gateway that manages its own MCP config), the subcommand takes no catalogue:

```sh
hyprpilot mcp harness --max-sessions 64
```

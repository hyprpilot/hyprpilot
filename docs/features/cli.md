---
title: Daemon & CLI
order: 5
---

# Daemon and CLI

Hyprpilot is one binary doing two jobs. Pick the role with a subcommand:

| Command | Role |
| --- | --- |
| `hyprpilot daemon` | Long-running overlay process. Hosts the webview, manages instances, owns the unix socket. |
| `hyprpilot ctl …` | Short-lived CLI client. Sends commands to a running daemon and prints the result. |

Running `hyprpilot` (no subcommand) is shorthand for `hyprpilot daemon`. A second invocation against a running daemon pops the overlay forward instead of starting a new one.

## Daemon

```sh
hyprpilot daemon                            # start with defaults
hyprpilot daemon --cwd ~/code/hyprpilot     # chdir before any setup
hyprpilot daemon --hidden                   # boot with overlay invisible (default behaviour)
hyprpilot daemon --config /path/to.toml     # custom config path
hyprpilot daemon --config-profile work      # pick a config-layer overlay
hyprpilot daemon --log-level debug          # override the log filter
```

Logs land at `~/.local/state/hyprpilot/logs/hyprpilot.log.<date>`. When running under systemd: `journalctl --user -u hyprpilot.service -f`.

## ctl

Every `ctl` command is one round-trip to the daemon's unix socket. They print JSON to stdout on success and exit 0; transport / RPC errors print to stderr and exit 1.

### Overlay

```sh
hyprpilot ctl overlay show                          # show + focus
hyprpilot ctl overlay show --instance <uuid|name>   # show + focus a specific instance
hyprpilot ctl overlay hide                          # hide (webview stays warm)
hyprpilot ctl overlay toggle                        # flip
```

### Prompts

```sh
# Send a prompt to a specific instance.
hyprpilot ctl prompts send --instance my-review "look at the diff"

# Send to the focused instance (auto-spawns under --profile if none focused).
hyprpilot ctl prompts send --profile engineer "show failing tests"

# Pipe stdin — push-to-talk pattern.
whisper-stream | hyprpilot ctl prompts send --stdin

# Append to the composer instead of dispatching — you hit Enter to send.
hyprpilot ctl prompts send --instance review --append "and check the migration"

# Cancel the in-flight turn.
hyprpilot ctl prompts cancel --instance review
```

`--instance` accepts a UUID, an existing instance name, or a fresh slug. A slug that doesn't match any live instance auto-spawns a new one named after the slug — single keybinds like `ctl prompts send --instance scratch "hi"` become "open scratch, creating it if needed".

### Instances

```sh
hyprpilot ctl instances list                                        # list live instances
hyprpilot ctl instances spawn --profile engineer --name review     # spawn + name
hyprpilot ctl instances focus --instance review --present          # focus + show overlay
hyprpilot ctl instances focus --instance new-thing --ensure --profile ask  # focus, spawn if needed
hyprpilot ctl instances rename --instance <uuid> --name pr-23
hyprpilot ctl instances restart --instance review
hyprpilot ctl instances shutdown --instance review
hyprpilot ctl instances info --instance review
```

### Status (waybar surface)

```sh
hyprpilot ctl status            # one-shot — prints a single waybar-shaped JSON object
hyprpilot ctl status --watch    # streams JSON lines per state change; reconnects with back-off
```

Always exits 0 — even when the daemon is down it emits an `"offline"` payload so waybar's `exec` field stays valid. See the [Waybar guide](../guide/waybar) for module config.

### Daemon lifecycle

```sh
hyprpilot ctl daemon status     # pid, uptime, version, instance count
hyprpilot ctl daemon version    # version string
hyprpilot ctl daemon reload     # re-read config + skills + MCPs
hyprpilot ctl daemon shutdown   # graceful — refuses if turns are in flight
hyprpilot ctl daemon shutdown --force
hyprpilot ctl kill              # hard stop (shortcut)
```

### Diagnostics

```sh
hyprpilot ctl diag snapshot                       # structural dump to stdout
hyprpilot ctl diag snapshot --output state.json   # ... or to a file
```

Read-only. Useful for support tickets and "what is this daemon doing" investigation. Profile env values and transcript bodies are redacted.

## Status states

The status stream emits one of:

| `state` | `class` | Meaning |
| --- | --- | --- |
| `idle` | `idle` | Nothing in flight. |
| `streaming` | `streaming` | An agent is responding to a prompt. |
| `awaiting` | `awaiting` | A permission prompt is waiting on you. |
| `error` | `error` | The last turn errored. |
| `offline` | `offline` | The daemon is not running (only `ctl status` reports this). |

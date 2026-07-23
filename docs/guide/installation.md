---
title: Installation
order: 1
---

# Installation

Hyprpilot is a single Rust binary. It resolves a session profile from
layered config and `exec()`s into the vendor's native agent CLI — so the
only hard runtime dependency is the vendor CLI you want to launch
(`claude`, `codex`, or `opencode`).

## Arch

Hyprpilot is published to the AUR in two flavors. Pick one — they
conflict by design.

### Prebuilt binary — `hyprpilot-bin`

```sh
yay -S hyprpilot-bin
```

Tracks the latest GitHub Release.

### Source build — `hyprpilot-git`

```sh
yay -S hyprpilot-git
```

Builds from `main` with `cargo`. No webkit / gtk / node toolchain is
needed — it is a plain Rust build.

## Building from source

```sh
git clone https://github.com/hyprpilot/hyprpilot
cd hyprpilot
cargo build --release
install -Dm755 target/release/hyprpilot ~/.local/bin/hyprpilot
```

The AUR `hyprpilot-git` package also installs the desktop entry and
hicolor icons; a manual build only drops the binary. See
[Development](../repository/development) for the pinned toolchain and
`task` targets.

## The desktop entry

The AUR packages install a terminal-launcher `.desktop` file at
`/usr/share/applications/hyprpilot.desktop`:

```ini
[Desktop Entry]
Type=Application
Name=hyprpilot
GenericName=Agent CLI launcher
Comment=Resolves a profile and execs your coding agent's native CLI.
Exec=hyprpilot
Icon=hyprpilot
Categories=Development;Utility;
StartupNotify=false
Terminal=true
```

`Terminal=true` means the desktop launcher opens a terminal and runs
bare `hyprpilot` — which pops the interactive profile picker, then execs
the chosen vendor CLI in that terminal. Bind it to an app-launcher entry
or a keybind that opens a terminal (see [Integration](./integration)).

## After install

1. **Install a vendor CLI** — hyprpilot launches `claude`, `codex`, or
   `opencode`; at least one must be on your `$PATH`.
2. **Configure at least one profile** — drop `[[profiles]]` (and the
   agent it references) into `~/.config/hyprpilot/config.toml`. Fresh
   installs ship agent defaults but **no** profile, and hyprpilot
   refuses to launch until one exists. See
   [Configuration → Profiles](../configuration/profiles).
3. _(Optional)_ **Wire it into your compositor** — a keybind, an
   app-launcher entry, or the terminal `.desktop`. See
   [Integration](./integration).

## Running it

```sh
hyprpilot                       # pick a profile interactively, then exec
hyprpilot -p engineer           # launch the `engineer` profile directly
hyprpilot profiles              # list configured profiles
hyprpilot --help
```

Because hyprpilot `exec()`s into the vendor CLI, it inherits your shell
environment as-is — API keys, `$PATH`, and everything else the vendor
needs are already present when you run it from a terminal. There is no
long-lived process to keep hydrated.

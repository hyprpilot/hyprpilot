---
title: Release
order: 2
---

# Releases

Hyprpilot ships through the [Arch User Repository](https://aur.archlinux.org) in two flavors:

- **`hyprpilot-bin`** — prebuilt binary, fastest to install, tracks tagged releases.
- **`hyprpilot-git`** — builds from the latest `main` with `cargo`, for people who want the bleeding edge.

You don't need to do anything to get a new version; once changes land on `main`, a new release rolls out to the AUR on its own. `yay -S hyprpilot-bin` (or your AUR helper of choice) picks it up the next time you upgrade.

## What about other distros?

Right now hyprpilot only publishes for Arch and Arch-likes. The binary itself is a plain Rust build with no webkit / gtk / node dependency, so building from source on other distros is straightforward — see [Development](./development.md) for the toolchain. If you'd like to maintain a package for another distro, that'd be very welcome — open an issue and we'll help where we can.

## Something broken in a release?

If a published version misbehaves on your machine — the picker won't open, a launch fails, the wrong vendor flags get projected, anything — please [open an issue](https://github.com/hyprpilot/hyprpilot/issues) with your distro, the vendor CLI + version, and the relevant log snippet (`--log-level debug`). The faster we hear about it, the faster the next release fixes it.

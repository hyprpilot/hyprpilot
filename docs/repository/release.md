---
title: Release
order: 30
---

# {{ $frontmatter.title }}

Releases are fully automated: conventional commits drive [release-please](https://github.com/googleapis/release-please), and every published release rolls out to the [Arch User Repository](https://aur.archlinux.org) on its own.

<!-- more -->

## Versioning

Commit types map to version bumps through the standard Angular table:

| Commit                                     | Bump                           |
| ------------------------------------------ | ------------------------------ |
| `feat: …`                                  | minor                          |
| `fix:` / `perf:` / `refactor:` / `docs: …` | patch                          |
| `chore:` / `test:` / `ci: …`               | none (hidden in the changelog) |
| `!` suffix or `BREAKING CHANGE:` footer    | major                          |

release-please maintains a rolling release PR on `main`; merging it tags the version, writes the changelog, and publishes the GitHub Release.

## The AUR pipeline

Publishing a release triggers the release workflow, which builds a Linux x86_64 tarball and pushes the updated **`hyprpilot-bin`** package to the AUR. The **`hyprpilot-git`** PKGBUILD is pushed separately whenever the PKGBUILD itself changes — a VCS package rebuilds from the latest `main` on your machine, so it needs no per-release update.

- **`hyprpilot-bin`** — prebuilt binary, fastest to install, tracks tagged releases.
- **`hyprpilot-git`** — builds from the latest `main` with `cargo`, for the bleeding edge.

You don't need to do anything to get a new version; `yay -S hyprpilot-bin` (or your AUR helper of choice) picks it up the next time you upgrade.

## What about other distros?

Right now hyprpilot only publishes for Arch and Arch-likes. The binary itself is a plain Rust build with no webkit / gtk / node dependency, so building from source on other distros is straightforward — see [Development](./development) for the toolchain. If you'd like to maintain a package for another distro, that'd be very welcome — open an issue and we'll help where we can.

## Something broken in a release?

If a published version misbehaves on your machine — the picker won't open, a launch fails, the wrong vendor flags get projected, anything — please [open an issue](https://github.com/hyprpilot/hyprpilot/issues) with your distro, the vendor CLI + version, and the relevant log snippet (`--log-level debug`). The faster we hear about it, the faster the next release fixes it.

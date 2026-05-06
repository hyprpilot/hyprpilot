---
title: Release
order: 2
---

# Release process

Hyprpilot ships as a single Rust binary, distributed via the Arch User Repository (AUR) in two flavors: `hyprpilot-bin` (prebuilt tarball) and `hyprpilot-git` (VCS source build). Releases are conventional-commit-driven via [release-please](https://github.com/googleapis/release-please).

## Cycle at a glance

```
conventional commits land on main
   ↓
release-please opens a Release PR (CHANGELOG bump + version bump)
   ↓
captain merges the Release PR
   ↓
release-please tags vX.Y.Z + creates the GitHub Release
   ↓
release.yml builds the tarball + uploads to the Release
   ↓
release.yml publishes hyprpilot-bin to AUR
   ↓
hyprpilot-git's PKGBUILD already tracks `main` — yay --rebuild picks it up
```

## Conventional commit → version bump

| Commit type | Version impact |
| --- | --- |
| `feat:` | minor (`0.1.x → 0.2.0`) |
| `fix:` | patch (`0.1.0 → 0.1.1`) |
| `feat!:` / `BREAKING CHANGE:` | major (`0.1.0 → 1.0.0`) |
| `chore:` / `docs:` / `refactor:` / `ci:` / `test:` / `style:` | no bump |

Commits hidden from the changelog (per `.github/release-please-config.json`): `chore`, `test`, `ci`. They still influence the version-bump rules above.

## Workflows

| Workflow | Trigger | What it does |
| --- | --- | --- |
| `release-please.yml` | Push to `main` | Opens / updates the Release PR; on merge, tags + creates the GitHub Release. |
| `release.yml` | `release: published` event | Builds the release tarball, uploads to the GitHub Release, publishes `hyprpilot-bin` to AUR. |
| `package.yml` | Push to `packaging/aur/hyprpilot-git/**` or manual dispatch | Publishes `hyprpilot-git` to AUR. |
| `docs.yml` | Push to `docs/**` | Builds and deploys this site to GitHub Pages. |

## How release.yml gets triggered

GitHub's anti-recursion rule blocks events created via `GITHUB_TOKEN` from triggering downstream workflows — including `release: published`. To avoid needing a Personal Access Token, `release-please.yml` runs a follow-up step that dispatches `release.yml` via `workflow_dispatch` (which IS exempt from the rule):

```yaml
- name: Dispatch release.yml on new release
  if: ${{ steps.release.outputs.release_created == 'true' }}
  env:
    GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
    TAG: ${{ steps.release.outputs.tag_name }}
  run: gh workflow run release.yml --ref main -f tag="$TAG"
```

Needs `actions: write` on the workflow's `permissions` block — that's it. No PAT, no extra secret.

## AUR publish

Both AUR variants use [`KSXGitHub/github-actions-deploy-aur`](https://github.com/KSXGitHub/github-actions-deploy-aur) pinned to `v4.1.3` (the action repo doesn't ship a moving `v4` tag).

Required GitHub secrets:

- `AUR_USERNAME`
- `AUR_EMAIL`
- `AUR_SSH_PRIVATE_KEY` (passphrase-less, registered on aur.archlinux.org)

The first publish per package creates the AUR slot — no manual setup needed beyond having the SSH key on the AUR side.

## Manual retries

If `release.yml` fails partway (e.g. AUR push hangs, tarball upload fails), re-run via `workflow_dispatch`:

```sh
gh workflow run Release --ref main -f tag=v0.1.3
```

The `tag` input lets you target an existing release. `release.yml` checks out that tag, rebuilds, re-uploads, re-publishes — no need to delete + re-tag.

For `hyprpilot-git`:

```sh
gh workflow run Package --ref main
```

No tag input — VCS builds always track current `main`.

## Cutting your first release

Solo dev path:

1. Land conventional-commit changes on `main`.
2. Wait for release-please to open the Release PR.
3. Read the CHANGELOG bump; merge the PR.
4. Watch `release.yml` run on the new tag — confirm the GitHub Release gets the tarball + AUR publish completes.

If anything fails, the workflows log the specific step + error. CLAUDE.md's "Manual verification patterns" section has end-to-end smokes to compare against.

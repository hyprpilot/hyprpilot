---
title: Contributions
order: 20
---

# {{ $frontmatter.title }}

Issues, ideas, and pull requests are all welcome.

<!-- more -->

## Found a bug?

Open a [GitHub Issue](https://github.com/hyprpilot/hyprpilot/issues) and include enough that someone else can reproduce it: what you expected, what actually happened, and a minimal config + profile that triggers it. A stderr snippet from a `--log-level debug` run helps a lot too.

## Have an idea?

[Discussions](https://github.com/hyprpilot/hyprpilot/discussions) is the right place for "would this fit?" or "how would you do this?" — anything where you're sketching rather than reporting. If the conversation lands on something concrete, we can move it to an issue from there.

## Sending a pull request

Fork, branch, push, open a PR against `main`. That's it. Smaller PRs are easier to review and land — one logical change per PR if you can swing it. Don't worry about getting the commit history perfect; we can tidy it on the way in.

Commit messages follow [conventional commits](https://www.conventionalcommits.org/) — they drive the [release automation](./release), so a `feat:` / `fix:` prefix is what turns your change into a version bump.

If you're not sure your idea will be accepted, open a Discussion or Issue first to sanity-check the direction. Saves everyone time.

## Building from source

See [Development](./development) for the toolchain and `task` targets. The pre-push bar is `task build && task lint && task test` — all green.

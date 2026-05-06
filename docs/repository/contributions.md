---
title: Contributions
order: 1
---

# Contributions

Contributions are welcome. Issues, ideas, fixes, features — bring them. The repo conventions below are non-negotiable; everything else is up for discussion.

## Branch & commit conventions

### Branch prefix

| Prefix | Use for |
| --- | --- |
| `feat/` | New capability. |
| `fix/` | Bug fix or correction. |
| `refactor/` | Restructuring without behavior change. |
| `chore/` | Maintenance, deps, tooling. |
| `docs/` | Documentation only. |
| `ci/` | CI workflow / release pipeline changes. |
| `perf/` | Performance work. |

Descriptive partial is **kebab-case**. Example: `feat/composer-attachments`, `fix/permission-modal-z-index`.

### Commit messages

Conventional commits, imperative mood, ≤72-char subject:

```
feat(composer): add caret-anchored autocomplete

Multiple sources (skills, paths, ripgrep, slash commands) walk in
priority order. First detector match owns the response.
```

Optional trailers:

- `refs K-123` — references a Linear issue without closing it.
- `closes K-123` — closes the referenced issue on merge.

**No `Co-authored-by:` trailers. No "Generated with Claude" or similar AI attribution.**

### One logical change per commit

If a diff spans multiple unrelated concerns, split into separate commits. The subject tells what; the body tells why; the diff itself shows what changed where.

## PR workflow

1. **Branch off `main`** — never implement on `main` (branch protection enforces this).
2. **Push the branch** — `origin` is GitHub.
3. **Open a PR** — `gh pr create` or the GitHub UI. The body should:
   - Open with a 1-3 sentence summary.
   - Bullet the logical changes (not the file list).
   - Include a `## Reasoning` section for non-trivial design choices.
   - Include a `## Verification` block with concrete commands you ran + the actual output.
4. **Review** — at least eyeball the diff yourself before flagging for review. Solo dev can merge without external review; the squash-merge button is the only path.
5. **Merge** — squash-only. Auto-deletes the branch.

## Verification expectations

Every PR that changes runtime behavior includes a manual smoke-test block in the description with:

- Concrete commands run.
- The actual literal output (paste it; "should pass" is not evidence).

Examples worth pasting:

- `task build` exit code + last 5 lines.
- `cargo nextest run` summary line.
- `hyprctl layers` excerpt for layer-shell changes.
- `gh run view <id>` summary for CI-visible behavior.

## House rules — code style

These come from `CLAUDE.md` at the repo root; consult it for the full set. The biggest:

- **No backwards-compatibility layers.** When a design stops fitting, delete it and rewire call sites in the same commit. No deprecated method aliases, no shim enums.
- **Stubs panic, they don't pretend.** A feature not yet wired end-to-end uses `unimplemented!("…")` — never round-trips a fake-success response.
- **Comment discipline — terse WHY, never WHAT.** Default to no comments. Comments earn their keep on non-obvious reasons (protocol quirks, data-source disagreements). Restating function names is a deletion target.
- **Names are additive: scope first, noun last.** Members of a scope drop the scope's noun (`useToasts() → { push }`, not `pushToast`).
- **Components compose, don't bag.** When a consumer needs rendering flexibility, accept a slot or render fn — never a structured prop bag of primitives the component pattern-matches over.
- **Traits for open extension points; closed enums for closed sets.** Mix is fine: an enum names the universe, a trait is what consumers hold.

The full list lives in `CLAUDE.md` — keep it open while working.

## Asking for help

- **GitHub Issues** for bugs / features / questions.
- **GitHub Discussions** for design conversations.

There is no Slack / Discord / Matrix room. Issues are the surface.

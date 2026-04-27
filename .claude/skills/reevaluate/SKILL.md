---
name: reevaluate
description: Audit a section of the codebase, walk findings through an interview, ship the resolution as one branch + one MR. Use when the user says "reevaluate <area>", "let's audit <module>", "go through <directory>", "let's improve <section>", or any wording that asks for structured analysis + iteration + delivery on a slice of code. Replays the workflow we used on the adapters audit (rounds 1 + 2): read-only exploration → structured smells report → user-guided interview → single-PR shipout.
interaction: chat
---

## When to invoke

User asks for a structured pass over a section of the codebase — not "fix this bug", not "add this feature", but "look at this area, tell me what's wrong, then we'll iterate". Triggers:

- "reevaluate `<path>`"
- "let's go through `<module>`"
- "audit the `<area>` directory"
- "review `<file>` and tell me what could be better"
- "let's improve `<section>`"
- "we need another pass on `<X>`"

Do NOT use for:
- Specific bug fixes (just fix it).
- Implementation of a known feature (use plan-hard or just implement).
- Code review of already-written changes (use code-review).
- Cross-cutting refactors that affect many areas (too broad — narrow to one section first).

## Phases

The skill runs four phases in order. Each phase has a contract; do not skip ahead.

### Phase 1 — Audit (read-only)

**Goal:** produce a structured report of the section's state.

1. The user has named a directory / module. If they haven't, ask once.
2. Map the file tree:
   - `Glob` for every `*.rs` / `*.ts` / `*.vue` / etc. under the section.
   - `wc -l` (via `Bash`) on the matched files to see size distribution. Big files often hide multiple jobs.
3. Read the major files. For sections >5 files or >2000 LoC, dispatch up to 3 `Explore` subagents in parallel with focused search briefs (e.g. "find every dispatch site for X", "trace the lifecycle of Y", "check if Z is consumed anywhere"). Keep the main thread's context clean.
4. Cross-check usage: `Grep` for external callers of the section's public surface to know what's load-bearing vs. dead.
5. Produce the audit as a single markdown document in chat, with these sections (in this order):
   - **Inventory** — file table with sizes + one-line role per file.
   - **What works well** — 3-5 strengths. Be specific (cite file:line). This sets the baseline.
   - **Smells worth flagging** — numbered (S1, S2, …), each with: short name, one-paragraph explanation, file:line evidence, severity tag (high / medium / low impact). Don't pad — only real findings.
   - **Opportunities (table)** — `# | Tag | Action | Payoff | Risk` — one row per smell, ordered by dependency (which fixes unblock others).
   - **Critical files** — paths to consult when acting on findings.

**Length target:** the audit fits in one chat message. ~500-1000 words. If it's longer, you're padding.

### Phase 2 — Interview (plan-hard style, user-guided)

**Goal:** walk every smell branch with the user, accept a decision per branch, log it.

1. Enter plan mode (`EnterPlanMode`). The plan file lives at `~/.claude/plans/<date>-<repo>-reevaluate-<slug>.md` (or whatever the harness assigns).
2. **Before touching code, write the audit to the plan file as the "Audit" appendix.** The Design Decisions section appends as the user answers questions.
3. **Ask one focused question per turn**, in this format:

   > **Question:** <the single decision>
   >
   > **Recommended:** <your pick> — <one-line rationale>.
   >
   > **Alternatives:**
   > - **Option B** — <one-line trade-off>
   > - **Option C** — <one-line trade-off>
   >
   > **Depends on:** <prior decisions, if any>
   > **Cascades into:** <which other smells/branches this resolves or reshapes>

4. **Always recommend.** Saying "what do you want?" is a failure mode. Even when you're unsure, pick the option you'd defend and explain why.

5. **Self-answer when possible.** Before asking, check: can the codebase answer this? (Read the file, grep usage, check git log.) If yes, decide and **report the finding briefly** in the same turn — don't interrupt the user with questions whose answers are obvious from inspection.

6. **Pitfall flag pattern.** When you notice a related concern that's not the current branch but the user should know about, surface it inline as `**Before continuing — one thing worth flagging:** …`. It's *information*, not a new branch unless the user explicitly opens it.

7. **Accept deviations gracefully.** When the user picks something different from your recommendation:
   - Accept the answer.
   - Ask one clarifying question only if the reasoning is non-obvious (e.g., "you picked X — is that because of Y, or a different reason?").
   - Update your mental model. Do **not** re-litigate.
   - If the deviation cascades into other branches, flag the ripple before continuing.

8. **Topic-by-topic ordering.** Walk smells in dependency order from the audit table. After each accepted decision, write it to the plan file's Design Decisions section before asking the next question.

9. **Stop conditions:** continue interviewing until one of:
   - All branches resolved (every smell has an accepted answer).
   - User says `g` / `go` / `y` / `yolo` / "good" / "proceed".
   - User says "quick plan" / "just outline it" — wrap with what's resolved.

### Phase 3 — Execute (single branch + single MR)

**Goal:** ship the resolved plan as one merge request via CLI tools.

1. **Branch off `main`.** Convention from the hyprpilot repo: `<type>/<descriptive-kebab-case>` (`refactor/`, `fix/`, `feat/`, `chore/`). Don't tie to a Linear issue unless the user has one.
2. **Implement in dependency order from the audit table.** Run `cargo check` (or repo equivalent) after each logical chunk so failures localize.
3. **Verify on every PR**:
   - Rust: `cargo nextest run --manifest-path src-tauri/Cargo.toml`, `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`, `cargo fmt --all`.
   - UI: `pnpm --filter hyprpilot-ui exec vue-tsc --noEmit`, `pnpm --filter hyprpilot-ui test`.
   - **Pre-existing UI test failures**: this repo carries known flakiness (~19 tests on node 20). Verify parity with `git stash` against `main` before declaring failures "from your changes". Document the baseline in the MR description; don't gate on it.
4. **Single commit (or grouped commits) per logical unit.** No "fix typo" amendments — squash before pushing if the history got messy.
5. **CLI tools, not MCP, when the harness blocks MCP write actions:**
   - `git` for branch / commit / push.
   - `glab mr create --squash-before-merge --remove-source-branch --title ... --description ...` for the MR.
   - User's standing rule: prefer MCP, but `glab` / `git` CLI is the working fallback.
6. **MR description structure** (from the gitlab-mr conventions):
   - 1-3 sentence summary.
   - Bulleted list of logical changes (per smell → per bullet).
   - **## Reasoning** section: why these design choices, what alternatives were rejected.
   - **## Appendix**: verification output (test counts, lint clean), pre-existing flakiness disclaimer, manual smoke checklist.

### Phase 4 — Verify + iterate on regressions

**Goal:** when the user reports a runtime issue or test regression after merge / on the open branch, fix on the **same** branch.

1. Read the failure carefully — log line, panic site, test name.
2. **Trace from the symptom to the cause.** Don't assume. Check git diff to see if your recent changes touched the involved code path.
3. Common patterns to check:
   - **Tokio runtime context** — `tokio::spawn` panics with "no reactor running" inside Tauri's sync `setup` closure. Use `tauri::async_runtime::spawn` there.
   - **Hard-coded assumptions** — `const fn` returning a literal that's wrong for some inputs. Replace with proper detection (`mime_guess`, `infer`, etc.).
   - **State that doesn't reset** — phase / busy / queue computations gating on "ever happened" instead of "currently active". Gate on the live signal (open turn, in-flight request).
4. Fix + commit + push to the same branch (the open MR auto-updates).
5. Add a regression test pinning the bug. Name it descriptively (e.g., `returns_idle_in_between_turns_even_when_prior_agent_turns_exist_queue_stuck_regression`).
6. Run the full verify cycle again before reporting "fixed".

## Key principles

- **Audit first, recommend second.** The user can't choose between alternatives you haven't shown them. Lay out the smell + 2-3 options + your pick + why.
- **One question per turn, every turn.** Multiple questions overwhelm; the user picks one and silently drops the rest. Worse than asking nothing.
- **Always recommend.** "What do you want?" is the failure mode. Pick + defend.
- **Self-answer aggressively.** The user's time is expensive; codebase reads are cheap. Only escalate decisions of intent / preference / future direction.
- **Update the plan file as you go.** Decisions land in the file the moment they're accepted, not at the end. If the session crashes, the file is the authoritative trace of what was decided.
- **Single PR, single branch.** No "split into 4 PRs" unless the user explicitly asks. The user merges one MR; that's the unit of delivery.
- **Pre-existing failures get documented, not fixed.** This repo has UI test flakiness from a node-version mismatch. Don't try to fix it inside an unrelated MR — call out the baseline in the description and move on.
- **Stay flexible on reshapes.** When the user pushes back on a recommendation ("we don't need this, just X" / "this is overkill"), often they're right — the codebase has constraints you don't fully see. Restructure quickly, don't defend the original answer.
- **Pitfall flags ≠ branches.** When you notice a tangential concern, raise it inline as info. Don't expand the scope unless the user opens the branch explicitly.

## Output format reference

### Audit smell entry

> **S3 — `Adapter` trait is trait-by-name only (medium impact)**
>
> The trait advertises transport-agnosticism but the consumers tell a different story:
>
> - 17/17 dyn-Adapter call sites outside `adapters/` first pull `Arc<AcpAdapter>` from state, then cast at the trait call.
> - `RpcState` (`rpc/server.rs:32-33`) and `HandlerCtx` (`rpc/handler.rs:42-49`) carry **both** `Arc<dyn Adapter>` AND `Arc<AcpAdapter>`.
> - Trait has 7 methods; concrete `AcpAdapter` adds 14+.
>
> The `dyn` is a fiction — every handler that needs anything beyond the trait surface reaches for the concrete.

### Interview question

> **Question:** What's the disposition for the `Adapter` trait?
>
> **Recommended:** **Drop it.** Hold `Arc<AcpAdapter>` everywhere. `HandlerCtx` + `RpcState` collapse to a single field. The layering wall (`no_acp_imports_outside_adapters` test) stays. When HTTP lands, re-introduce the trait with two impls' worth of constraints to design against.
>
> **Why:** CLAUDE.md says *"traits pay their way when new implementers arrive"*. We have one. Speculative trait-building risks getting the abstraction wrong.
>
> **Alternatives:**
> - **Push more onto the trait** — list_sessions / load_session / restart_with_cwd / etc. each gated by `Capabilities` flags, default `Unsupported`. Trade: HTTP slots in cleaner. Cost: speculative method shapes.
> - **Status quo + doc** — keep dual-handle, add an explanatory comment. Trade: zero risk. Cost: every PR pays the dual-handle tax.
>
> **Depends on:** your read of HTTP-adapter timeline.
> **Cascades into:** S4 (collapses), S5 (cleaner without trait indirection), `HandlerCtx` shape.
>
> Drop, push, or status-quo?

## Composing with other skills

- **`plan-hard`** — `reevaluate` borrows its interview discipline (recommendation format, one-question-per-turn, self-answer rule, accept deviations). Don't load it separately; this skill already incorporates it.
- **`gitlab-mr`** — invoke for the Phase 3 MR description if you need full template handling. Otherwise, `glab mr create` with the inline description is fine.
- **`git-branch`** / **`git-commit`** — informal use is enough; the skill calls `git` directly.
- **`code-review`** — different mode. Use `code-review` for "review my recent changes against guidelines"; use `reevaluate` for "audit + improve this section from scratch".

## Examples

**User:** *"let us start here `src-tauri/src/adapters` analyze this how it is structured what can be done better"*

→ Phase 1: 4 explore reads in sequence (or 3 parallel `Explore` agents for a 9000-LoC tree); audit lands in chat with 8 smells (S1-S8) ranked by impact + dependency. Phase 2 enters plan mode, walks each smell with one focused question per turn, captures decisions. User pushes back on S3 ("we should do HTTP eventually") → swap recommendation to "speculative trait expansion". Phase 3: single branch, one commit per logical unit, MR via `glab`. Phase 4: when user reports daemon panic on boot, trace to `tokio::spawn` in Tauri's sync setup → fix to `tauri::async_runtime::spawn` → push to same branch.

**User:** *"reevaluate the rpc handlers"*

→ Same shape, scoped to `src-tauri/src/rpc/handlers/`. Audit notes the dual-handle pattern across 12 files, the inconsistency between `Arc<dyn Adapter>` and `Arc<AcpAdapter>` storage, the `unimplemented!()` stubs vs. `Err(internal_error)` stubs. Interview proposes consolidations. Ship as one branch, one MR.

**User:** *"can we go through ui/src/composables and clean it up"*

→ Survey first (the duplicate-elimination outcome here is *typically* "the existing factoring is already clean — the win is in the wire contract"). Audit reflects the truth: don't manufacture smells that aren't there. If there are no real smells, **say so** and stop after Phase 1. Don't enter Phase 2 just because the skill is loaded.

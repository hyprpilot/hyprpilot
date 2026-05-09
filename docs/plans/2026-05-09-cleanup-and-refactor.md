# Cleanup + refactor plan

**Status:** plan-hard self-walked. Decisions resolved. Ready to execute.

**Source:** five parallel reviewer agents covered:
1. `src-tauri/src/adapters/`
2. `src-tauri/src/rpc/`
3. `src-tauri/src/daemon/` + `tools/` + `completion/` + `skills/` + `mcp/` + `remote/` + `logging.rs` + `paths.rs` + `ctl/`
4. `ui/src/composables/` + `ui/src/ipc/`
5. `ui/src/views/` + `ui/src/components/` + `ui/src/lib/` + `ui/src/constants/wire/` + `ui/src/interfaces/wire/`

Plus a sixth that refactored `CLAUDE.md` (2551 → 1339 lines, already in working tree).

Total findings across the five reviews: ~86.

---

## Decision rule

Every finding gets one of three labels:

- **GO** — fixed in this PR. Mechanical, low-risk, or fixes a real bug.
- **TEST** — write a regression test. Pin the contract before / instead of refactoring.
- **DEFER** — needs its own plan + PR. Documented as future work; not touched here.

The PR limits itself to GO + TEST. Defers stay in this doc + a follow-up file
issue.

---

## Real bug found by review (highest priority)

**B1 — `mergeWireToolCall` divergence between live patch and snapshot replay**

Daemon CONCATS (`src-tauri/src/adapters/acp/instance.rs:993-995`:
`running.content.extend(arr.iter().cloned())`). The wire `ToolCallUpdate`
ships per-delta `content`, never the merged accumulated array. So UI
consumers MUST concat to reconstruct the merged view that the daemon's
formatter produces.

`ui/src/composables/instance/snapshot-timeline.ts:277-279` concats
(correct). `ui/src/composables/instance/use-chat-viewport.ts:213-216`
replaces wholesale (BUG — drops earlier content blocks on every
tool_call_update event during live streaming).

**GO.** Extract a shared `mergeWireToolCall(target, incoming)` helper in
`ui/src/lib/tools/merge-tool-call.ts`. Both consumers call it. Helper
**concatenates** content arrays. Unit test pins "content concatenates,
never replaces", with both an isolated test and a regression that runs
the same input through the live-patch path and the snapshot-replay path
and asserts byte-equal output.

---

## GO: structural fixes (real wins)

| ID | Source | What | Why |
|----|--------|------|-----|
| G1 | A-F4, D-F2 | Type the daemon's wire-list payloads (`AgentSummary` / `ProfileSummary` / `InstanceListEntry` / `BootSnapshot.{agents,profiles,instances,completionConfig}`). Replace hand-rolled `serde_json::json!()` macros with `serde_json::to_value(&typed_struct)` everywhere. | Same bug class as the recent `null`-vs-omit incident on `name` / `profileId`. Hand-rolled JSON drifts; typed structs with `skip_serializing_if = "Option::is_none"` don't. |
| G2 | D-F3 | Extract `RipgrepCompletionConfig::resolved() -> RipgrepCompletionView` (typed). Use from both `boot_snapshot` and `get_completion_config`. | Defaults are duplicated as inline `unwrap_or` literals at two emit sites; drift would silently land different values on the wire. |
| G3 | A-F5, A-F6 (half) | Rename `acp::instance::MetaSnapshot` → `ActorMetaCache`. Disambiguates from `mirror::MetaSnapshot`. | Two types named identically with different field sets is a footgun. Mirror is canonical; actor cache is internal. |
| G4 | D-F1 | Lift `WindowRenderer::present` / `hide` / `toggle` methods that take `&AppHandle` + `&Arc<StatusBroadcast>`. `tray.rs::toggle/present`, `daemon::window_toggle`, `rpc::overlay::toggle/present/hide` collapse to one-liners. | Three independent code paths reimplementing the same lock-present → is_visible → show/hide → set_visible sequence. Already missed `set_visible` once between transports. |
| G5 | UI-F8 | Move wire interfaces from `ui/src/constants/wire/command.ts` (`BootSnapshot`, `CompletionConfigSnapshot`, `RemotePair*`, `RemotePendingPair`) into `ui/src/interfaces/wire/{boot,completion,remote-pair}.ts`. Re-export from the wire barrel. | CLAUDE.md says interfaces live under `interfaces/wire/`; constants are for enums. |
| G6 | UI-F4 | Convert `CompletionState` `string \| null` fields (`sourceId`, `documentation`, `latestQueryId`, `latestResolveId`) → `?: string`. | CLAUDE.md "Optional fields use `?` syntax." `null` carries no semantic value here. |
| G7 | UI-F9 (fix) | `useSnapshotHydration::applySessionInfoFromMeta` re-pushes mode + model TWICE per snapshot apply. Mirror live-event ordering: `if (availableModes.length > 0) push state; else if (currentModeId) push currentMode;`. | Wasted writes; reactive subscribers fire twice per snapshot. |
| G8 | UI-F15 | `useSnapshotHydration` uses `!== undefined` for null-defence on wire fields the daemon emits with `skip_serializing_if`. Switch to `!= null` to match `useFocusPrefetch::brimSync`. | Inconsistent defence. `null !== undefined === true` slipped through to a runtime crash before; this is the residual hazard. |

## GO: dead-code deletions

Mechanical strip-outs. Each one tiny.

- D1 — Delete `TurnState::synthetic` accessor (`acp/instance.rs:74-77`, `#[allow(dead_code)]`, no callers). [A-F10]
- D2 — Delete orphaned doc comment in `adapters/permission.rs:324` ("Compile a list of glob patterns…"). [A-F13]
- D3 — Drop `RpcDispatcher::Default` impl (`rpc/mod.rs:80-84`). No callers. [R-F11]
- D4 — Drop `RpcError::CODE_*` constants; replace cross-layer use with `From<RpcError> for AdapterError`. [R-F3]
- D5 — Inline `rpc::server::dispatch_line` (one-line wrapper; rename `dispatch` to `dispatch_line` and make it `pub(crate)`). [R-F10]
- D6 — Inline `WindowRenderer::hide_on_main` (used by 3 callers; safe to call `window.hide()` directly per the comment). [D-F11]
- D7 — Drop `_ = args.instance_id;` discard in `rpc/handlers/tauri_proxy.rs::completion_query`. [R-F9]
- D8 — Drop `let _ = instance_id;` in `completion/commands.rs:39`. [D quick-win]
- D9 — Delete `getRemotePairView` + `__seedPairPreview` + the `lastResolution`/`lastErr` writes-without-reads in `ipc/remote-bridge.ts` + `composables/ui-state/use-remote-pair.ts`. [UI-F2, F7]
- D10 — Delete `startQueueDispatcher` / `stopQueueDispatcher` no-op stubs + their two `Overlay.vue` callers + the dispatcher describe block. [UI-F1]
- D11 — Delete the dead boot-fallback granular loaders (`loadHomeDir` / `loadDaemonCwd` / `loadKeymaps` / `loadCompletionConfig` / `applyTheme` / `applyWindowState`). Drop the `if (!await applyBootSnapshot)` branch in `main.ts`. CLAUDE.md no-backwards-compat rule. [UI-F3]
- D12 — Drop the no-op `watch(instanceId, () => {})` + speculative comment in `use-chat-viewport.ts`. [UI-F10]
- D13 — Drop `<style scoped>` dead block at end of `Overlay.vue` (`.chat-transcript`/`.chat-transcript-inner` no longer in template). [UI-F1]
- D14 — Inline `headerCwd`, `headerCwdFull`, `idleCwd` aliases in `Overlay.vue`. [UI-F2]
- D15 — Drop `setSessionRestored` end-to-end (writers, field, projection — no reader). [UI-F6] (could go either way; "drop, since pill was never wired" is the no-fabrication path per CLAUDE.md)
- D16 — Drop `MCPsRegistry::get` `#[allow(dead_code)]` if no caller; it's only used by tests via different paths. [D quick-win]
- D17 — Replace `remote/commands.rs::err_message` with `err.to_string()` (PairError has `Display`). [D-F7]
- D18 — `epoch_ms(_: Instant)` — drop the param, rename to `now_epoch_ms()`. [D-F4]
- D19 — Replace `[...slot.turns].reverse().find(...)` with a reverse `for` loop in `use-turns.ts::pushUsageUpdate`. [UI-F16]
- D20 — Move `shutdownInstance` from `views/palette/instances.ts` into `composables/instance/`. [UI-F9]
- D21 — Move `RpcError::CODE_*` constants behind `From<RpcError> for AdapterError`. [R-F3 == D4]
- D22 — Delete `apply_thinking_budget` from generic `profile.rs`; move into vendor module if/when a hook surface lands. [A-F15] *(actually defer — needs hook trait; downgrading)*
- D23 — Drop `Adapter::permissions` Option-wrapping (`Option<Arc<dyn …>>`); just `Arc<dyn …>`. Only ACP exists. [A-F9]

## GO: comment rot strip

Mechanical sweep. Each comment that mentions:

- "K-XXX" / "K-NNN" Linear references documenting LANDED work
- "Phase A2/A3/A4/A5/B/C1/C2/C3" historical phase labels
- "round 3" / "round X" iteration labels
- "MR XX review" / "in PR #N"
- "wireframe spec" / "per wireframe"
- "captain's note:" prose
- TODO / FIXME / TODO(K-XXX) — scrutinise each; delete unless actionable in this PR

Files touched: `mirror.rs`, `instance.rs`, `commands.rs`, `permission.rs`,
`completion/cancellations.rs`, `completion/source/commands.rs`, `skills/mod.rs`,
`daemon/wm.rs`, `protocol.rs`, `Composer.vue`, `QueueStrip.vue`, `Frame.vue`,
`Turn.vue`, `StreamCard.vue`, `Overlay.vue`, `palette/profiles.ts`,
`palette/models.ts`, `palette/modes.ts`, `palette/root.ts`, `palette/instances.ts`.

## TEST: regression coverage to add

Five highest-leverage, none of which require new architecture:

- `T1 — adapters/registry::resolve_token` — name + UUID-shaped slug collision paths. [A tests-to-add]
- `T2 — adapters/mirror::ConfigOptionsUpdate round-trip` — apply + meta_snapshot + chat_snapshot all see the update. [A tests-to-add]
- `T3 — composables/instance/use-turns.test.ts` — colocated. Covers `pushTurnStarted` idempotency, `pushUsageUpdate` synth-placeholder path, `markThinkingStart` no-op when no open turn. [UI-F12]
- `T4 — components/Modal.test.ts` + `components/Toast.test.ts` — primitives that drive every dialog the captain sees. Slot dispatch, tone variants, body discriminator. [UI-F14]
- `T5 — lib/tools/merge-tool-call.test.ts` — pin "content replaces wholesale, never concats" against both live-patch and snapshot-replay paths. Catches B1 regressions. [B1]
- `T6 — daemon/wm::detect` — `detect_picks_hyprland_when_env_set`, `detect_picks_sway_when_only_swaysock`, `detect_falls_back_to_gtk`. [D tests-to-add]
- `T7 — tools/git::snapshot` — outside-repo / fresh-repo / detached-head / no-upstream. [D tests-to-add]
- `T8 — rpc/server::tauri_proxy_dispatch_matches_tauri_command_shape` — for the duplicated arms (instances_list, sessions_info, session_submit, …) assert proxy + command produce identical JSON for the same args. Pre-emptive guard until R-F1 lands. [R tests-to-add]

## DEFER: real refactors that need their own plans

None of these belong in a "cleanup PR." Each is its own scope.

| ID | Source | What | Why deferred |
|----|--------|------|--------------|
| X1 | A-F1 | Split `acp/instance.rs::run` (1500-line actor body) into `bootstrap_fresh`/`bootstrap_resume` + per-command handlers. | Each spawned future captures ~10 cloned vars; needs a context struct. Plan first. |
| X2 | A-F3 | Decide `Adapter` trait fate — delete (until 2nd adapter) OR commit (migrate every call site to `Arc<dyn Adapter>`). | Product question (HTTP adapter timeline). Don't decide in cleanup. |
| X3 | A-F2 | Extract `project_advertised_modes` / `project_advertised_models` to dedupe Fresh + Resume bootstrap arms in `acp/instance.rs`. | Dependent on X1's split. |
| X4 | R-F1 | Collapse `tauri_proxy.rs` ↔ `adapters/commands.rs` ↔ `completion/commands.rs` ↔ `skills/commands.rs` body duplication via `_impl` fns called from both shims. | Substantial; touches every Tauri command + every JSON-RPC handler. Worth doing; not as a drive-by. |
| X5 | R-F2 | Add `HandlerOutcome::ShutdownAfterReply(Value)` variant; replace string-marker `{killed,exiting}` test in `server.rs`. | Touches every `RpcHandler` impl + the dispatch loop; coordinate. |
| X6 | R-F13 | `PermissionController::resolve_if_pending` `Option<bool>` → `enum ResolveResult`. | Trait change ripples to Tauri command + RPC handler. |
| X7 | UI-F8 | Extract `mergeWireToolCall` into `lib/tools/`. | **Wait — this is B1, IS in this PR.** Reclassified above as GO. |
| X8 | UI-F18 | Split `use-chat-viewport.ts` (696 lines) into pagination + merge + eviction. | Hot path; needs characterisation tests before split. |
| X9 | UI-F17 | Split large SFCs (`Overlay.vue` 1000, `Composer.vue` 827, `Viewport.vue` 735, `CommandPalette.vue` 653). | Each is its own plan; not drive-by. |
| X10 | UI-F5 | Extract `usePairQr` + `usePairQrScanner` shared between `RemotePairScreen` + `RemotePairModal`. | Genuine win but requires QrScanner lifecycle review. |
| X11 | UI-F4 | Extract `useDebouncedMarkdown` shared between `MarkdownBody` + `StreamCard`. | Same shape — extract carefully so plain-pass / shiki-pass / staleness-guard semantics stay intact. |
| X12 | A-F15 | Move `apply_thinking_budget` from generic `profile.rs` to vendor module. | Requires per-vendor `pre_resolve_hook` trait method. Cross-cutting. |
| X13 | UI-F8 (subagent's note) | Extract `useDebouncedMarkdown`, `useAutoExpand`, `usePairQr`, `usePairQrScanner` composables. | Each is a single-day refactor. Bundle as "DRY pass" PR after this cleanup ships. |

## DEFER: explicit no-touch

- The `biased` `select!` in `rpc/server::handle_connection` and `remote/ws::handle_socket`. Reordering breaks the documented invariant.
- The chat virtualization story (`Viewport.vue`'s no-virt + content-visibility + v-memo). Don't touch.
- `MetaSnapshot` consolidation (A-F5 second half — collapse actor route into mirror route). The actor route blocks on actor command channel; consolidating risks pre-handshake-default reads.
- `AdapterRegistry` topology + `setup_app` boot wiring.
- `WindowRenderer::apply_anchor` body (init-once + idempotent re-config; documented carefully).
- `remote/cert::resolve_or_generate` SAN sidecar regenerate-on-drift logic. TOFU implications.
- Layer-shell init order. Manual verification gated behind Hyprland session.

---

## Implementation order (one commit per logical chunk)

1. **`docs(claude-md)`** — already in working tree; commit alone.
2. **`fix(ui)`** — extract `mergeWireToolCall` helper + test (B1 / G1 from UI). Single-file new helper + unit test pinning the contract.
3. **`refactor(adapters)`** — typed wire shapes for `AgentSummary` / `ProfileSummary` / `InstanceListEntry` consumers. Replace `json!()` macros at every call site with `to_value(&typed)`. (G1 daemon)
4. **`refactor(daemon)`** — typed `BootSnapshot.{agents,profiles,instances,completionConfig}` (G2). Shared `RipgrepCompletionConfig::resolved()` helper used by both `boot_snapshot` and `get_completion_config`.
5. **`refactor(daemon)`** — `WindowRenderer::{present,hide,toggle}` methods; collapse `tray::toggle/present` + `daemon::window_toggle` + `rpc::overlay::*` callers (G4).
6. **`refactor(adapters)`** — rename `acp::instance::MetaSnapshot` → `ActorMetaCache` (G3).
7. **`refactor(ui)`** — move wire interfaces from `constants/wire/` to `interfaces/wire/` (G5).
8. **`refactor(ui)`** — `CompletionState` null → optional (G6); `useSnapshotHydration` mode/model push dedup (G7); `!= null` defences sweep (G8).
9. **`chore(deletes)`** — strip dead code (D1-D23). One commit if the line-count is small.
10. **`chore(comments)`** — strip K-XXX / Phase-X / wireframe / captain's note rot. One commit; mechanical.
11. **`test(rust)`** — T1 + T2 + T6 + T7 + T8.
12. **`test(ui)`** — T3 + T4 + T5.
13. **`docs(plans)`** — commit this plan doc itself + a follow-up issues file enumerating X1-X13.

Each commit verified through `task lint` (or per-language equivalents) +
the relevant test suite. Failures roll the commit back; never compound.

## Branch + PR

- Branch: `chore/repo-cleanup` off `main`.
- Each commit listed above lands in order.
- PR opens against `main` with description summarising:
  - line-count delta (CLAUDE.md −1212; total likely net negative)
  - real bug fixed (B1)
  - structural fixes (G1-G8)
  - dead code removed
  - tests added
  - explicit list of deferred items + this plan doc as the persisted record

---

## Pickup checklist (for the implementer — i.e. me, next pass)

1. Read this plan top to bottom.
2. Land commits 1-12 in the order above.
3. Run `task lint` + `task test` after each commit; revert on failure.
4. After commit 13, push branch, open PR, paste the cleanup summary.
5. File one issue per X1-X13 deferred item with the source-finding ID + suggested fix from the original review reports.

# Chat virtualization — finalised plan

**Status:** decisions resolved. Ready to execute. Each phase lands as one
commit; rollback is `git revert` per phase.

**Background:** branch `chore/perf-leak-audit` already shipped
viewport-relative page sizing, measured-page-size off real DOM extent,
`MAX_PAGES_KEPT = 3` cache cap, and a floating chevron. The chat
surface today runs a plain `v-for` over `blocks` with no
virtualization. Captain's report: "i do not think this works at all".
This plan resolves whether virtualization is genuinely needed and, if
so, how to ship it without re-introducing the measure-loop failures
that bit the prior two attempts.

---

## Decisions (resolved during plan-hard interview)

Every open branch from the original plan has a concrete answer. No
"if X then Y else Z" runtime branches.

### D1. Profile baseline first, do NOT leap to D

**Decision: profile current status quo (E) before shipping any
mitigation.** The captain's complaint may be about wire bytes /
hydration latency / DOM count — three different problems with three
different fixes. Shipping D blind risks cementing 33ms streaming lag
without evidence it solves the real pain.

**Deterministic profiling harness** (`tests/perf/streaming-harness.ts`,
new): a Playwright MCP browser-mode script that:

1. Boots Vite dev preview (`pnpm --filter hyprpilot-ui dev`).
2. Seeds `__hyprpilot_dev` with a synthetic 200-turn fixture (mix of
   short prompts + long agent replies + tool-call cards).
3. Drives `pushTranscriptEvent` at 30 Hz for 30 seconds, simulating a
   live streaming turn growing one chunk per frame.
4. Captures via `page.metrics()` + `performance.now()`:
   - P95 frame time during streaming.
   - Total DOM node count (`document.querySelectorAll('*').length`).
   - Scroll-jank index during a backward-fetch trigger
     (frames > 50 ms within a 2-second window).
   - Scroll-position retention across simulated prepend
     (`scrollTop` delta after fetch settles, ±2 px tolerance).

This is a **synthetic harness**, not a CI gate today — captain runs
it manually before/after each phase and pastes numbers into the PR
description. Promotes to CI later if the numbers warrant it.

### D2. Verify Strategy B (`content-visibility: auto`) before A

**Decision: yes — run a 10-minute support probe BEFORE picking A.**
WebKit2GTK 4.1 (the daemon's pinned webview) tracks WebKit roughly 6
months behind Safari Tech Preview; `content-visibility` shipped in
Safari 17.4 (March 2024). The pinned webkit2gtk on Tauri 2.10 is from
late 2024 and likely supports it.

The probe lives as one Playwright MCP step:

```js
browser_evaluate(() => CSS.supports('content-visibility', 'auto'))
```

against the dev preview running inside a `task run` daemon. **If
true**, Strategy B becomes the recommended path (lower risk, fewer
moving parts) and Strategy A is deleted from this plan. **If false**,
proceed to A as currently planned. The probe runs in Phase 0.

### D3. `v-memo` slots in as Phase 0.5 (cheap intermediate)

**Decision: ship `v-memo` BEFORE virtualization** — closes most of the
gap for free. Apply to history rows (every block where
`groupKey !== openTurnId`):

```vue
<Turn
  v-for="(block, blockIdx) in blocks"
  :key="block.groupKey"
  v-memo="[block.groupKey, block.role, block.turnEntries.length, block.toolCalls.length, blockIdx === liveBlockIdx]"
  …
>
```

Rationale: history rows never change shape after their turn ends.
Vue's render-cache short-circuits the entire subtree when the memo
deps haven't changed, so streaming chunks into the live row don't
re-walk the prior 150 rows' VNodes. Cheap; risk-free; might delete
the need for Strategy A entirely.

### D4. Throttle in Strategy D — measure first, named constant when shipped

**Decision: do NOT ship D blind.** `flushPatches` already batches
through `queueMicrotask`, which collapses each event-loop tick's
events into one `setQueryData` call. A `setTimeout(33ms)` upgrade
trades observable streaming smoothness for *theoretical* render
savings; the harness from D1 must show the improvement before we
commit.

**If profiling proves throttling helps**, the constant is named:

```ts
/// Maximum streaming-patch flush rate (Hz). 30 Hz matches typical
/// agent-chunk arrival; tighter rates trade smoothness for
/// render-pressure relief. 30 Hz = 33.33 ms; we round to 34 to
/// avoid sub-frame drift.
const STREAMING_FLUSH_INTERVAL_MS = 34
```

`flushPatches` becomes a trailing-edge throttle (microtask coalesce
within the 34 ms window, `setTimeout` schedules the actual flush).
The microtask layer stays — it batches within one tick; the timeout
caps the flush rate across ticks.

### D5. Live-row carve-out keys on `openTurnId`, not head-of-array

**Confirmed.** `useSnapshotHydration` can replay older `TurnStarted`
records, briefly placing the open turn mid-list. Rule:

```
A block is "live" iff block.turnId === openTurnId.value
```

Implemented as a named composable, `useLiveTurnPin(blocks, openTurnId)
→ { liveBlock, historyBlocks }`. Returned shape:

```ts
export interface UseLiveTurnPinApi {
  /// The block whose turnId matches the currently-open turn, or
  /// undefined when no turn is in flight (history-only view).
  liveBlock: ComputedRef<TimelineBlock | undefined>
  /// Every block except the live one, in oldest-first order.
  historyBlocks: ComputedRef<TimelineBlock[]>
}

export function useLiveTurnPin(
  blocks: ComputedRef<TimelineBlock[]>,
  openTurnId: ComputedRef<string | undefined>
): UseLiveTurnPinApi
```

History blocks feed the virtualizer; the live block renders as a
sibling pinned to the tail. Identity is the turn id, not the array
index — survives prepends and replay without flipping carve-out
membership.

### D6. Compensation timing — double-RAF, not double-`nextTick`

**Confirmed.** `nextTick` resolves after Vue's render flush, but
`ResizeObserver` callbacks fire in their own paint-frame phase.
Reading `scrollHeight` after two `nextTick`s catches Vue's render
but races the observer that drives the virtualizer's measurement
commit. Use double-RAF instead:

```ts
async function afterPaintSettled(): Promise<void> {
  await new Promise<void>((resolve) => {
    requestAnimationFrame(() => {
      requestAnimationFrame(() => resolve())
    })
  })
}
```

Two frames: frame 1 lets layout settle; frame 2 lets the
ResizeObserver's commit land. Vue's `nextTick` is wrong here because
ResizeObserver isn't part of Vue's reactive cycle.

### D7. Re-entrancy guard for `onScroll` during compensation

Writing `el.scrollTop = newValue` synchronously dispatches a
`scroll` event in WebKit. Without a guard, the compensation handler
re-enters `onScroll`, which can re-fire `fetchNextPage` against a
half-settled state.

```ts
let scrollCompensating = false

function onScroll(): void {
  if (scrollCompensating) {
    return
  }
  // … existing body
}

async function compensateAfterPrepend(deltaPx: number): Promise<void> {
  const el = scrollEl.value
  if (!el || deltaPx === 0) {
    return
  }
  scrollCompensating = true
  el.scrollTop += deltaPx
  // Release on the next frame so the synchronous scroll event
  // dispatched by the assignment above is the only one that sees
  // the guard set.
  requestAnimationFrame(() => {
    scrollCompensating = false
  })
}
```

One-frame suppression, not a full debounce — too long a window risks
swallowing real scroll input from a captain who scrolls during fetch.

### D8. Adaptive overscan for tall turns

Single turn cards can exceed 5 viewports (long agent replies with
multiple diff cards). Fixed `overscan: 2` then renders 10+ viewports
of DOM, defeating the cap.

**Decision: per-direction adaptive overscan.**

```ts
const TALL_BLOCK_VIEWPORT_RATIO = 2

function computeOverscan(directionItems: TimelineBlock[], viewportPx: number): number {
  const tall = directionItems.some(
    (b) => measureCacheGet(b.groupKey) ?? 0 > viewportPx * TALL_BLOCK_VIEWPORT_RATIO
  )
  return tall ? 0 : 2
}
```

When the next item in either direction exceeds 2× viewport, drop
overscan to 0 in that direction. TanStack Virtual doesn't expose
per-direction overscan natively; we approximate by feeding a
direction-aware `overscan` getter that the virtualizer re-reads on
each layout pass. `overscan: 0` means we render exactly the visible
range, which is the right call when one row already eats the
viewport.

### D9. Strategy ranking — final order

1. **Phase 0** — verify webkit2gtk supports `content-visibility: auto`.
2. **Phase 0.5** — ship `v-memo` on history rows. Re-run harness.
3. **Phase 1** — measure status quo. Decide: are the harness numbers
   already inside budget?
4. **Phase 2** — ship Strategy B if D2 probe passed AND Phase 1
   showed insufficient; else Strategy A. The Phase 0 probe makes
   this a binary, not a ladder.

Strategy D (throttling) becomes a *fallback knob* the captain can
opt into via dev flag, not a default phase. We don't ship it
prophylactically.

Strategy C (hand-rolled IntersectionObserver) is **deleted from the
plan** — TanStack Virtual covers its design space and we don't have
NIH-budget for it.

### D10. Performance targets (named, measurable)

These are the bars the harness from D1 must clear after each phase:

| Metric | Target | Why |
| --- | --- | --- |
| P95 frame time during 30 Hz streaming | ≤ 16.6 ms (60 Hz) on desktop, ≤ 33 ms (30 Hz) on mobile remote | one missed frame per 5 seconds is acceptable; more = visible jank |
| DOM node count at peak (200-turn synthetic) | ≤ 6000 nodes | rough-budget on mid-range mobile; below the WebKit per-page tipping point |
| Scroll-jank index (frames > 50 ms during backward fetch) | 0 | one >50 ms frame = visible stutter |
| Scroll-position retention across prepend | ±2 px | sub-pixel rounding tolerance |

**Bar must be cleared after every phase**, not at the end. A phase
that regresses any metric reverts.

---

## Goals (re-stated, unchanged)

1. **Cap rendered DOM** to roughly "what the captain can see + a small
   buffer". Page-trim caps cache to 3 pages of items today; that is
   60-200+ DOM nodes depending on density. Target is bounded by
   viewport, not cache size.
2. **Don't break streaming** at ~30 Hz chunk arrival. No measure
   loops, no jank.
3. **Don't break backward pagination.** Prepend must keep visible
   content anchored within ±2 px.
4. **Don't break stick-to-bottom.** Live area follows the tail when
   stuck; leaves the captain alone when reading history.

---

## Prior attempts — root cause that informs this plan

| SHA | Move | Outcome |
| --- | --- | --- |
| `e2e2404` | fixed-height virtualizer (`estimateSize: 80`) | giant gaps; reverted |
| `5147fb3` | drop virtualization, plain v-for | works but unbounded DOM |
| `92f025f` | variable-height + `shouldAdjustScrollPositionOnItemSizeChange` | "Maximum recursive updates exceeded" + ResizeObserver loop under streaming |
| `33218e4` | drop the predicate, stable `measureRow` callback | loops persisted — fundamental measure-on-resize incompatibility with continuously growing rows |
| `6790fd3` | drop virtualization again | status quo today |

The fundamental loop, which Strategy A's live-turn carve-out exists
to break:

```
agent_message_chunk → setQueryData → blocks recompute → row resizes →
ResizeObserver fires → virtualizer recomputes → re-render → row
resizes (still streaming) → ResizeObserver fires → … → Vue caps at
100 iterations and throws.
```

The carve-out solves it by removing the live row from the
virtualizer's tracked range — `ResizeObserver` on a non-virtualized
row doesn't feed back into the virtualizer's `state` ref.

---

## Strategy B (preferred if Phase 0 probe passes)

`content-visibility: auto` + `contain-intrinsic-size: auto N px` per
turn block. The browser skips layout + paint for off-screen blocks
while keeping the DOM allocated.

**Pros** (vs Strategy A):
- Zero JS measurement code.
- No ResizeObserver loop possible.
- Plays naturally with streaming — only the visible row pays.
- Far smaller diff (one CSS rule + one `contain-intrinsic-size`
  hint per block).

**Cons:**
- DOM is still allocated. Memory ceiling = same as today (page-trim
  bound). DOM-node-count target from D10 still requires page-trim
  to do its job.
- `contain-intrinsic-size` is a placeholder height; wrong value
  → janky scrollbar. Use the `auto` keyword so the browser
  remembers the last-rendered size:
  ```css
  .chat-block {
    content-visibility: auto;
    contain-intrinsic-size: auto 240px;
  }
  ```

**Implementation lives in `Turn.vue`'s scoped style** — one rule.
That's the entire diff for Strategy B beyond Phase 0.5's `v-memo`.

---

## Strategy A (fallback if Phase 0 probe fails)

Turn-level group virtualization, with the live-turn carve-out as a
permanent feature, not a safety net.

```
Today (item-level conceptually): ~30 transcript items per ~3 turns
After (turn-level): ~3-10 virtual rows per ~3-10 turns
```

### Architecture

- **Virtualizer scope:** `historyBlocks` from `useLiveTurnPin`. The
  live block renders as a sibling.
- **Stable keys:** `block.groupKey` (verified turn-id-stable in
  `timelineBlocksFromSnapshot`; tool-call merges don't change the
  key — they merge on `toolCallId`, not `groupKey`).
- **`shouldAdjustScrollPositionOnItemSizeChange`:** OMITTED. It's
  what triggered the `92f025f` loop. We compensate prepends
  out-of-band (D6 + D7).
- **Stable measure callback:** captured once in setup; `33218e4`'s
  fix preserved.
- **`overscan`:** adaptive per D8.
- **`estimateSize`:** 240 px (median turn block measured from
  screenshots; refined when `measureElement` lands).

```ts
const virtualizer = useVirtualizer(computed(() => ({
  count: historyBlocks.value.length,
  getScrollElement: () => scrollEl.value ?? null,
  estimateSize: () => 240,
  overscan: computeOverscan(historyBlocks.value, scrollEl.value?.clientHeight ?? 800),
  getItemKey: (i) => historyBlocks.value[i]?.groupKey ?? i
})))

function measureRow(el: Element | null): void {
  if (el) {
    virtualizer.value.measureElement(el)
  }
}
```

### Out-of-band prepend compensation

```ts
let scrollHeightBeforeFetch = 0

async function fetchNextPage(): Promise<unknown> {
  const el = scrollEl.value
  scrollHeightBeforeFetch = el?.scrollHeight ?? 0
  const r = await viewport.fetchNextPage()
  await afterPaintSettled()                       // double-RAF (D6)
  if (el && scrollHeightBeforeFetch > 0) {
    const delta = el.scrollHeight - scrollHeightBeforeFetch
    if (delta !== 0) {
      await compensateAfterPrepend(delta)         // re-entrant-guarded (D7)
    }
  }
  return r
}
```

### Stick-to-bottom integration

`useStickToBottom`'s `stuck` flag still gates auto-scroll. The new
move is that "scroll to bottom" targets the live block (rendered as
a sibling), not a virtualizer index:

```ts
function scrollLiveBlockIntoView(): void {
  liveBlockEl.value?.scrollIntoView({ block: 'end', behavior: 'auto' })
}
```

`scrollToIndex(last, 'end')` is wrong here — the last virtualized
index is the most-recent *history* block, not the live tail. The
live block lives outside the virtualizer's measured range.

### Concurrent prepend + streaming

Streaming patches into the live block do NOT touch
`historyBlocks` (the live block is carved out). Concurrent prepend +
streaming therefore touch disjoint reactive surfaces — no delta
corruption.

---

## Implementation phases

### Phase 0 — `content-visibility` probe (10 min, blocking)

1. `task run` boots the daemon.
2. Playwright MCP `browser_evaluate(() => CSS.supports('content-visibility', 'auto'))`
   against the dev preview.
3. Record the boolean in the PR description.

**If true** → execute Phase 0.5 → Phase 1 → Phase 2B.
**If false** → execute Phase 0.5 → Phase 1 → Phase 2A.

### Phase 0.5 — `v-memo` on history rows (one commit)

1. Identify the `<Turn>` v-for loop in `Viewport.vue`.
2. Add `v-memo` keyed on the deps from D3.
3. Run the synthetic harness; capture P95 frame time + DOM node
   count.
4. Paste numbers in PR description.

**If P95 + DOM clear D10's targets after this phase**, the plan
ends here. Strategy A and B both become unnecessary; close the PR
as "v-memo was sufficient".

### Phase 1 — measurement (no code change)

1. Run synthetic harness against current state-after-0.5.
2. Decide: are P95 + DOM + jank + retention already inside D10
   targets?
   - **Yes** → end of plan. Document harness numbers in
     `docs/plans/2026-05-09-chat-virtualization-revisit.md`'s
     outcome section + close PR.
   - **No** → proceed to Phase 2 (A or B based on D2 probe).

### Phase 2A — Strategy A (Phase 0 probe failed)

One commit per sub-step so each can revert independently.

1. **2A.1 — `useLiveTurnPin` composable.** New file
   `ui/src/composables/instance/use-live-turn-pin.ts`. Pure
   computed; no side effects. Test colocated.
2. **2A.2 — Viewport wires `useLiveTurnPin`.** History v-for now
   reads `historyBlocks`; live block renders as a sibling. **No
   virtualizer yet.** This is the carve-out shape on its own; verify
   no regression against today's behaviour first.
3. **2A.3 — Introduce virtualizer over `historyBlocks`.**
   `@tanstack/vue-virtual` (already in `package.json`). Stable
   `measureRow` callback. `estimateSize: 240`. `overscan: 2`
   (adaptive comes in 2A.5). No prepend compensation yet — backward
   fetch will jump; that's expected and fixed in 2A.4.
4. **2A.4 — Out-of-band prepend compensation.**
   `afterPaintSettled` + `compensateAfterPrepend` helpers in
   `useChatViewport`. `fetchNextPage` wrapper from D6.
5. **2A.5 — Adaptive overscan.** `computeOverscan` reads measured
   sizes from the virtualizer's measurement cache.
6. **2A.6 — Stick-to-bottom wiring.** `scrollLiveBlockIntoView`
   replaces the prior `scrollToBottom` call inside the
   `useStickToBottom` mutation handler.
7. **2A.7 — Harness re-run.** All four metrics from D10 must clear.

### Phase 2B — Strategy B (Phase 0 probe passed)

Two commits.

1. **2B.1 — Apply `content-visibility: auto` to `.chat-block`** in
   `Turn.vue`'s scoped style. `contain-intrinsic-size: auto 240px`.
2. **2B.2 — Harness re-run.** All four metrics from D10 must clear.

---

## Test plan (synthetic harness shape)

`tests/perf/streaming-harness.ts` (new, browser-mode Playwright MCP
script — not a CI gate today):

```ts
// 1. Boot Vite dev server; navigate to http://localhost:1420
// 2. Seed 200-turn fixture via __hyprpilot_dev.pushSnapshot(fixture)
// 3. Start streaming: 30 Hz pushTranscriptEvent into open turn for 30s
// 4. Capture P95 frame time across the 30s window
// 5. capture DOM node count at peak (T+15s)
// 6. Trigger backward fetch (scroll to top); capture jank index over 2s
// 7. Capture scrollTop before fetch resolves; compare against scrollTop
//    one second after fetch resolves; emit delta
// 8. Print one JSON line: { p95Ms, domCount, jankIdx, retentionPx }
```

Numbers paste into the PR description per phase. Once the harness
proves itself stable, we promote it to a Vitest perf bench (gated
behind `pnpm test:perf`) so CI catches regressions automatically.

Existing tests in `Viewport.test.ts` (backward-fetch trigger + load
chip) must continue to pass after every phase.

New unit tests:

- `useLiveTurnPin.test.ts` (Phase 2A.1) — given mock blocks +
  `openTurnId`, returns the carve-out correctly across (a) no open
  turn, (b) open turn at tail, (c) open turn mid-list (the
  hydration-replay case D5 calls out).
- `Viewport.test.ts` extension (Phase 2A.4) — backward fetch
  retains `scrollTop` within ±2 px (mock the prepend, assert the
  delta).
- No new test for Strategy B beyond the harness — it's a CSS
  change.

---

## Risks + rollback

| Risk | Mitigation |
| --- | --- |
| Phase 0.5 (`v-memo`) breaks reactivity for live tool-call merges | live block is excluded from the v-memo'd loop in Phase 2A.2; for the interim Phase 0.5, the `liveBlockIdx` dep in the memo array forces re-render when the live block changes identity |
| Phase 2A.3 re-introduces measure loop | live-turn carve-out (Phase 2A.2 ships first as a no-virtualizer test) means streaming events never enter the virtualizer's reactive surface; the loop's prerequisite is gone |
| Phase 2B browser support absent on captain's mobile remote | the Phase 0 probe runs in the daemon's webview, not desktop Chrome — the captain's mobile is a remote browser, not the daemon. Run the probe again in mobile-remote mode before declaring 2B viable for that surface |
| Prepend jump despite compensation | D7's re-entrancy guard + D6's double-RAF; if numbers still drift, pin via `scrollIntoView(historyBlocks[0], { block: 'start' })` after compensation as a belt-and-braces |
| Adaptive overscan misses on slow-arriving measurements | `computeOverscan` falls back to `2` when `measureCacheGet` returns undefined; the worst case is "a tall turn renders 2 viewports of overscan once before the cache populates" — bounded |

**Rollback:** every phase = one commit. Each phase that fails its
harness target gets reverted on the spot. The status-quo render
path (plain v-for + page-trim) is the floor.

---

## Style conformance

- Named constants: `STREAMING_FLUSH_INTERVAL_MS`,
  `LOAD_MORE_THRESHOLD_PX` (existing), `TALL_BLOCK_VIEWPORT_RATIO`,
  `PREPEND_RETENTION_TOLERANCE_PX`.
- Composable shape: `useLiveTurnPin(): UseLiveTurnPinApi` — no
  drive-by exports; tests under `tests/composables/instance/`.
- All single-statement control flow braced.
- No `__` in CSS class names. No `--pilot-*`. No new keyframes.
- Comments: terse WHY only. The "why double-RAF instead of
  nextTick" comment from D6 is a fair example; restating that the
  function "compensates after prepend" is not.

---

## What this plan does NOT include

- Daemon-side delta events. The reviewer flagged these as cleaner
  than UI-side patching; agreed in principle, out of scope here.
  File a separate plan for K-XXX if the harness numbers stay red
  after Phase 2.
- `shallowRef` on query.data. Marginal; revisit only if the
  harness shows the deep-reactivity walk dominates.
- Mobile-remote-specific tuning. The harness targets desktop today;
  if mobile numbers diverge sharply we'll fork a per-surface phase.

---

## Pickup checklist

1. Run Phase 0 probe (10 min). Record the boolean in PR description.
2. Ship Phase 0.5 (`v-memo`) as one commit. Re-run harness; paste
   numbers in PR description.
3. Run Phase 1 (measurement). Decide: stop or continue.
4. If continue: execute Phase 2A or 2B per Phase 0 result, one
   commit per sub-step. Re-run harness after each.
5. Update this doc's "outcome" section (TBD — append after Phase 2
   completes) with final numbers and any deviations from the plan.

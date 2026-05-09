# Chat virtualization — revisit + plan

**Status:** plan-handoff for tomorrow. Don't start implementation until we
agree on Strategy.

**Background context:** branch `chore/perf-leak-audit` already shipped
viewport-relative page sizing, measured page-size from real DOM extent,
`MAX_PAGES_KEPT = 3` cache cap, and a floating chevron. The chat surface
runs on a plain `v-for` over `blocks` today (no virtualization). Captain's
report: "i do not think this works at all" — measured page sizing alone
isn't enough; we should look at virtualization again.

This doc covers (1) why prior virtualization attempts failed, (2) what
TanStack Virtual's contract actually is, (3) candidate strategies with
trade-offs, (4) recommended path, (5) an explicit fallback if the
recommended path also fails.

---

## Goals (re-stated)

1. **Cap rendered DOM** to roughly "what the captain can see + a small
   buffer" — currently page-trim caps to 3 pages of items, which can be
   60-200+ DOM nodes depending on item density. We want the rendered
   set bounded by viewport, not by cache size.
2. **Don't break streaming.** Agent replies stream chunks at ~30Hz.
   Virtualization must not collapse, loop, or jank under streaming.
3. **Don't break backward pagination.** When a captain scrolls up to
   read history, prepending older items must not yank the visible
   content out of view.
4. **Don't break stick-to-bottom.** New chunks at the tail keep the
   transcript at the bottom while the captain is "stuck"; if they've
   scrolled away, leave them alone.

These are the hard constraints. Anything else (memory ceiling, CPU,
mobile-specific concerns) is secondary.

---

## Prior attempts — what failed and why

Five commits already exist on this branch / its predecessors that touched
chat virtualization. Read each before designing the next attempt:

| SHA | Date | Move | Outcome |
| --- | --- | --- | --- |
| `e2e2404` | May 8 10:58 | First attempt: fixed-height virtualizer with `estimateSize: 80` | **Giant gaps** between rows of varying actual height. Reverted. |
| `5147fb3` | May 8 11:40 | Drop virtualization, plain v-for | **Worked** but unbounded DOM under long sessions. |
| `92f025f` | May 8 12:32 | Re-introduce with proper variable-height setup: `getItemKey` (stable across prepends), `data-index` + `:ref="(el) => virtualizer.measureElement(el)"`, `shouldAdjustScrollPositionOnItemSizeChange`, `ESTIMATE_SIZE_PX = 200`, `overscan: 8`. Stick-to-bottom via `watch(blocks.length) + nextTick + scrollToIndex(last, 'end')` | **Maximum recursive updates exceeded** + **ResizeObserver loop completed with undelivered notifications** under streaming reply / session-load replay. |
| `33218e4` | May 8 23:58 | Identify two loops, fix both: drop `shouldAdjustScrollPositionOnItemSizeChange` (the offset write triggered re-render → re-fire ResizeObserver → loop); replace inline `:ref="(el) => measureElement(el)"` with stable `measureRow` callback (Vue treats inline arrows as identity-changing each render → unregister/register ResizeObserver every render) | **Loops persisted.** Even with stable callbacks the streaming chunk → row resize → ResizeObserver fires → re-render → measure → loop pattern still hit Vue's 100-iteration cap. |
| `6790fd3` | May 9 00:05 | Drop virtualization again. Page-trim alone keeps DOM bounded ~150 rows; "correctness over memory" | Where we are today (plain v-for + `MAX_PAGES_KEPT = 3`). |

### Root-cause analysis of the `92f025f` → `33218e4` failures

The fundamental loop, even after fixing the obvious culprits:

```
agent_message_chunk arrives via acp:transcript
  ↓
patchLatestPage → setQueryData
  ↓
items recomputes → blocks recomputes → Vue re-renders
  ↓
the live (head) row's <Body> content grows (new chunk text)
  ↓
ResizeObserver on that row fires (height changed)
  ↓
virtualizer recomputes virtualItems → triggerRef(state)
  ↓
Vue re-renders the v-for over virtualRows
  ↓
the same row re-renders with the new content (still growing)
  ↓
ResizeObserver fires AGAIN
  ↓
[Vue throws "Maximum recursive updates exceeded"]
```

The **measure-on-resize** model is intrinsically incompatible with
**continuously growing rows**. Streaming text into one row produces
infinite ResizeObserver ticks; each tick recomputes virtualItems;
each recomputation triggers re-render; re-render gives the
ResizeObserver another size to react to. The chain only terminates
when the chunk stops arriving.

Note this isn't TanStack-specific — the loop is fundamental to any
"measure DOM → drive layout decisions → render → measure DOM" cycle
when one of the rendered rows keeps changing size.

---

## TanStack Virtual contract — what it is and isn't

[`@tanstack/vue-virtual`](https://tanstack.com/virtual/latest/docs/framework/vue/vue-virtual) wraps the same headless core
([`@tanstack/virtual-core`](https://tanstack.com/virtual/latest/docs/api/virtualizer))
React + Solid + Svelte use. The relevant pieces:

- **Fixed-height** (`estimateSize: () => N`): no measurement needed. Works
  perfectly when every row is the same height. Useless for chat.
- **Variable-height via `measureElement`**: each rendered row registers
  with a `ResizeObserver` (managed by virtual-core) via a `data-index`
  + `ref` callback. The observer fires on size changes, and virtual-core
  stores measurements keyed by `getItemKey(i)`.
- **`shouldAdjustScrollPositionOnItemSizeChange`** (predicate): when an
  off-screen-above row measures in (taller than estimate), virtual-core
  writes the scroll offset to compensate so visible content stays
  anchored. **This is the predicate that triggered our loop in
  `92f025f`** — writing scroll offset re-runs the virtualizer logic.
- **`overscan`**: render N additional rows above/below the visible
  range. Bigger value = smoother scroll, more DOM.

**What TanStack Virtual is NOT designed for:**

- Rows that **continuously grow** (live streaming text). Their examples
  ([`dynamic-rows`](https://tanstack.com/virtual/latest/docs/framework/react/examples/dynamic), [`variable`](https://tanstack.com/virtual/latest/docs/framework/react/examples/variable), [`lanes`](https://tanstack.com/virtual/latest/docs/framework/react/examples/dynamic-lanes))
  show variable-but-stable content (markdown blocks of fixed text
  rendered once). None of the official examples cover the
  streaming-chunk-into-the-live-row case we have.
- DOM additions while streaming. Even Discord / Slack-style chat virt
  libraries (e.g. [`react-virtuoso`](https://virtuoso.dev)) explicitly
  document trade-offs around streaming and recommend **debouncing
  streaming updates** to ~30Hz at most.

The streaming-into-the-live-row case is genuinely hard. We're not the
first to hit it; the answer in production chat apps is either:
- (a) Don't virtualize the **streaming row** — pin it to the tail and
  virtualize everything else.
- (b) Throttle live updates aggressively (60Hz+) so each batch lands as
  one resize, not 30/sec.
- (c) Use group-level virtualization (turns, not items) so the live
  row's growth only triggers ONE measurement per batch.

---

## Candidate strategies

Five candidates ranked roughly by complexity vs upside.

### A — Turn-level group virtualization (RECOMMENDED)

Virtualize at the **turn block** level instead of the **transcript item**
level. Each turn is one virtual row containing all of its items
(user prompt, agent reply, thoughts, tool cards, terminal cards).

```
Today (item-level conceptually): ~30 transcript items per ~3 turns
After (turn-level): ~3-10 virtual rows per ~3-10 turns
```

**Pros:**

- **Far fewer virtual rows.** Streaming agent chunks resize the
  CURRENT turn's row, not multiple item rows. ResizeObserver fires
  once per chunk per (live) row, not N times.
- **Block grouping already exists.** `timelineBlocksFromSnapshot` already
  produces `blocks` keyed by `turnId`; virtualizing over `blocks` is the
  natural unit.
- **Stable keys.** `block.groupKey` is stable across prepends + live
  updates (grouped by turnId, derived from item content not array
  index).
- **Streaming impact bounded.** Only 1 virtual row resizes per chunk
  (the live turn's). Captain reading history has ZERO measure traffic
  on history rows because they're stable.

**Cons:**

- A single turn can be VERY tall (long agent reply + multiple tool
  diffs ≈ 3-5 viewports). One virtual row taller than the viewport
  defeats the "render only what's visible" principle for that row.
  Mitigation: this is fine — the virtualizer still skips off-screen
  ROWS, and a single huge row gets rendered fully but at least it's
  the only one being rendered.
- Backward pagination prepends OLD turns at the top. Their measured
  heights are unknown until rendered. The
  `shouldAdjustScrollPositionOnItemSizeChange` predicate would compensate
  but caused our prior loop. We need a different prepend strategy
  (see implementation below).

**Implementation sketch:**

```ts
// useChatViewport already exposes `items` (oldest-first SeqTranscriptItem[])
// blocks computed in Viewport.vue groups by turnId already
// virtualize over `blocks`, NOT `items`
const virtualizer = useVirtualizer(computed(() => ({
  count: blocks.value.length,
  getScrollElement: () => scrollEl.value ?? null,
  estimateSize: () => 240,           // median turn block height
  overscan: 2,                        // 2 turn-blocks above + below
  getItemKey: (i) => blocks.value[i]?.groupKey ?? i,
  // shouldAdjustScrollPositionOnItemSizeChange: OMITTED (caused 92f025f loop)
})))

// Each Turn block in the v-for gets a `data-index` + stable measureRow ref.
// Avoid inline arrow refs — use a single closure captured in setup.
function measureRow(el: Element | null): void {
  if (el) virtualizer.value.measureElement(el)
}
```

**Mitigating the prepend-jump without `shouldAdjustScrollPositionOnItemSizeChange`:**

- Capture `scrollHeight` BEFORE the fetch settles.
- After the fetch lands AND virtual-core has measured the new top rows,
  set `scrollTop = scrollTop + (newScrollHeight - oldScrollHeight)`.
- Do this OUTSIDE the virtualizer's reactive cycle (e.g. in a
  `watch(viewport.items.length, ..., { flush: 'post' })` so it runs
  after Vue's render but before paint).

This is the same compensation the predicate did, but applied
out-of-band so it doesn't re-trigger the virtualizer's recompute.

**Streaming row carve-out (if measure loops still surface):** keep the
**newest** turn block out of the virtualizer entirely — render it
unconditionally as a sibling of the virtual list. This eliminates the
"row resizes while it's a virtual row" problem because the live row
isn't virtualized. When the turn ENDS (`TurnEnded`), commit it back
into the virtualizer's tracked range.

### B — `content-visibility: auto`

Modern CSS primitive that lets the browser skip layout + paint for
off-screen elements. The DOM still contains every node; the browser
treats them as zero-sized for layout until they enter the viewport.

```css
.chat-block {
  content-visibility: auto;
  contain-intrinsic-size: 0 200px; /* placeholder height before render */
}
```

**Pros:**

- **Almost zero JS.** Browser does the work.
- **No measurement loops.** Layout decisions live entirely in the
  browser engine.
- Plays well with streaming — only the visible row pays the cost.

**Cons:**

- WebKit2GTK 4.1 (the Tauri webview on Linux) has limited support;
  WebKit added it in 17.4 (2024) but our pinned webview lags. Need to
  test before committing.
- `contain-intrinsic-size` placeholder height is its own approximation
  — same fundamental problem as `estimateSize`. Wrong placeholder
  → janky scrollbar.
- All DOM is still allocated (just not laid out). Memory ceiling
  unchanged from current page-trim.

**Recommended as a fallback** if Strategy A's measurement issues prove
unavoidable. Lower upside, much lower risk.

### C — IntersectionObserver-based windowing

Hand-rolled "render only visible" without a virtualization library.
Each block gets an IntersectionObserver; off-screen blocks render a
placeholder div with last-known height; on-screen blocks render full
content.

**Pros:**

- Full control over the measurement loop (or absence thereof).
- No external dependency on TanStack Virtual.

**Cons:**

- We're rebuilding what TanStack Virtual already provides.
- Placeholder height tracking is still its own subtle problem.
- Risk of NIH; TanStack Virtual is maintained, ours wouldn't be.

**Not recommended** unless A and B both fail.

### D — Throttle live updates to 30Hz

Don't virtualize. Just rate-limit `setQueryData` patches so streaming
agent text updates the head page at most 30 times/sec instead of
~per-chunk.

**Pros:**

- Tiny diff (just add throttling to `flushPatches`).
- Cuts streaming render pressure by ~3-10×.

**Cons:**

- Doesn't address the unbounded-DOM concern (still have N items in
  cache × N items in DOM after page-trim).
- Visible chunk lag (~33ms) — captain might notice on faster networks.

**Worth doing regardless.** Even with virtualization, throttling to
30Hz means each batch fires one resize instead of many.

### E — Status quo (no virtualization)

Plain v-for over blocks; page-trim caps cache at `MAX_PAGES_KEPT × pageSize`
items. With viewport-relative page size that's ~30 items in DOM at
peak — 60 max during scroll-up + reading.

**Pros:**

- Already shipped. Stable. Tested.

**Cons:**

- DOM size grows linearly with cache. On a 4K monitor with a chat full
  of long replies and tool cards, we're not bounded by viewport.
- Captain's complaint: "still loads too much".

**Acknowledged baseline.** The real question is whether A + D buys
enough over E to be worth the complexity.

---

## Recommendation

**Strategy A + Strategy D, layered:**

1. **First land Strategy D** (throttle live patches to 30Hz). Tiny
   change. Test alone for a day to confirm streaming is smooth and the
   captain's "loads too much" feeling is reduced.
2. **If still wanting virtualization, then Strategy A** (turn-level
   group virtualization) with the streaming-row carve-out as the
   safety net.
3. **If Strategy A regresses streaming** (any "Maximum recursive
   updates exceeded" report from the captain), fall back to **Strategy
   B** (`content-visibility: auto`) AFTER confirming WebKit2GTK 4.1
   support — likely sufficient for memory bounds without JS-side
   measurement.

The ordering — D first, then A, then B — is risk-graded: D is a 5-line
change, A is a viewport rewrite, B requires browser-support
verification. We commit each step independently and roll back at any
point.

---

## Implementation steps (Strategy A; do not start until aligned)

### Phase 1: Strategy D (preflight)

1. Add a 30Hz throttle in `useChatViewport.flushPatches` so streaming
   patches batch over a 33ms window. Today they batch via microtask
   (one tick); upgrade to a setTimeout(33ms) trailing-edge flush.
2. Run the captain's mobile remote against a streaming reply session.
   Confirm: chat smooth, no jank, "loads too much" feeling reduced.
3. Ship if ✓.

### Phase 2: Strategy A scaffold

1. Re-introduce `@tanstack/vue-virtual` (it's still in `package.json`).
2. In `Viewport.vue`, wrap the `v-for blocks` render in a virtualizer
   keyed by `block.groupKey`. **Use turn blocks, not transcript items.**
3. **Stable measure callback:** capture `measureRow = (el) => virtualizer.value.measureElement(el)` once in setup. Bind via `:ref="measureRow"`.
4. **Drop `shouldAdjustScrollPositionOnItemSizeChange`** — known
   loop trigger. Compensate prepends out-of-band (Phase 3).
5. **Live row carve-out:** the head block (highest `block.groupKey`
   with an open turn) renders OUTSIDE the virtualizer as a sibling.
   Commit it back into the virtual range only when its turn ends.
6. `estimateSize` = 240px (median turn-block measured from screenshots).
7. `overscan: 2` (turn blocks are big; less overscan needed).

### Phase 3: Out-of-band prepend compensation

Replace `shouldAdjustScrollPositionOnItemSizeChange` with a watcher
that compensates AFTER the fetch settles + measurements complete:

```ts
let scrollHeightBeforeFetch = 0
async function fetchNextPage(): Promise<unknown> {
  const el = scrollEl.value
  scrollHeightBeforeFetch = el?.scrollHeight ?? 0
  const r = await viewport.fetchNextPage()
  await nextTick()                              // Vue's render tick
  await nextTick()                              // virtualizer's tick
  if (el && scrollHeightBeforeFetch > 0) {
    const delta = el.scrollHeight - scrollHeightBeforeFetch
    el.scrollTop += delta                       // anchor visible content
  }
  return r
}
```

Crucially this happens AFTER the virtualizer's reactive cycle, so
writing scrollTop doesn't trigger a re-measure.

### Phase 4: Stick-to-bottom via virtualizer

Replace the `useStickToBottom` watcher's auto-scroll with
`virtualizer.scrollToIndex(blocks.length - 1, { align: 'end' })`. Fires
inside `nextTick` after blocks length grows (live event added an item
to the live turn → block list grew or live block resized).

### Phase 5: Measurement audit

For 1 hour, run the daemon under
`RUST_LOG=info,acp::emit::chunk=trace,webview=trace` and instrument the
virtualizer with `console.count('measure')` in dev. Confirm: streaming
a 100-line agent reply produces O(100) measure calls, not O(10,000).

### Phase 6: Testing

- Existing tests in `Viewport.test.ts` cover backward fetch trigger +
  load chip — shouldn't regress.
- New test: `streaming row growth doesn't exceed N measure calls per
  chunk`. Assert via `console.count` interception.
- New test: `prepend doesn't yank visible content`. Snapshot scrollTop
  before/after backward fetch and assert visible content stays fixed
  (within ±2px tolerance for sub-pixel rounding).

---

## Risks + rollback

| Risk | Mitigation |
| --- | --- |
| Strategy A re-introduces measure loop under streaming | Live-row carve-out (Phase 2 step 5). Falls back to Strategy B. |
| TanStack Virtual + Vue 3 reactivity quirks | The `measureRow` stable-callback fix from `33218e4` is preserved. |
| Prepend jump | Out-of-band compensation (Phase 3); doesn't hit the predicate that caused 92f025f. |
| WebKit2GTK 4.1 `content-visibility` partial support | Verify before Strategy B; fall back to status-quo (E) if unsupported. |
| Stick-to-bottom drift | `scrollToIndex(last, 'end')` post-render; `useStickToBottom`'s `stuck` flag still drives the trigger. |

**Rollback:** every commit lands as its own commit on a feature branch.
Any regression → `git revert` the offending commit. The status quo
(plain v-for + page-trim) is the floor; we never ship something worse.

---

## Open questions (need to discuss before starting)

1. **Is the captain's "loads too much" complaint about render perf or
   bytes-on-wire?** If wire, viewport-relative page sizing already
   addresses it (no virtualization needed).
2. **What's the rendering cost target?** Frames-per-second target during
   streaming on the captain's phone? 30fps? 60fps?
3. **Should the live row stay non-virtualized permanently** (Strategy
   A's carve-out as a permanent feature, not just safety net)?
4. **Is throttling alone (Strategy D) acceptable as the final
   answer?** It might be; we should measure before committing to A.

---

## What I did NOT do this round

- Pure render perf measurement (no DevTools profile of streaming under
  current v-for + page-trim).
- WebKit2GTK 4.1 `content-visibility` support test.
- Prototype of any strategy.

These are deliberately deferred — they belong to whoever picks this up
tomorrow, with the directional choice made first.

---

## Pickup checklist for tomorrow

1. Read the prior commits' bodies in full: `git show e2e2404 5147fb3
   92f025f 33218e4 6790fd3 -- ui/src/views/chat/Viewport.vue`
2. Run a captured streaming session under `task run` with browser
   devtools profiler. Sample DOM count + frame rate. Decide if
   Strategy E (status quo) is genuinely insufficient.
3. If yes: ship Strategy D first (~10 LoC). Re-test.
4. If still insufficient: prototype Strategy A scaffold on a worktree
   branch off this one. Validate streaming + prepend behaviour
   in isolation before merging.
5. Update this doc with profile numbers, decisions, and outcomes.

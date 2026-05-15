# Hydration + Instance-Switch UX Fix Plan

> **For Claude:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` to implement this plan task-by-task.

**Goal:** Make instance switching, snapshot hydration, and lazy-load-older seamless: the viewport always lands on the latest message after a switch; the load-older trigger never fires during the switch; subsequent scroll-up loads older as expected; sticky-to-bottom re-arms on every flip.

**Architecture:** The bug is in `ui/src/views/chat/Viewport.vue` + `ui/src/composables/instance/use-chat-viewport.ts`. The viewport has a pre-existing empty `watch(instanceId, () => {})` placeholder (use-chat-viewport.ts:699-703). On instance flip, TanStack re-keys the infinite-query cache and Vue re-renders, but `scrollEl.scrollTop` stays at whatever value it held — usually `0` (the previous instance was scrolled near top OR the viewport just mounted). The `onScroll` handler fires `fetchNextPage()` when `scrollTop < 240px` regardless of "we just switched instances", which immediately loads ancient history of the newly-focused instance instead of letting the head page render. The fix is a four-line state machine: on `instanceId` change, set a `flipping` flag, wait for `nextTick`, anchor `scrollTop = scrollHeight`, clear the flag. Plus: gate `onScroll` on `!flipping`, and re-stick the `useStickToBottom` anchor.

**Tech Stack:** Vue 3 + TanStack Query (`useInfiniteQuery`), `@vueuse/core` (`useStickToBottom` lives in our `Body.vue` composables), vitest.

---

## Investigation summary

From the parallel exploration:

- **`Viewport.vue:257-288`** — `onScroll` fires `fetchNextPage` when `scrollTop < LOAD_MORE_THRESHOLD_PX = 240px`. No instance-flip guard.
- **`use-chat-viewport.ts:699-703`** — Pre-existing `watch(instanceId, () => {})` empty placeholder. Right place for our fix.
- **`use-snapshot-hydration.ts:132-136`** — Clears dedup sets on flip, but does NOT signal scroll reset.
- **`use-focus-prefetch.ts:227-240`** — On `AcpInstancesFocused`, prefetches meta + first chat page but doesn't touch scroll.
- **`use-instance-chat-infinite-query.ts:57-79`** — Query is keyed on `instanceId`; TanStack auto-swaps cache; `getNextPageParam` uses `oldestSeq` for back-pagination.
- **`Viewport.vue:120, 142-146`** — `useStickToBottom` runs MutationObserver + ResizeObserver; 64px threshold gates `stuck`. When `stuck=true`, auto-scrolls. Doesn't know about instance flips.
- **`ChatSnapshot` (`mirror.rs:550-579`)** — `before = None` returns latest `limit` items head-anchored. `latest_seq`, `oldest_seq`, `has_more` are populated. UI has everything it needs.
- **No backwards-compat concerns** — every consumer of the affected files is in-repo.

---

## Locked design decisions

1. **Jump to bottom on every flip, not last-known-scroll.** Captain's explicit ask: "always show the end of the instance when we switched." Don't restore per-instance scroll memory; that's a different feature for another PR.
2. **Gate `onScroll` during flip, not throttle it.** A guard flag is simpler and zero-cost; throttling would still allow a single fetch through.
3. **Single `watch(instanceId, ...)` in `use-chat-viewport.ts`** — that's where the existing placeholder is + where the scroll element ref is in scope. Don't sprinkle logic across `Viewport.vue` + the composable.
4. **`useStickToBottom` re-arm via `scrollIntoView({ block: 'end' })`** rather than messing with its internals. The observer will see the scroll change and re-arm `stuck=true`.
5. **No `next, prev` early-exit on first mount.** The watcher fires once on registration (when `instanceId` first resolves). That first run should ALSO anchor to bottom — the captain wants to land at the latest message on initial boot too.
6. **Concurrent flip race**: if the captain spam-switches A→B→C within the `nextTick` window, the latest flip's anchor wins. Use a per-flip token to invalidate stale anchors.
7. **Empty-instance edge case**: instance with zero items → `scrollHeight === clientHeight` → no-op. Fine.

---

## Wire shape (no changes)

No daemon-side changes. The wire already gives us `latest_seq`, `has_more`, head-anchored default page. Pure frontend bug.

---

## File map

**Modified (Vue UI only):**
- `ui/src/composables/instance/use-chat-viewport.ts` — flesh out the empty `watch(instanceId)` block. Add `isFlippingInstance: Ref<boolean>` to the exposed API; bind it during flip; clear it after anchor.
- `ui/src/views/chat/Viewport.vue` — `onScroll` guards on `!isFlippingInstance.value`; ALSO call `scrollToBottom()` via `useStickToBottom`'s exposed method (or `scrollEl.scrollTop = scrollEl.scrollHeight` directly) inside the watcher.

**Added (tests):**
- `ui/src/composables/instance/use-chat-viewport-instance-flip.test.ts` — new file pinning:
  - Flip resets scrollTop to bottom after nextTick
  - `fetchNextPage` does NOT fire during the flip window
  - Subsequent scroll-up DOES trigger `fetchNextPage`
  - Empty instance (no items) doesn't crash

---

## Tasks

### Task 1: Expose a `isFlippingInstance` ref from `useChatViewport`

**Files:**
- Modify: `ui/src/composables/instance/use-chat-viewport.ts` (around line 699, replace the empty watch + extend the returned API).

**Step 1: Write the failing test.**

Create `ui/src/composables/instance/use-chat-viewport-instance-flip.test.ts`:

```ts
import { describe, expect, it, vi } from 'vitest'
import { defineComponent, h, nextTick, ref } from 'vue'
import { mount } from '@vue/test-utils'
import { useChatViewport } from './use-chat-viewport'

vi.mock('@ipc', async() => ({
  ...(await vi.importActual<object>('@ipc')),
  invoke: vi.fn().mockResolvedValue({ items: [], latestSeq: undefined, oldestSeq: undefined, hasMore: false }),
  listen: () => Promise.resolve(() => {})
}))

describe('useChatViewport instance-flip', () => {
  it('exposes isFlippingInstance ref', () => {
    const TestComp = defineComponent({
      setup() {
        const id = ref<string | undefined>('A')
        const scrollEl = ref<HTMLElement | undefined>(undefined)
        const vp = useChatViewport(id, { scrollEl })
        return { vp }
      },
      render() { return h('div') }
    })
    const wrapper = mount(TestComp)
    expect(wrapper.vm.vp.isFlippingInstance).toBeDefined()
    expect(wrapper.vm.vp.isFlippingInstance.value).toBe(false)
  })
})
```

**Step 2: Run + verify fail.**

```bash
pnpm --filter hyprpilot-ui test use-chat-viewport-instance-flip
```

Expected: FAIL — `isFlippingInstance` undefined on the return.

**Step 3: Add `isFlippingInstance` to the API.**

In `use-chat-viewport.ts`, replace the empty watcher:

```ts
const isFlippingInstance = ref(false)

watch(instanceId, (next, prev) => {
  if (next === prev) {
    return
  }
  // Every instance flip (and the initial mount) arms a brief anchor
  // window. `onScroll` gates fetchNextPage during this window so the
  // viewport doesn't grab ancient history just because the new
  // instance's scroll position happens to be near the top before the
  // head page has rendered. The window closes after the scroll-to-
  // bottom microtask resolves.
  isFlippingInstance.value = true
}, { immediate: true })
```

Add to the returned object:

```ts
return {
  // ... existing fields
  isFlippingInstance,
}
```

**Step 4: Run + verify pass.**

**Step 5: Commit.**

```bash
git add ui/src/composables/instance/use-chat-viewport.ts ui/src/composables/instance/use-chat-viewport-instance-flip.test.ts
git commit -m "feat(viewport): expose isFlippingInstance flag on useChatViewport"
```

---

### Task 2: Anchor scroll to bottom on flip; gate `onScroll`

**Files:**
- Modify: `ui/src/views/chat/Viewport.vue` (around line 88-97 — wire the flip handler; lines 257-288 — guard `onScroll`).

**Step 1: Write the failing test.** Extend the same test file:

```ts
it('sets scrollTop to scrollHeight on instance flip after nextTick', async() => {
  const scrollDiv = document.createElement('div')
  Object.defineProperty(scrollDiv, 'scrollHeight', { value: 1000, configurable: true })
  Object.defineProperty(scrollDiv, 'clientHeight', { value: 300, configurable: true })
  scrollDiv.scrollTop = 50

  const TestComp = defineComponent({
    setup() {
      const id = ref<string | undefined>('A')
      const scrollEl = ref<HTMLElement | undefined>(scrollDiv)
      const vp = useChatViewport(id, { scrollEl })
      return { vp, id }
    },
    render() { return h('div') }
  })
  const wrapper = mount(TestComp)

  // Initial mount fires the watcher once; anchor lands.
  await nextTick()
  expect(scrollDiv.scrollTop).toBe(1000)

  // Flip → re-anchor.
  scrollDiv.scrollTop = 100
  wrapper.vm.id = 'B'
  await nextTick()
  expect(scrollDiv.scrollTop).toBe(1000)
})

it('keeps isFlippingInstance true until nextTick resolves, then clears', async() => {
  const TestComp = defineComponent({
    setup() {
      const id = ref<string | undefined>('A')
      const scrollEl = ref<HTMLElement | undefined>(document.createElement('div'))
      const vp = useChatViewport(id, { scrollEl })
      return { vp, id }
    },
    render() { return h('div') }
  })
  const wrapper = mount(TestComp)
  wrapper.vm.id = 'B'
  // The watcher set the flag synchronously.
  expect(wrapper.vm.vp.isFlippingInstance.value).toBe(true)
  await nextTick()
  // After nextTick + microtask flush the anchor + clear should be done.
  await nextTick()
  expect(wrapper.vm.vp.isFlippingInstance.value).toBe(false)
})
```

**Step 2: Run + verify fail.**

**Step 3: Implement the anchor + clear inside the watcher.**

```ts
watch(instanceId, (next, prev) => {
  if (next === prev) {
    return
  }
  isFlippingInstance.value = true
  const flipToken = ++flipSeq
  void nextTick().then(() => {
    if (flipToken !== flipSeq) {
      // A newer flip superseded us — let the latest one anchor.
      return
    }
    const el = options.scrollEl.value
    if (el) {
      el.scrollTop = el.scrollHeight
    }
    isFlippingInstance.value = false
  })
}, { immediate: true })
```

Where `flipSeq` is a module-local `let flipSeq = 0` — invalidates stale anchors when the captain spam-switches A→B→C inside one render frame.

**Step 4: Run + verify pass.**

**Step 5: Update `Viewport.vue`'s `onScroll`** to gate on `!isFlippingInstance.value`:

```vue
const viewport = useChatViewport(instanceId, { scrollEl })

function onScroll(): void {
  if (viewport.isFlippingInstance.value) {
    return
  }
  // ... existing logic
}
```

**Step 6: Add a guard-fires test:**

```ts
it('does not fetchNextPage during the flip window', async() => {
  const fetchNextPageSpy = vi.fn()
  // ... set up viewport that exposes fetchNextPage, force isFlippingInstance=true,
  // simulate onScroll fire, assert spy not called.
})
```

**Step 7: Commit.**

```bash
git add ui/src/composables/instance/use-chat-viewport.ts ui/src/views/chat/Viewport.vue ui/src/composables/instance/use-chat-viewport-instance-flip.test.ts
git commit -m "fix(viewport): anchor to bottom + gate load-older on instance flip"
```

---

### Task 3: Re-arm `useStickToBottom` on flip

**Background**: `useStickToBottom`'s `stuck` flag goes false the moment the captain scrolls more than 64px above bottom. On instance flip, even though we anchor `scrollTop = scrollHeight`, the captain might have scrolled the OLD instance up; `stuck` is `false`; the next live event for the new instance won't auto-scroll. We need to re-arm.

**Files:**
- Modify: `ui/src/views/chat/Viewport.vue` — wherever `useStickToBottom` is initialized, expose a `stickToBottom()` imperative method (or read the existing one if `@vueuse/core` provides it) and call it inside the flip handler.

**Step 1: Inspect `useStickToBottom`'s API.**

The current binding is `const { stuck } = useStickToBottom(scrollEl)`. Check if it exposes a `stickToBottom()` re-arm method. If yes, call it.

**Step 2: If no API exposed**, do `scrollEl.value?.scrollIntoView({ block: 'end' })` or `el.scrollTop = el.scrollHeight` inside the flip handler (already done in Task 2), and let the MutationObserver re-evaluate `stuck` based on the new position.

**Step 3: Add a test asserting the new-message-after-flip auto-scrolls.**

(Likely needs a more integrated harness — mount Viewport.vue, push a live transcript chunk, assert scrollTop stays at bottom. Defer if too complex for unit-level.)

**Step 4: Commit.**

---

### Task 4: Manual smoke checklist

Run `task run` (dev mode). Verify:

1. **Cold boot, single instance, scrollable history** → viewport lands at the LATEST message, not the top.
2. **Cold boot, two instances** → focus on B, then click instance A → viewport jumps to A's bottom. Click B → jumps to B's bottom.
3. **Scroll A up to load older pages** → confirm `fetchNextPage` fires, older messages prepend, scroll anchor preserved.
4. **From mid-scroll position on A, switch to B** → B viewport at bottom; no spurious older-page fetch on A or B; spinner / loading indicator absent.
5. **Spam-switch A→B→C→D quickly** → final viewport at D's bottom; no stuck "loading older" state.
6. **Empty instance** (fresh spawn) → no crash; viewport renders empty state cleanly.
7. **From bottom of A, switch to B (also at bottom)** → B re-arms `stuck`; new live message in B auto-scrolls.
8. **`task lint && task test && task build`** green.

---

### Task 5: Final wrap

```bash
mise exec -- task lint
mise exec -- task test
mise exec -- task build
```

Commit any cleanup. `git push -u origin fix/hydration-instance-switch`. Open PR with:
- Bug description (scrollTop-at-top → load-older fires after flip → captain sees ancient history)
- The flip-window guard pattern
- Manual smoke from Task 4
- Mention this is purely UI — no daemon changes.

---

## Risk register

| Risk | Mitigation |
|---|---|
| `nextTick` resolves before TanStack's `useInfiniteQuery` finishes fetching the new instance's head page → `scrollHeight` is still the OLD instance's value → anchor lands at wrong position | Watch `[instanceId, latestSeq]` together; re-anchor when `latestSeq` first publishes for the new instance OR (alternative) chain on `prefetchInstanceChatFirstPage`'s promise. |
| Captain previously had a saved scroll position; this PR drops it | Out of scope per the captain's explicit ask. Note as a possible follow-up. |
| The `flipSeq` token doesn't invalidate cleanly when the captain triggers a flip mid-anchor | Token uses a strict `++flipSeq` + closure-captured comparison; stale anchors no-op cleanly. |
| `useStickToBottom` doesn't expose a re-arm method → Task 3 is a no-op | Falling back to `scrollTop = scrollHeight` works for the captain's stated case; the auto-scroll-on-new-message re-arm is a secondary concern. |

---

## Definition of done

- `task lint && task test && task build` green.
- New tests pin: flip anchors to bottom, fetchNextPage gated during flip, empty instance no-op, spam-switch invalidates stale anchors.
- Manual smoke from Task 4 walks cleanly on `task run`.
- PR body documents the bug + fix + smoke evidence.

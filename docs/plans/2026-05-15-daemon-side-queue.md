# Daemon-Side Queue Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` to implement this plan task-by-task.

**Goal:** Move the per-instance FIFO submit queue from the Vue UI into the daemon so every frontend (Vue desktop, Vue remote, hyprpilot-nvim, future clients) reads + mutates the same authoritative queue over the existing RPC + event bus.

**Architecture:** The queue lives on the per-instance `AcpInstance` actor, mutated via new `InstanceCommand` variants, broadcast as a new `InstanceEvent::QueueChanged` (full state, idempotent), cached in `InstanceMirror` for snapshot reads, and exposed as `queue/*` JSON-RPC handlers plus `queue_*` Tauri commands. The Vue UI's `use-queue.ts` becomes a thin read-through cache: writes go through `invoke()`, reads come from the live event broadcast + initial snapshot. The composer's "submit while busy" + the QueueStrip's per-row buttons re-route through the daemon. The desktop Tauri overlay, the remote-bridge WS, and any future client (nvim plugin, ctl scripts) all share one queue per instance.

**Tech Stack:** Rust (tokio, async-trait, serde), Vue 3 + TypeScript, Tauri 2, vitest + cargo-nextest.

---

## Plan revisions from peer review

Applied after autonomous review pass — critical issues + decisions on the previously-open questions:

- **C1 — Palette consumer.** `ui/src/views/palette/instances.ts:28` reads `useQueue(entry.instanceId)` for the `q<N>` badge. Hydration is now "refresh on first observation per instanceId" (a Map of "seen" ids in `use-queue.ts`). Triggered both by the live router (Task 11) and by `useQueue(id)` itself when called with an unseen id.
- **C2 — `enqueued_seq: u64`** added to `QueueItem` alongside `enqueued_at`. Per-instance monotonic sequence assigned by the actor at enqueue time. Frontends order by `enqueued_seq`; `enqueued_at` is informational ("queued 4s ago") only. Resolves the rapid-double-enqueue tie.
- **C3 — Cancel during dispatch: item is lost.** Pinned semantic. The popped item is gone the moment `queue/dispatch` starts the prompt future — a concurrent `Cancel` only kills the in-flight turn, never re-inserts. Document on the actor command + the RPC verb.
- **C4 — `queue/clear` mid-dispatch** clears the tail only (head is already popped). Captain wanting to also kill the in-flight turn calls `prompts/cancel` + `queue/clear` (two calls). Document on the verb.
- **C5 — Task 12 + 13 merge.** The dispatcher stubs (`startQueueDispatcher` / `stopQueueDispatcher`) and the local-store helpers are deleted in the SAME task as the Overlay rewires — otherwise the intermediate commit has compile errors. Renumbered as Task 12 (combined) + the original Task 13 folded in.
- **Suggestion: `queue/replace { items: [] }`** added — single broadcast for drag-reorder.
- **Suggestion: `queue/enqueue` reply** includes the full `items` array so optimistic UIs can render immediately without waiting for the broadcast.
- **Suggestion: extra tests** — `events/lagged` recovery via re-fetch, concurrent-enqueue FIFO, restart-preserves-queue (Q1 pinned: queue survives `instances/restart`).
- **Suggestion: boot snapshot follow-up** — note that `daemon/boot_snapshot` should eventually include per-instance queues so second-frontends avoid N+1 on connect. Out of this PR.
- **Q5 — wire-size soft cap deferred.** No cap in this PR. Captains pasting multi-MB images are a "not happened yet" problem; add when measured.
- **Q6 — `refresh()` trigger location.** First-observation logic lives inside `use-queue.ts`'s `useQueue(id)` — call site doesn't need to know. The live router (`use-session-stream.ts`) also calls `markObserved(id)` whenever an `acp:queue-changed` event lands so the seen-set stays warm.
- **Task 4 split** into 4a (enqueue + list) and 4b (remove + move + insert + clear). Dispatch is its own Task 7 already.

---

## Architectural decisions (locked before implementation)

1. **Actor-owned, mirror-cached.** The actor serialises queue mutations (same pattern as turn state). The mirror's `apply()` writes through on `QueueChanged` events for snapshot reads. No two-sources-of-truth.

2. **Full-state broadcasts, not deltas.** Every queue change emits `QueueChanged { items: Vec<QueueItem> }` carrying the full current queue. Queues are small (<20 items typical); deltas add complexity without measurable gain.

3. **Unified `Attachment` wire shape.** Today the UI splits `pills: ComposerPill[]` (image data URLs + skill resources for preview) and `skillAttachments: Attachment[]` (wire shape). The daemon's `QueueItem` carries a single `attachments: Vec<Attachment>` — both kinds. The Vue UI now sends skill + image attachments through the same field on enqueue; preview re-derives chip labels from `attachment.title`. **Out of scope for this PR**: queueing raw paste-image-from-clipboard pills that haven't been hydrated into an `Attachment`. Document this in the queue module.

4. **Dispatch happens server-side.** `queue/dispatch { instanceId, itemId? }` pops the queue head (or a specific item) AND calls the existing `submit_prompt` path on the adapter — no client-side `useAdapter().submit()` round-trip. The reply mirrors the `prompts/send` reply shape (`accepted`, `disposition`, `sessionId?`).

5. **No auto-dispatch.** The daemon never automatically dispatches the queue head on `TurnEnded`. Captain remains explicit ("Ctrl+Enter" / per-row button). This pins the contract `PR 1 (#66)` documented.

6. **Cancel does not touch the queue.** Pinned existing behaviour. Daemon-side `cancel_turn` (and the `Cancel` actor command) explicitly do NOT clear the queue.

7. **Hydration on connect/focus.** Boot snapshot does not include the queue (typically empty at boot). On first focus / first reconnect, the UI calls `queue/list { instanceId }` to seed, then `events/subscribe` streams live updates. Same pattern other per-instance state (terminals, transcript pagination) uses.

8. **Captain controls = idempotent.** `queue/enqueue` is NOT idempotent (each call appends). The UI must not retry a failed enqueue blindly. Document this in the wire-types comment.

---

## Wire types (will live in `src-tauri/src/adapters/queue.rs`)

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QueueItem {
    /// Server-minted UUID v4. Stable across re-renders + reorder
    /// operations. Frontends use this as the `:key` for list rendering.
    pub id: String,
    pub text: String,
    /// Wire-shape attachments — skill resources, image data, etc.
    /// Same shape `session/prompt` expects.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
    /// Per-instance monotonic counter. Frontends order by this; ties
    /// can't happen because the actor mints it under a single-mailbox
    /// lock. Drives the QueueStrip `:key` so re-renders stay stable
    /// across reorder operations.
    pub enqueued_seq: u64,
    /// ms-epoch at enqueue time. Display-only ("queued 4s ago"). NOT
    /// load-bearing for ordering — that's `enqueued_seq`'s job.
    pub enqueued_at: i64,
}
```

### `InstanceEvent::QueueChanged`

```rust
QueueChanged {
    agent_id: String,
    instance_id: String,
    items: Vec<QueueItem>,
}
```

- `topic()` → `"instance.queue_changed"`
- Tauri event name → `"acp:queue-changed"`

### Actor commands (added to `InstanceCommand`)

```rust
QueueEnqueue { item: QueueItemDraft, reply: oneshot::Sender<Result<QueueItem, AdapterError>> },
QueueInsert { position: usize, item: QueueItemDraft, reply: oneshot::Sender<Result<QueueItem, AdapterError>> },
QueueRemove { item_id: String, reply: oneshot::Sender<Result<bool, AdapterError>> },
QueueMove { item_id: String, position: usize, reply: oneshot::Sender<Result<bool, AdapterError>> },
QueueClear { reply: oneshot::Sender<Result<u32, AdapterError>> },
QueueList { reply: oneshot::Sender<Vec<QueueItem>> },
QueueDispatch { item_id: Option<String>, reply: oneshot::Sender<Result<QueueDispatchResult, AdapterError>> },
```

`QueueDispatchResult` matches `prompts/send`'s reply:
```rust
pub struct QueueDispatchResult {
    pub item: QueueItem,             // the popped item
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub accepted: bool,
}
```

### RPC verbs (under `queue/*` namespace)

| Verb | Params | Returns |
|---|---|---|
| `queue/list` | `{ instanceId? }` | `{ items: QueueItem[] }` |
| `queue/edit` | `{ instanceId, itemId, text, attachments?: Attachment[] }` | `{ item: QueueItem }` |
| `queue/remove` | `{ instanceId, itemId }` | `{ removed: bool }` |
| `queue/move` | `{ instanceId, itemId, position: u32 }` | `{ moved: bool }` |
| `queue/clear` | `{ instanceId }` | `{ cleared: u32 }` |
| `queue/dispatch` | `{ instanceId, itemId? }` | `{ item, sessionId?, turnId?, accepted }` |

All take an optional `instanceId` that falls back to the focused instance (same convention as `prompts/cancel`, `permissions/respond`).

### Tauri commands (1:1 mirror for the Vue UI)

`queue_list`, `queue_enqueue`, `queue_insert`, `queue_remove`, `queue_move`, `queue_clear`, `queue_dispatch`. The `TauriProxyHandler` already auto-proxies `tauri/<cmd>` to each, so remote-WS clients reach them through the same RPC envelope.

---

## File map

**New files:**
- `src-tauri/src/adapters/queue.rs` — `QueueItem`, `QueueItemDraft`, `QueueDispatchResult`
- `src-tauri/src/rpc/handlers/queue.rs` — `QueueHandler`
- `src-tauri/src/rpc/handlers/queue.rs` test module (inline)
- `ui/src/composables/instance/use-queue.ts` — rewritten as daemon mirror
- `ui/src/interfaces/wire/queue.ts` — `QueueItem` + dispatch result type
- `ui/src/composables/instance/use-queue.test.ts` — rewritten against `vi.mock('@ipc')`

**Modified:**
- `src-tauri/src/adapters/mod.rs` — re-export queue module
- `src-tauri/src/adapters/instance.rs` — new `InstanceEvent::QueueChanged` variant + topic / event-name arms
- `src-tauri/src/adapters/mirror.rs` — `MirrorInner.queue: Vec<QueueItem>`, `apply` arm, `queue_snapshot()` method
- `src-tauri/src/adapters/acp/instance.rs` — new actor commands + handlers
- `src-tauri/src/adapters/acp/instances.rs` — facade methods for enqueue/list/etc; auto-emit `QueueChanged` after state mutations
- `src-tauri/src/adapters/commands.rs` — seven new `#[tauri::command]` fns
- `src-tauri/src/main.rs` — register the new commands in `.invoke_handler`
- `src-tauri/src/rpc/mod.rs` — add `Box::new(QueueHandler)` to `with_defaults`
- `src-tauri/src/rpc/handlers/mod.rs` — re-export `QueueHandler`
- `src-tauri/src/rpc/handlers/util.rs` (if needed) — instance-id resolution helper if not shared yet
- `ui/src/ipc/commands.ts` — seven new `TauriCommand` enum entries + event enum entry
- `ui/src/interfaces/ipc/invoke.ts` — argument shapes + result map entries
- `ui/src/interfaces/ipc/types.ts` (if exists) — `acp:queue-changed` payload type
- `ui/src/views/Overlay.vue` — composer's enqueue path goes through `invoke`; queue strip handlers go through `invoke`
- `ui/src/composables/instance/use-session-stream.ts` — listen for `acp:queue-changed`, fan into the new store
- `ui/src/composables/instance/cleanup.ts` — drop `resetQueue` call (queue lives daemon-side now; stale on reconnect is handled by re-fetch)

**Deleted (or emptied):**
- The local-store internals (`pushToQueue`, `popQueueHead`, etc.) — replaced by the invoke wrappers. Existing test file gets rewritten.

---

## Task sequence

### Task 1: Define `QueueItem` + draft type

**Files:**
- Create: `src-tauri/src/adapters/queue.rs`
- Modify: `src-tauri/src/adapters/mod.rs:18` (add `pub mod queue;`)

**Step 1: Write the failing test (in `queue.rs`)**
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn queue_item_serialises_to_camel_case() {
        let item = QueueItem {
            id: "q-1".into(),
            text: "hello".into(),
            attachments: vec![],
            enqueued_at: 1700000000,
        };
        let v = serde_json::to_value(&item).expect("serialise");
        assert_eq!(v["id"], "q-1");
        assert_eq!(v["text"], "hello");
        assert_eq!(v["enqueuedAt"], 1700000000);
        assert!(v.get("attachments").is_none(), "empty attachments must drop off the wire");
    }
}
```

**Step 2: Run + verify fail**
```bash
cargo test --manifest-path src-tauri/Cargo.toml adapters::queue::tests
```

**Step 3: Implement the type** (per the wire-types section).

**Step 4: Run + verify pass.**

**Step 5: Commit**
```bash
git add src-tauri/src/adapters/queue.rs src-tauri/src/adapters/mod.rs
git commit -m "feat(adapters): add QueueItem wire type"
```

---

### Task 2: Add `QueueChanged` event variant

**Files:**
- Modify: `src-tauri/src/adapters/instance.rs` (add enum variant ~line 86-175, topic arm ~line 459, event_name arm ~line 488)

**Step 1: Test** — add to the existing `mod tests` block in `instance.rs`:
```rust
#[test]
fn queue_changed_event_topic_and_name() {
    let evt = InstanceEvent::QueueChanged {
        agent_id: "claude-code".into(),
        instance_id: "i-1".into(),
        items: vec![],
    };
    assert_eq!(evt.topic(), "instance.queue_changed");
    assert_eq!(evt.event_name(), "acp:queue-changed");
}
```

**Step 2: Run + verify fail.**

**Step 3: Implement** — variant + both match arms.

**Step 4: Run + verify pass.**

**Step 5: Commit**
```
feat(adapters): add InstanceEvent::QueueChanged variant
```

---

### Task 3: Mirror caches the queue

**Files:**
- Modify: `src-tauri/src/adapters/mirror.rs` — add `queue: Vec<QueueItem>` to `MirrorInner`, `apply` arm for `QueueChanged`, `queue_snapshot() -> Vec<QueueItem>` method.

**Step 1: Test** (TDD — write first)
```rust
#[tokio::test]
async fn mirror_queue_apply_overwrites_with_full_state() {
    let mirror = InstanceMirror::new();
    let evt_a = InstanceEvent::QueueChanged {
        agent_id: "claude-code".into(),
        instance_id: "i-1".into(),
        items: vec![sample_queue_item("q-1")],
    };
    mirror.apply(&evt_a).await;
    assert_eq!(mirror.queue_snapshot().await.len(), 1);

    let evt_b = InstanceEvent::QueueChanged {
        agent_id: "claude-code".into(),
        instance_id: "i-1".into(),
        items: vec![sample_queue_item("q-2"), sample_queue_item("q-3")],
    };
    mirror.apply(&evt_b).await;
    let snap = mirror.queue_snapshot().await;
    assert_eq!(snap.len(), 2);
    assert_eq!(snap[0].id, "q-2");
}
```

**Step 2-4: TDD cycle.**

**Step 5: Commit**
```
feat(mirror): cache queue items + emit through apply()
```

---

### Task 4: Actor commands + queue state

**Files:**
- Modify: `src-tauri/src/adapters/acp/instance.rs`:
  - Add per-actor `queue: VecDeque<QueueItem>` to the actor's local state (alongside `turn_state`)
  - Add `InstanceCommand::{QueueEnqueue, QueueInsert, QueueRemove, QueueMove, QueueClear, QueueList, QueueDispatch}` variants
  - Handle each in the actor's main `select!` loop
  - Every queue mutation emits `InstanceEvent::QueueChanged` via the standard `publish()` path (so mirror + broadcast subscribers stay in sync)
  - On `QueueDispatch`, pop the head (or named item), inline the same prompt-build path as `InstanceCommand::Prompt`. Reply with `QueueDispatchResult`.

**Step 1: Tests** (start with enqueue/list/remove — dispatch arrives later)
```rust
#[tokio::test]
async fn actor_enqueue_appends_and_emits() {
    let (handle, _events) = spawn_test_actor().await;
    let (tx, rx) = oneshot::channel();
    handle.send(InstanceCommand::QueueEnqueue {
        item: QueueItemDraft { text: "hi".into(), attachments: vec![] },
        reply: tx,
    }).await.unwrap();
    let item = rx.await.unwrap().unwrap();
    assert_eq!(item.text, "hi");

    let (tx, rx) = oneshot::channel();
    handle.send(InstanceCommand::QueueList { reply: tx }).await.unwrap();
    let items = rx.await.unwrap();
    assert_eq!(items.len(), 1);
}
```

Build out tests for `remove`, `insert`, `move`, `clear`, and `dispatch-empty-noop` first; full `dispatch` ties into the prompt path (Task 7).

**Step 2-4: TDD per command.**

**Step 5: Commit** (after every 2-3 commands)
```
feat(actor): add QueueEnqueue / QueueInsert / QueueRemove
feat(actor): add QueueMove / QueueClear / QueueList
```

---

### Task 5: Adapter facade methods

**Files:**
- Modify: `src-tauri/src/adapters/acp/instances.rs` — add `enqueue_item`, `remove_queue_item`, `move_queue_item`, `clear_queue`, `list_queue`, `insert_queue_item`, `dispatch_queue_item` on `AcpAdapter`. These resolve `InstanceKey` → look up the live actor → send the command → await the reply.

**Step 1-4: TDD each method via the existing `adapter_with_dead_child` test harness.**

**Step 5: Commit**
```
feat(adapter): facade methods for queue operations
```

---

### Task 6: RPC handler `queue/*`

**Files:**
- Create: `src-tauri/src/rpc/handlers/queue.rs` — `QueueHandler` implementing `RpcHandler`
- Modify: `src-tauri/src/rpc/handlers/mod.rs` — re-export `QueueHandler`
- Modify: `src-tauri/src/rpc/mod.rs:55` — add `Box::new(QueueHandler)` to `with_defaults`

**Param types per verb** (deserialised with `deny_unknown_fields`):
```rust
#[derive(Deserialize)] struct ListParams { instance_id: Option<String> }
#[derive(Deserialize)] struct EnqueueParams { instance_id: Option<String>, text: String, attachments: Vec<Attachment> }
// ...
```

**Step 1: Tests** — follow the `tests` pattern in `prompts.rs` (`dispatch` helper with an `AcpAdapter` set up over a dead child). One per verb covering happy + invalid-params + no-such-instance + empty-text.

**Step 2-4: TDD per verb.**

**Step 5: Commit** (after each pair of verbs)
```
feat(rpc): queue/list + queue/enqueue
feat(rpc): queue/remove + queue/move + queue/clear
feat(rpc): queue/insert + queue/dispatch
```

---

### Task 7: Wire dispatch to the prompt path

**Files:**
- Modify: `src-tauri/src/adapters/acp/instance.rs` — `QueueDispatch` handler. Build the prompt blocks the same way `InstanceCommand::Prompt` does (`build_prompt_blocks(text, &attachments)`); spawn the same prompt future; resolve `QueueDispatchResult` with the eventual `accepted` + `session_id` + `turn_id`. The dispatch must remove the item from the queue BEFORE the prompt fires (so a concurrent re-dispatch can't double-fire the same item).

**Step 1: Test** — pin "dispatch pops the item then submits", "dispatch on empty queue returns `accepted: false`", "dispatch by id removes that specific item".

**Step 2-4: TDD.**

**Step 5: Commit**
```
feat(actor): wire queue/dispatch through the prompt path
```

---

### Task 8: Tauri command bridge

**Files:**
- Modify: `src-tauri/src/adapters/commands.rs` — `queue_list`, `queue_enqueue`, `queue_insert`, `queue_remove`, `queue_move`, `queue_clear`, `queue_dispatch`
- Modify: `src-tauri/src/main.rs` — register them in `.invoke_handler(tauri::generate_handler![ … ])`

Each command resolves `Option<String>` instance id (falling back to focused), then forwards to the matching facade method.

**Step 1: Tests** — extend `commands.rs` test module if it has one (otherwise unit-test via `tauri::Builder::default()` smoke).

**Step 2-4: TDD if straightforward; otherwise rely on the RPC-handler tests + a manual round-trip during Task 14.

**Step 5: Commit**
```
feat(commands): Tauri queue_* bridge
```

---

### Task 9: Vue `QueueItem` wire type + `@ipc` enum entries

**Files:**
- Create: `ui/src/interfaces/wire/queue.ts` — mirrors the Rust shape
- Modify: `ui/src/interfaces/wire/index.ts` — re-export
- Modify: `ui/src/ipc/commands.ts` — add seven `TauriCommand` entries + one `TauriEvent.AcpQueueChanged = 'acp:queue-changed'`
- Modify: `ui/src/interfaces/ipc/invoke.ts` — argument + result shapes keyed off the enum entries
- Modify: `ui/src/interfaces/ipc/types.ts` (or wherever event payload types live) — `AcpQueueChangedPayload`

**Step 1: Test** — `invoke<TauriCommand.QueueEnqueue>` should infer the correct return type at the type system level. Add a `.ts` file in `tests/` that exercises this via `expectTypeOf()` (vitest-style).

**Step 2-4: TDD types (compile-time checks).**

**Step 5: Commit**
```
feat(ipc): wire types + invoke arg shapes for queue commands
```

---

### Task 10: Rewrite `use-queue.ts` as a daemon mirror

**Files:**
- Rewrite: `ui/src/composables/instance/use-queue.ts`

New shape (sketch):

```ts
const store = reactive(new Map<InstanceId, QueueItem[]>())

export function applyQueueChanged(id: InstanceId, items: QueueItem[]): void {
  store.set(id, items)
}

export interface UseQueueApi {
  items: ComputedRef<QueueItem[]>
  enqueue: (text: string, attachments?: Attachment[]) => Promise<QueueItem | undefined>
  insert: (position: number, text: string, attachments?: Attachment[]) => Promise<QueueItem | undefined>
  remove: (itemId: string) => Promise<void>
  dispatch: (itemId?: string) => Promise<void>
  move: (itemId: string, position: number) => Promise<void>
  clear: () => Promise<void>
  /** Force-refresh from the daemon — call on focus / reconnect. */
  refresh: () => Promise<void>
}

export function useQueue(instanceId?: InstanceId): UseQueueApi { /* ... */ }
```

**Step 1: Tests** — rewrite `use-queue.test.ts` with `vi.mock('@ipc')` style. Pin: enqueue calls invoke with right shape, dispatch calls invoke, `applyQueueChanged` mutates the store, the computed `items` resolves through active id, refresh fetches via `QueueList`.

**Step 2-4: TDD.**

**Step 5: Commit**
```
refactor(ui): use-queue.ts becomes a daemon-side mirror
```

---

### Task 11: Wire `acp:queue-changed` into the live router

**Files:**
- Modify: `ui/src/composables/instance/use-session-stream.ts` — add `await listen(TauriEvent.AcpQueueChanged, (e) => applyQueueChanged(e.payload.instanceId, e.payload.items))` alongside the other listeners

**Step 1: Test** — touch the existing snapshot-stream test or add a new spec proving the listener fans into the store.

**Step 2-4: TDD.**

**Step 5: Commit**
```
feat(stream): route acp:queue-changed into the queue store
```

---

### Task 12: Composer + Overlay integration

**Files:**
- Modify: `ui/src/views/Overlay.vue`:
  - `onSubmit`'s busy branch (line ~787-796 of current main): replace `pushToQueue(instanceId, { ... })` with `await invoke(TauriCommand.QueueEnqueue, { instanceId, text, attachments: [skillAttachments, ...imageAttachments] })`. Image pills get converted to `Attachment` (`{ data, mime, slug, path }`) here.
  - `editing.position` re-submit (line ~772-777): replace `pushToQueueAt` with `invoke(TauriCommand.QueueInsert, { instanceId, position, text, attachments })`.
  - Queue dispatch keybind (line ~461): `invoke(TauriCommand.QueueDispatch, { instanceId })`.
  - Queue-strip per-row send (line ~876): `invoke(TauriCommand.QueueDispatch, { instanceId, itemId })`.
  - Queue-strip per-row drop (line ~487): `invoke(TauriCommand.QueueRemove, { instanceId, itemId })`.
  - Queue-strip drop-all (line ~830): `invoke(TauriCommand.QueueClear, { instanceId })`.
  - On focus / instance flip: `useQueue().refresh()` to seed the store.

**Step 1: Tests** — adjust `views/composer/composer.test.ts` and add a tiny `views/Overlay.test.ts` shim if one doesn't already exist that mocks `@ipc`.

**Step 2-4: TDD.**

**Step 5: Commit**
```
feat(overlay): route queue actions through daemon RPC
```

---

### Task 13: Drop `pushToQueue` / `popQueueHead` / etc. from the UI

**Files:**
- Modify: `ui/src/composables/instance/use-queue.ts` — remove the local-store helpers (`pushToQueue`, `pushToQueueAt`, `popQueueHead`, `popQueueItem`, `removeFromQueue`, `flushQueue`, `dispatchQueueHead`, `dispatchQueueItem`, `startQueueDispatcher`, `stopQueueDispatcher`). Keep only the new `useQueue()` API + `applyQueueChanged` (for the wire router).
- Modify: `ui/src/composables/instance/cleanup.ts` — drop `resetQueue` import / call (the daemon owns the queue; the local mirror clears itself on the next `acp:queue-changed` event from the new instance focus).

**Step 1-4:** Each removal is mechanical — grep for usages, replace, verify the build is clean.

**Step 5: Commit**
```
chore(ui): drop legacy queue store helpers
```

---

### Task 14: Manual end-to-end smoke

- `task build && task lint && task test` exits 0.
- Open the overlay, enqueue 3 prompts while a turn is mid-flight, confirm the strip renders all three.
- `hyprpilot ctl rpc-raw '{"jsonrpc":"2.0","id":"1","method":"queue/list","params":{}}'` (or via `socat`) returns the same items.
- Open the SPA on a phone over the HTTPS bridge, enqueue from there, confirm the desktop overlay's strip updates within ~100ms.
- Cancel the in-flight turn — confirm the queue strip survives.
- Click "send now" on row 2 — confirm row 2 dispatches (not the head), row 1 + row 3 stay in order.
- `hyprpilot-nvim` follow-up: not in scope for THIS PR but the wire is ready; document in the PR body.

---

## Definition of done

- `task lint && task test && task build` green; all 543+ Rust tests + 483+ UI tests + new tests pass.
- The Vue UI no longer holds queue state locally — only a read-through mirror updated by daemon events.
- The composer's enqueue path, every QueueStrip button, and the queue keybind all route through `invoke()` (no direct local mutations).
- `queue/*` RPC verbs callable from `socat`-style raw clients + the WS remote bridge.
- `acp:queue-changed` event fans out to every connected client on every queue mutation.
- The cancel-doesn't-flush contract is preserved (existing test still green).
- The PR body documents the wire contract so the hyprpilot-nvim plugin can add a queue surface without re-asking.

---

## Open questions to resolve during review

1. Should `queue/dispatch { itemId: head_id }` and `queue/dispatch {}` produce the same result? (Pinned: yes — when omitted, default to the head's id.)
2. Reorder via `queue/move`: position index semantics — `position: 0` is the head, `position: items.len()` is the tail? (Pinned: yes, clamp to bounds, same as the current `pushToQueueAt`.)
3. Maximum queue size? (Pinned: no cap. The captain controls; a runaway script can always be stopped via `queue/clear`.)
4. Permissions for the queue actions across remote clients? (Out of scope: the daemon already gates RPC on the WS auth handshake; once authenticated, queue ops are free.)

---

## Risks + mitigations

- **Wire size on image attachments.** Image `data` is base64. A captain queueing several big screenshots will balloon `QueueChanged` payloads. Mitigation (out of this PR): consider a content-id mechanism that ships attachment bodies once and references them by id thereafter. Documented as a follow-up; not blocking.
- **Race between optimistic UI and daemon roundtrip.** Captain types + hits Enter while busy; `invoke(QueueEnqueue)` is async. The QueueStrip shouldn't show the row until the daemon's `QueueChanged` arrives. Mitigation: render off the daemon-state store only; brief delay is OK (typically <10ms over Tauri, <50ms over WS).
- **Reconnect drift.** On WS reconnect (already triggers a full page reload per PR #64), the daemon's queue survives; the SPA re-hydrates via `queue/list`. Verify in manual smoke.

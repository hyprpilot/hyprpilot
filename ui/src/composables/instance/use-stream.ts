import { computed, reactive, type ComputedRef } from 'vue'

import { nextSeq } from './sequence'
import { openTurnIdFor } from './use-turns'
import { useActiveInstance, type InstanceId } from '../chrome/use-active-instance'

export enum StreamItemKind {
  Thought = 'thought',
  Plan = 'plan',
  ModeChange = 'mode_change',
  ModelChange = 'model_change',
  ConfigOptionChange = 'config_option_change',
  SystemPromptInjected = 'system_prompt_injected'
}

interface BaseStream {
  id: string
  sessionId: string
  /// Active turn id at receive time; `undefined` for spontaneous
  /// updates outside a turn. Consumers group by this when rendering.
  turnId?: string
  createdAt: number
  updatedAt: number
}

export interface ThoughtStreamItem extends BaseStream {
  kind: StreamItemKind.Thought
  text: string
  /// Wall-clock at first observation. Pairs with the parent turn's
  /// `endedAtMs` (or `liveNow` while the turn is still in flight) for
  /// the thinking-card elapsed chip on agents that ship thoughts via
  /// `agent_thought_chunk` notifications (claude-agent-acp).
  startedAtMs: number
}

export interface PlanEntry {
  content?: string
  status?: string
  priority?: string
}

/// Generic running stat for a checklist-shaped item. Mirrors the
/// daemon's `ChecklistStats` (`adapters::transcript::ChecklistStats`).
export interface ChecklistStats {
  done: number
  total: number
}

export interface PlanStreamItem extends BaseStream {
  kind: StreamItemKind.Plan
  entries: PlanEntry[]
  /// Latest `done / total` snapshot from the daemon. Plans re-emit
  /// fully on every agent update; we replace the prior stat in
  /// place. `undefined` only when the daemon ships a plan without
  /// stats (defensive — daemon always computes them).
  stats?: ChecklistStats
}

/// Banner chip rendered between turns whenever the agent emits a
/// `current_mode_update` (claude-code switching from `plan` →
/// `default` after the user accepts the exit-plan permission, etc.).
/// `name` is the human label from `availableModes` when known;
/// `modeId` falls through when we don't have a name. `prevName` /
/// `prevModeId` are the values BEFORE this transition so the banner
/// can read `mode · plan → default` instead of just `mode → default`.
export interface ModeChangeStreamItem extends BaseStream {
  kind: StreamItemKind.ModeChange
  modeId: string
  name?: string
  prevModeId?: string
  prevName?: string
}

/// Banner chip for model switches — same chrome as `ModeChangeStreamItem`,
/// keyed off the model id instead. Fires on the captain's palette
/// commit; mirrors the agent's `current_model_update` notification
/// path so user-initiated and agent-initiated changes both leave a
/// chapter-break in the transcript.
export interface ModelChangeStreamItem extends BaseStream {
  kind: StreamItemKind.ModelChange
  modelId: string
  name?: string
  prevModelId?: string
  prevName?: string
}

/// Banner chip for vendor-extension config-option flips (claude-agent-acp's
/// `effort` is the canonical first user). Same chrome as ModeChange /
/// ModelChange — chapter break in the transcript so the captain can see
/// "X changed from A to B" inline. `categoryId` carries the wire id
/// (`effort`) and doubles as the lead-in noun on the banner; `name` is
/// the picked option's display label resolved against `category.options`.
export interface ConfigOptionChangeStreamItem extends BaseStream {
  kind: StreamItemKind.ConfigOptionChange
  categoryId: string
  value: string
  name?: string
  prevValue?: string
  prevName?: string
}

/// Fires once per instance start when the daemon actually attaches a
/// system prompt to the spawning agent. Sessions with no configured
/// prompt files (or every entry's inject toggle off for this
/// bootstrap path) get nothing — silence by design.
export interface SystemPromptInjectedStreamItem extends BaseStream {
  kind: StreamItemKind.SystemPromptInjected
  files: string[]
}

export type StreamItem = ThoughtStreamItem | PlanStreamItem | ModeChangeStreamItem | ModelChangeStreamItem | ConfigOptionChangeStreamItem | SystemPromptInjectedStreamItem

export interface StreamState {
  items: StreamItem[]
  /// Per-session id of the agent's open thought item for the current
  /// turn. Cleared on `user_message_chunk` (the next turn starting);
  /// every `agent_thought_chunk` in between appends to the same item.
  openThoughtBySession: Map<string, string>
  /// Per-session id of the open plan item for the current turn. Plans
  /// arrive as full snapshots, so subsequent updates replace `entries`
  /// in place rather than appending — but stay anchored to the same
  /// item id (same `createdAt`) until the turn closes.
  openPlanBySession: Map<string, string>
}

const states = reactive(new Map<InstanceId, StreamState>())

function slotFor(id: InstanceId): StreamState {
  let slot = states.get(id)

  if (!slot) {
    slot = {
      items: [],
      openThoughtBySession: new Map(),
      openPlanBySession: new Map()
    }
    states.set(id, slot)
  }

  return slot
}

/// Close the per-session turn — clears both thought and plan trackers.
/// Called from the demuxer when a `user_message_chunk` arrives, signalling
/// the previous agent turn is done and the next thought / plan should
/// open a fresh item.
export function closeTurn(id: InstanceId, sessionId: string): void {
  const slot = states.get(id)

  if (!slot) {
    return
  }
  slot.openThoughtBySession.delete(sessionId)
  slot.openPlanBySession.delete(sessionId)
}

interface ThoughtUpdate {
  sessionUpdate: string
  content?: { text?: string }
  messageId?: string
}

interface PlanUpdate {
  sessionUpdate: string
  entries?: PlanEntry[]
  stats?: ChecklistStats
}

// ── Internal store-mutation surface ───────────────────────────────
// Sibling-store wire-listener inputs. CLAUDE.md "Two-tier composables".

export function pushThoughtChunk(id: InstanceId, sessionId: string, raw: ThoughtUpdate): void {
  const slot = slotFor(id)
  const seq = nextSeq(id)
  const text = typeof raw.content?.text === 'string' ? raw.content.text : ''
  const hasExplicitId = typeof raw.messageId === 'string'
  const explicitId = hasExplicitId ? (raw.messageId as string) : undefined
  const openId = explicitId ?? slot.openThoughtBySession.get(sessionId)

  if (openId) {
    const target = slot.items.find((it): it is ThoughtStreamItem => it.kind === StreamItemKind.Thought && it.sessionId === sessionId && it.id === openId)

    if (target) {
      target.text += text
      target.updatedAt = seq

      return
    }
  }

  const itemId = explicitId ?? `thought-${sessionId}-${slot.items.length}`

  slot.items.push({
    kind: StreamItemKind.Thought,
    id: itemId,
    sessionId,
    turnId: openTurnIdFor(id, sessionId),
    createdAt: seq,
    updatedAt: seq,
    text,
    startedAtMs: Date.now()
  })
  slot.openThoughtBySession.set(sessionId, itemId)
}

export function pushPlan(id: InstanceId, sessionId: string, raw: PlanUpdate): void {
  const slot = slotFor(id)
  const seq = nextSeq(id)
  const entries = Array.isArray(raw.entries) ? raw.entries : []
  const stats = raw.stats
  const openId = slot.openPlanBySession.get(sessionId)

  if (openId) {
    const target = slot.items.find((it): it is PlanStreamItem => it.kind === StreamItemKind.Plan && it.sessionId === sessionId && it.id === openId)

    if (target) {
      target.entries = entries
      target.stats = stats
      target.updatedAt = seq

      return
    }
  }

  const itemId = `plan-${sessionId}-${slot.items.length}`

  slot.items.push({
    kind: StreamItemKind.Plan,
    id: itemId,
    sessionId,
    turnId: openTurnIdFor(id, sessionId),
    createdAt: seq,
    updatedAt: seq,
    entries,
    stats
  })
  slot.openPlanBySession.set(sessionId, itemId)
}

export interface ModeChangePush {
  modeId: string
  name?: string
  prevModeId?: string
  prevName?: string
}

/// Banner-only push: emits a fresh `ModeChangeStreamItem` for the
/// active turn (or no turn — solo block). De-dupes against the most
/// recent mode change so a noisy double-update from the agent doesn't
/// stack two identical banners.
export function pushModeChange(id: InstanceId, sessionId: string, change: ModeChangePush): void {
  const slot = slotFor(id)
  const seq = nextSeq(id)
  const last = slot.items[slot.items.length - 1]

  if (last && last.kind === StreamItemKind.ModeChange && last.sessionId === sessionId && last.modeId === change.modeId) {
    last.updatedAt = seq

    return
  }
  const itemId = `mode-${sessionId}-${slot.items.length}`

  slot.items.push({
    kind: StreamItemKind.ModeChange,
    id: itemId,
    sessionId,
    turnId: openTurnIdFor(id, sessionId),
    createdAt: seq,
    updatedAt: seq,
    modeId: change.modeId,
    name: change.name,
    prevModeId: change.prevModeId,
    prevName: change.prevName
  })
}

export interface ModelChangePush {
  modelId: string
  name?: string
  prevModelId?: string
  prevName?: string
}

/// Mirror of `pushModeChange` for model switches. Same dedupe rule:
/// re-pushing the same model id against the most-recent banner just
/// touches `updatedAt`. Drives the captain-initiated banner from the
/// models palette commit; an agent-side `current_model_update` echo
/// would dedupe against the same id without stacking a second card.
export function pushModelChange(id: InstanceId, sessionId: string, change: ModelChangePush): void {
  const slot = slotFor(id)
  const seq = nextSeq(id)
  const last = slot.items[slot.items.length - 1]

  if (last && last.kind === StreamItemKind.ModelChange && last.sessionId === sessionId && last.modelId === change.modelId) {
    last.updatedAt = seq

    return
  }
  const itemId = `model-${sessionId}-${slot.items.length}`

  slot.items.push({
    kind: StreamItemKind.ModelChange,
    id: itemId,
    sessionId,
    turnId: openTurnIdFor(id, sessionId),
    createdAt: seq,
    updatedAt: seq,
    modelId: change.modelId,
    name: change.name,
    prevModelId: change.prevModelId,
    prevName: change.prevName
  })
}

export interface ConfigOptionChangePush {
  categoryId: string
  value: string
  name?: string
  prevValue?: string
  prevName?: string
}

/// Mirror of `pushModeChange` for vendor-extension config option flips.
/// Dedupes on `(categoryId, value)` against the most-recent banner so an
/// agent-side `config_option_update` echo following the captain's commit
/// doesn't stack a second card.
export function pushConfigOptionChange(id: InstanceId, sessionId: string, change: ConfigOptionChangePush): void {
  const slot = slotFor(id)
  const seq = nextSeq(id)
  const last = slot.items[slot.items.length - 1]

  if (last && last.kind === StreamItemKind.ConfigOptionChange && last.sessionId === sessionId && last.categoryId === change.categoryId && last.value === change.value) {
    last.updatedAt = seq

    return
  }
  const itemId = `cfg-${change.categoryId}-${sessionId}-${slot.items.length}`

  slot.items.push({
    kind: StreamItemKind.ConfigOptionChange,
    id: itemId,
    sessionId,
    turnId: openTurnIdFor(id, sessionId),
    createdAt: seq,
    updatedAt: seq,
    categoryId: change.categoryId,
    value: change.value,
    name: change.name,
    prevValue: change.prevValue,
    prevName: change.prevName
  })
}

export interface SystemPromptInjectedPush {
  files: string[]
}

/// Banner item for the daemon's `acp:system-prompt-injected` event.
/// Sessions don't always carry a sessionId yet at injection time
/// (the event fires before `session/new` resolves on Bootstrap::Fresh
/// and during the LoadSession dance on Bootstrap::Resume), so we
/// stamp `sessionId: ''` and let the demuxer place it before the
/// first turn.
export function pushSystemPromptInjected(id: InstanceId, push: SystemPromptInjectedPush): void {
  const slot = slotFor(id)
  const seq = nextSeq(id)
  // Dedupe: a re-emitted event for the same instance + same files
  // refreshes the timestamp instead of stacking a second banner.
  const last = slot.items[slot.items.length - 1]

  if (last && last.kind === StreamItemKind.SystemPromptInjected && last.files.length === push.files.length && last.files.every((f, i) => f === push.files[i])) {
    last.updatedAt = seq

    return
  }
  const itemId = `system-prompt-${id}-${slot.items.length}`

  slot.items.push({
    kind: StreamItemKind.SystemPromptInjected,
    id: itemId,
    sessionId: '',
    turnId: undefined,
    createdAt: seq,
    updatedAt: seq,
    files: [...push.files]
  })
}

export function resetStream(id: InstanceId): void {
  states.delete(id)
}

/** Drop every stream item (thought / plan chunk) tagged with
 * `turnId`. Paired with `deleteTurnByTurnId` to fully remove a
 * cancelled / errored turn from the visible chat. */
export function deleteStreamByTurnId(id: InstanceId, turnId: string): number {
  const slot = states.get(id)

  if (!slot) {
    return 0
  }
  const before = slot.items.length

  slot.items = slot.items.filter((item) => item.turnId !== turnId)

  return before - slot.items.length
}

export function useStream(instanceId?: InstanceId): { items: ComputedRef<StreamItem[]> } {
  const { id: activeId } = useActiveInstance()
  const items = computed<StreamItem[]>(() => {
    const resolved = instanceId ?? activeId.value

    if (!resolved) {
      return []
    }

    return states.get(resolved)?.items ?? []
  })

  return { items }
}

/**
 * Wire-contract shapes for the daemon's per-instance state mirror —
 * the `instance/snapshot/{meta,chat,terminals}` Tauri commands and
 * matching JSON-RPC methods. Mirrors the Rust types in
 * `src-tauri/src/adapters/mirror.rs` exactly; the Rust side carries
 * `#[serde(rename_all = "camelCase")]` so every field is camelCase
 * across the wire. Optional fields use `?` per CLAUDE.md (`Option<T>`
 * → `null` becomes the consumer-edge `field?: T` shape).
 *
 * Three discrete shapes match the three RPC methods so a remote that
 * only needs the header pills doesn't drag the entire transcript over
 * the wire:
 *
 * - `MetaSnapshot` — header / chrome view (mode, model, advertised
 *   modes/models, cwd, mcps_count, profile id, current turn marker,
 *   pending permissions, usage tally, latest_seq cursor). Cheap.
 * - `ChatSnapshot` — windowed transcript page anchored at the latest
 *   turn (`before = undefined`) or strictly older than a `seq` cursor
 *   (`before = seq`). Backward-paginates on captain scroll.
 * - `TerminalsSnapshot` — full per-terminalId map. Small enough to
 *   ship whole today; revisit when sessions accumulate dozens.
 */

import type { SessionConfigOptionCategory } from './event'
import type { PermissionOptionView, TranscriptItem } from './transcript'

/** Mirrors Rust `acp::instance::SessionModeInfo`. */
export interface SessionModeInfo {
  id: string
  name: string
  description?: string
}

/** Mirrors Rust `acp::instance::SessionModelInfo`. */
export interface SessionModelInfo {
  id: string
  name: string
  description?: string
}

/**
 * Mirrors Rust `mirror::TurnEventMarker` — tagged enum. UI's phase
 * derivation reads it without re-walking the transcript.
 */
export type TurnEventMarker = { kind: 'started'; startedAt: number } | { kind: 'ended'; endedAt: number }

/** Mirrors Rust `acp::instance::UsageCost`. */
export interface UsageCost {
  amount: number
  currency: string
}

/** Mirrors Rust `mirror::UsageSnapshot`. Reset on `TurnStarted`. */
export interface UsageSnapshot {
  used: number
  size: number
  cost?: UsageCost
  /** Active turn id; absent between turns. */
  turnId?: string
}

/**
 * Mirrors Rust `permission::PermissionRequestSnapshot` — same shape
 * `permissions/pending` returns inline. Carried as part of
 * `MetaSnapshot.pendingPermissions`.
 */
export interface PermissionRequestSnapshot {
  requestId: string
  instanceId?: string
  tool: string
  args?: string
  options: PermissionOptionView[]
}

/**
 * One transcript entry with its monotonic sequence number. The daemon
 * stamps `seq` at insertion time; UI uses the oldest entry's `seq` as
 * the next-page cursor when scrolling backward.
 */
export interface SeqTranscriptItem {
  seq: number
  item: TranscriptItem
}

/**
 * Mirrors Rust `mirror::TerminalSnapshot`. `running` flips `false` when
 * an `Exit` chunk lands. Empty `stdout` / `stderr` strings are skipped
 * on the wire (`#[serde(skip_serializing_if = "String::is_empty")]`)
 * — consumers default to `""` when absent.
 */
export interface TerminalSnapshot {
  stdout?: string
  stderr?: string
  running: boolean
  exitCode?: number
  signal?: string
}

/**
 * Mirrors Rust `mirror::MetaSnapshot`. Cheap to fetch on focus-switch
 * and on `acp:instances-changed`.
 */
export interface MetaSnapshot {
  profileId?: string
  sessionId?: string
  cwd?: string
  currentModeId?: string
  currentModelId?: string
  availableModes?: SessionModeInfo[]
  availableModels?: SessionModelInfo[]
  configOptions?: SessionConfigOptionCategory[]
  mcpsCount: number
  currentTurnEvent?: TurnEventMarker
  pendingPermissions?: PermissionRequestSnapshot[]
  usage: UsageSnapshot
  /**
   * Latest `SeqTranscriptItem.seq` in the mirror; `undefined` when the
   * transcript is empty. UI seeds the chat infinite-query off this so
   * the first page request anchors at the right cursor.
   */
  latestSeq?: number
}

/**
 * Mirrors Rust `mirror::ChatSnapshot`. Windowed transcript page;
 * `oldestSeq` / `latestSeq` absent when `items` is empty.
 *
 * Pagination cursor: pass `before = oldestSeq` of the previous page
 * (or `undefined` for the latest page) to fetch the next-older page.
 * `hasMore` is `true` iff entries strictly older than the returned
 * window still exist in the buffer.
 */
export interface ChatSnapshot {
  items: SeqTranscriptItem[]
  oldestSeq?: number
  latestSeq?: number
  hasMore: boolean
}

/**
 * Mirrors Rust `mirror::TerminalsSnapshot`. Full per-`terminalId` map;
 * small enough to ship whole today.
 */
export interface TerminalsSnapshot {
  terminals: Record<string, TerminalSnapshot>
}

/**
 * Args for the `instance_snapshot_meta` Tauri command (and the
 * `instance/snapshot/meta` RPC method). `instanceId` is required —
 * the daemon does not auto-resolve to focused instance.
 */
export interface InstanceSnapshotMetaArgs {
  instanceId: string
}

/**
 * Args for `instance_snapshot_chat`. `before` strictly-older cursor
 * for backward pagination; `limit` defaults to the daemon's page
 * size (50) when unset or `0`.
 */
export interface InstanceSnapshotChatArgs {
  instanceId: string
  before?: number
  limit?: number
}

/** Args for `instance_snapshot_terminals`. */
export interface InstanceSnapshotTerminalsArgs {
  instanceId: string
}

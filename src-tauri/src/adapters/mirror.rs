//! Per-instance state mirror — write-through cache the actor pushes
//! every emitted [`InstanceEvent`] into so out-of-actor consumers
//! (snapshot RPC handlers, mid-session remote brim-sync) can read the
//! merged state without round-tripping through the command channel.
//!
//! The mirror is **the daemon's truth** for in-flight per-instance
//! state — everything that today flows through the registry's
//! `broadcast` channel and is otherwise UI-accumulated. Three snapshot
//! flavours ride on top:
//!
//! - [`MetaSnapshot`] — small / cheap. Header pills, current phase
//!   marker, pending permissions; pulled on focus-switch and on
//!   `acp:instances-changed`.
//! - [`ChatSnapshot`] — windowed transcript backed by a bounded ring
//!   buffer. Default cap: [`DEFAULT_TRANSCRIPT_CAP`] entries; older
//!   entries silently fall off the front. Pagination by internal
//!   monotonic [`SeqTranscriptItem::seq`] cursor (`before: Option<u64>`).
//! - [`TerminalsSnapshot`] — full per-`terminal_id` map; small enough
//!   to ship whole today.
//!
//! ## Why not a broadcast subscriber?
//!
//! "Couldn't the mirror just be another `events_rx.recv()` consumer of
//! the same broadcast everyone else reads?" comes up regularly and
//! deserves an inline answer.
//!
//! **The mirror is authoritative state; the broadcast is a lossy
//! transport.** `tokio::sync::broadcast` is capacity-256 and silently
//! drops on `RecvError::Lagged` — that's correct for UI fan-out (the
//! next tick re-paints from a snapshot pull), but **catastrophic for
//! a write-through cache**. A single lag during a streaming reply
//! (~30 transcript chunks/sec is typical) would punch a hole in the
//! mirror's `seq` cursor, and the UI's pagination would skip the
//! gap silently — no panic, no warn, just permanently corrupt
//! state. There's no unbounded `broadcast` in tokio, and `mpsc`
//! doesn't fan out, so a single-channel design that holds the
//! mirror-coherent invariant doesn't exist.
//!
//! The fix is structural: the **actor itself** is the single writer,
//! and **applies to the mirror BEFORE broadcasting**. The
//! [`publish`] helper in this module enforces the ordering at every
//! call site so the mirror physically cannot lag its producer. Any
//! out-of-actor reader (snapshot RPC, mid-session remote brim-sync)
//! that queries the mirror after an emit returned is guaranteed to
//! see a state that includes the emitted event.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use super::instance::{
    InstanceEvent, SessionConfigOptionCategory, SessionModeInfo, SessionModelInfo, TerminalChunk, TerminalStream,
};
use super::permission::PermissionRequestSnapshot;
use super::transcript::TranscriptItem;

/// Bounded ring-buffer ceiling for the transcript. Older entries
/// silently fall off the front when the cap is exceeded. The plan
/// sets this at "sane upper bound (e.g. 5000)" — comfortably above
/// any real session length while still capping daemon memory growth.
/// User-visible truncation is **not** part of the contract: the
/// captain never sees a "truncated" indicator, and the UI's windowed
/// query asks only for visible pages anyway.
pub const DEFAULT_TRANSCRIPT_CAP: usize = 5_000;

/// Default page size when the snapshot RPC caller passes `0`. The
/// daemon does NOT clamp caller-supplied limits — frontends own the
/// viewport sizing axis (a 4K monitor wants different pages than a
/// phone) and the mirror's own ring-buffer cap [`DEFAULT_TRANSCRIPT_CAP`]
/// is the natural ceiling. Trust the frontend; the backend serves
/// what it asks for.
const DEFAULT_CHAT_LIMIT: usize = 50;

/// Marker for the most-recent turn boundary the mirror has seen.
/// UI's phase derivation reads it without re-walking the transcript.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum TurnEventMarker {
    Started { started_at: u64 },
    Ended { ended_at: u64 },
}

/// One transcript entry plus its monotonic sequence number. The
/// daemon stamps `seq` at insertion time so the windowed
/// [`InstanceMirror::chat_snapshot`] knows its pagination cursor
/// without re-deriving it from wall-clock or array index.
///
/// `turn_id` is the active ACP turn id stamped onto the
/// `InstanceEvent::Transcript` that produced this entry. The
/// snapshot-driven block projector groups consecutive items by
/// `turn_id` (when present) so the chat body lays out the same
/// per-turn blocks the live router builds in
/// `useTimelineBlocks`. Items emitted outside a turn (synthetic /
/// pre-turn agent activity) carry `None`; those flow into role-run
/// grouping in the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeqTranscriptItem {
    pub seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    pub item: TranscriptItem,
}

/// Per-`terminal_id` accumulated state. Output chunks concatenate;
/// `Exit` lands once and flips `running = false`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSnapshot {
    /// Concatenated stdout chunks in arrival order.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub stdout: String,
    /// Concatenated stderr chunks in arrival order.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub stderr: String,
    /// `true` until an `Exit` chunk lands.
    pub running: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<String>,
}

/// Per-turn token tally — context budget + (optional) cost.
/// Reset on `TurnStarted`; updated on every `UsageUpdate` while a
/// turn is live.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSnapshot {
    pub used: u64,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<crate::adapters::acp::instance::UsageCost>,
    /// Turn id this tally belongs to; `None` between turns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
}

/// One per-turn record the snapshot ships so the UI's `useTurns`
/// store can hydrate without replaying the live event stream. Mirrors
/// the UI-side `TurnRecord` field set sufficient for the chat header's
/// elapsed + usage chips and stick-to-bottom phase derivation.
///
/// Built by accumulating `InstanceEvent::{TurnStarted, TurnEnded,
/// UsageUpdate}`. Items are ordered by emission (oldest-first) — the
/// UI reads them in arrival order.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnSnapshot {
    pub id: String,
    pub session_id: String,
    /// Wall-clock (epoch ms) when the actor accepted the prompt.
    pub started_at_ms: u64,
    /// Wall-clock (epoch ms) when the prompt resolved. `None` while
    /// the turn is mid-flight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at_ms: Option<u64>,
    /// ACP `StopReason` wire string when the turn ended cleanly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    /// ACP / transport error message when the turn errored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Latest `UsageUpdate` reading bound to this turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<UsageSnapshot>,
}

/// Inner mutable state. Wrapped in [`InstanceMirror`]'s
/// `Arc<RwLock<…>>` so the actor's write-through pairs with concurrent
/// snapshot reads.
#[derive(Debug, Default)]
pub struct MirrorInner {
    /// Bounded ring buffer. Front drops on cap overflow.
    pub transcript: VecDeque<SeqTranscriptItem>,
    /// Monotonic stamp counter. Always strictly greater than every
    /// surviving entry's `seq` — a `transcript.is_empty()` mirror
    /// still issues `next_seq` correctly because pagination cursors
    /// only ever read the queue's existing ids.
    pub next_seq: u64,
    /// Per-terminal accumulated state.
    pub terminals: HashMap<String, TerminalSnapshot>,
    /// Open permission rows. Cleared on resolve via the
    /// `PermissionResolved` event arriving in Phase A4.
    pub pending_permissions: Vec<PermissionRequestSnapshot>,
    /// Denormalized meta cache.
    pub meta: MirrorMetaCache,
    /// Most-recent turn boundary marker.
    pub last_turn_event: Option<TurnEventMarker>,
    /// Latest usage tally.
    pub usage: UsageSnapshot,
    /// Per-turn records. Oldest-first; bounded by the same eviction
    /// the transcript ring buffer applies — when the buffer drops a
    /// transcript item carrying a `turn_id` and that turn is no
    /// longer represented in the surviving transcript, its record is
    /// dropped too. Today the bound is implicit: `pushTurnStarted`
    /// appends, `pushTurnEnded` patches in place, `UsageUpdate`
    /// patches in place. The list grows linearly with turn count
    /// — at typical session lengths this is bounded enough not to
    /// warrant explicit eviction yet.
    pub turns: Vec<TurnSnapshot>,
    /// Cached snapshot of the per-instance submit queue. Replaced
    /// wholesale on every `QueueChanged` event; snapshot reads serve
    /// from this without an actor round-trip. Empty by default.
    pub queue: Vec<crate::adapters::queue::QueueItem>,
}

/// Mutable backing for [`MetaSnapshot`]. Mirrors the existing
/// `acp::instance::MetaSnapshot` field set + a few additions
/// (`profile_id`, the `current_turn_event` marker, the
/// `pending_permissions` count, the `latest_seq` cursor) the
/// snapshot RPC adds.
///
#[derive(Debug, Default, Clone)]
pub struct MirrorMetaCache {
    pub profile_id: Option<String>,
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    pub current_mode_id: Option<String>,
    pub current_model_id: Option<String>,
    pub available_modes: Vec<SessionModeInfo>,
    pub available_models: Vec<SessionModelInfo>,
    pub mcps_count: usize,
    /// Adapter-advertised category list. Latest [`ConfigOptionsUpdate`]
    /// wins — palette reads it via [`MetaSnapshot::config_options`].
    pub config_options: Vec<SessionConfigOptionCategory>,
}

/// Write-through state cache for one [`super::acp::instance::AcpInstance`].
///
/// One mirror per instance, owned alongside the actor's command
/// channel + the running tool-call cache. Cloning the [`Arc`] is
/// cheap; readers (snapshot handlers) hold a read lock for the
/// duration of a `*_snapshot()` call, the actor takes a write lock
/// per [`apply`](Self::apply) — same shape as the existing
/// `tool_calls: Arc<RwLock<ToolCallCache>>` plumbing.
///
/// Phase A2 lands the type + the wire-through plumbing on
/// [`crate::adapters::acp::instance::AcpInstance`]. Phase A3 calls
/// [`apply`](Self::apply) from the actor's emit lines; Phase A5
/// adds the snapshot RPC handlers.
#[derive(Debug, Default, Clone)]
pub struct InstanceMirror {
    inner: Arc<RwLock<MirrorInner>>,
    cap: usize,
}

impl InstanceMirror {
    /// Build a mirror with the default transcript cap.
    #[must_use]
    pub fn new() -> Self {
        Self::with_cap(DEFAULT_TRANSCRIPT_CAP)
    }

    /// Build a mirror with an explicit cap. Tests use this to drive
    /// ring-buffer eviction without inserting `DEFAULT_TRANSCRIPT_CAP`
    /// items; production callers should use [`Self::new`].
    #[must_use]
    pub fn with_cap(cap: usize) -> Self {
        Self {
            inner: Arc::new(RwLock::new(MirrorInner::default())),
            cap,
        }
    }

    /// Write-through entry point. Match exhaustive on every
    /// [`InstanceEvent`] variant — the compiler enforces coverage so
    /// adding a new event variant either reaches the mirror or carries
    /// an explicit no-op arm with a comment justifying it.
    ///
    /// Phase A3 wires this alongside every existing `events_tx.send(…)`
    /// in `acp/instance.rs`. Phase A2 lands the type only; nothing
    /// calls `apply` yet.
    /// Returns the seq value minted for `Transcript` events (so
    /// [`publish`] can stamp it onto the broadcast event before
    /// sending). `None` for every other variant — non-transcript
    /// events don't carry seq today.
    pub async fn apply(&self, event: &InstanceEvent) -> Option<u64> {
        // Same split as `acp::emit` — chunk events (transcript /
        // terminal) get their own sub-target so a captain debugging
        // lifecycle / usage doesn't drown in chunk spam at trace
        // level. Opt into chunks via `snapshot::mirror::chunk=trace`.
        if matches!(event, InstanceEvent::Transcript { .. } | InstanceEvent::Terminal { .. }) {
            tracing::trace!(
                target: "snapshot::mirror::chunk",
                event = event.topic(),
                "mirror.apply (chunk)",
            );
        } else {
            tracing::trace!(
                target: "snapshot::mirror",
                event = event.topic(),
                "mirror.apply",
            );
        }
        let mut g = self.inner.write().await;
        let mut minted_seq: Option<u64> = None;
        match event {
            // ── transcript firehose ──────────────────────────────
            InstanceEvent::Transcript { item, turn_id, .. } => {
                let seq = g.next_seq;
                g.next_seq = seq.saturating_add(1);
                g.transcript.push_back(SeqTranscriptItem {
                    seq,
                    turn_id: turn_id.clone(),
                    item: item.clone(),
                });
                while g.transcript.len() > self.cap {
                    g.transcript.pop_front();
                }
                minted_seq = Some(seq);
            }

            // ── per-turn lifecycle markers ───────────────────────
            InstanceEvent::TurnStarted {
                turn_id,
                session_id,
                started_at,
                ..
            } => {
                g.last_turn_event = Some(TurnEventMarker::Started {
                    started_at: *started_at,
                });
                // New turn → fresh usage tally; vendors that emit
                // `UsageUpdate` deltas mid-turn fill it back in.
                g.usage = UsageSnapshot {
                    used: 0,
                    size: 0,
                    cost: None,
                    turn_id: Some(turn_id.clone()),
                };
                // Record per-turn entry. Idempotent on duplicate
                // turn_id (replay safety).
                if !g.turns.iter().any(|t| t.id == *turn_id) {
                    g.turns.push(TurnSnapshot {
                        id: turn_id.clone(),
                        session_id: session_id.clone(),
                        started_at_ms: *started_at,
                        ended_at_ms: None,
                        stop_reason: None,
                        error: None,
                        usage: None,
                    });
                }
            }
            InstanceEvent::TurnEnded {
                turn_id,
                ended_at,
                stop_reason,
                error,
                ..
            } => {
                g.last_turn_event = Some(TurnEventMarker::Ended { ended_at: *ended_at });
                if let Some(rec) = g.turns.iter_mut().find(|t| t.id == *turn_id) {
                    rec.ended_at_ms = Some(*ended_at);
                    rec.stop_reason.clone_from(stop_reason);
                    rec.error.clone_from(error);
                }
            }

            // ── permission roundtrip ─────────────────────────────
            InstanceEvent::PermissionRequest {
                request_id,
                instance_id,
                tool,
                args,
                options,
                default_option_id,
                ..
            } => {
                // Idempotent: same `request_id` arriving twice would
                // be a wire-protocol bug, but we still tolerate it.
                if !g.pending_permissions.iter().any(|p| p.request_id == *request_id) {
                    g.pending_permissions.push(PermissionRequestSnapshot {
                        request_id: request_id.clone(),
                        instance_id: Some(instance_id.clone()),
                        tool: tool.clone(),
                        args: Some(args.clone()),
                        options: options.clone(),
                        default_option_id: default_option_id.clone(),
                    });
                }
            }

            // ── meta refresh ─────────────────────────────────────
            InstanceEvent::InstanceMeta {
                profile_id,
                session_id,
                cwd,
                current_mode_id,
                current_model_id,
                available_modes,
                available_models,
                mcps_count,
                ..
            } => {
                g.meta.profile_id.clone_from(profile_id);
                g.meta.session_id.clone_from(session_id);
                g.meta.cwd = Some(cwd.clone());
                g.meta.current_mode_id.clone_from(current_mode_id);
                g.meta.current_model_id.clone_from(current_model_id);
                g.meta.available_modes.clone_from(available_modes);
                g.meta.available_models.clone_from(available_models);
                g.meta.mcps_count = *mcps_count;
            }
            InstanceEvent::CurrentModeUpdate { current_mode_id, .. } => {
                g.meta.current_mode_id = Some(current_mode_id.clone());
            }
            InstanceEvent::ConfigOptionsUpdate { categories, .. } => {
                g.meta.config_options.clone_from(categories);
            }
            InstanceEvent::SessionInfoUpdate { .. } => {
                // No-op: title / updatedAt are session-storage
                // metadata, not part of the in-flight mirror today.
                // Phase A5's `instance/snapshot/meta` doesn't surface
                // them; if it ever does, fold them into
                // `MirrorMetaCache` here.
            }

            // ── usage tally ──────────────────────────────────────
            InstanceEvent::UsageUpdate {
                turn_id,
                used,
                size,
                cost,
                ..
            } => {
                g.usage = UsageSnapshot {
                    used: *used,
                    size: *size,
                    cost: cost.clone(),
                    turn_id: turn_id.clone(),
                };
                // Bind the latest reading to the matching turn record
                // so a snapshot replay carries the captain's last
                // visible usage chip per turn. Falls back to the
                // most-recent turn when `turn_id` is absent
                // (between-turn updates).
                let target_id = turn_id.clone().or_else(|| g.turns.last().map(|t| t.id.clone()));
                if let Some(id) = target_id {
                    if let Some(rec) = g.turns.iter_mut().find(|t| t.id == id) {
                        rec.usage = Some(UsageSnapshot {
                            used: *used,
                            size: *size,
                            cost: cost.clone(),
                            turn_id: turn_id.clone(),
                        });
                    }
                }
            }

            // ── terminal accumulation ───────────────────────────
            InstanceEvent::Terminal { terminal_id, chunk, .. } => {
                let entry = g
                    .terminals
                    .entry(terminal_id.clone())
                    .or_insert_with(|| TerminalSnapshot {
                        running: true,
                        ..TerminalSnapshot::default()
                    });
                match chunk {
                    TerminalChunk::Output { stream, data } => match stream {
                        TerminalStream::Stdout => entry.stdout.push_str(data),
                        TerminalStream::Stderr => entry.stderr.push_str(data),
                    },
                    TerminalChunk::Exit { exit_code, signal } => {
                        entry.running = false;
                        entry.exit_code = *exit_code;
                        entry.signal.clone_from(signal);
                    }
                }
            }

            // Permission roundtrip closed (captain-answered or
            // 10-min `WAITER_TIMEOUT` expiry). Drop the matching row
            // so the mirror's `pending_permissions` snapshot stays in
            // sync regardless of which transport (desktop / remote)
            // delivered the answer. Idempotent: missing entry → noop.
            InstanceEvent::PermissionResolved { request_id, .. } => {
                g.pending_permissions.retain(|p| p.request_id != *request_id);
            }

            // ── lifecycle / no-ops (registry-shape, not mirror-shape) ─
            //
            // Each arm below is a deliberate no-op:
            //
            // * `State` — lifecycle transitions live on the registry's
            //   `InstanceInfo`; the mirror is per-instance state, not
            //   the registry view.
            // * `InstancesChanged` / `InstancesFocused` — registry
            //   membership / focus pointer; sibling concerns to the
            //   mirror.
            // * `InstanceRenamed` — name lives on the per-instance
            //   `RwLock<Option<String>>`; not part of the snapshot
            //   field set today.
            // * `DaemonReloaded` — daemon-global, not per-instance.
            // * `SystemPromptInjected` — banner-only event; the file
            //   list isn't part of the snapshot wire shape today.
            // ── queue ────────────────────────────────────────────
            //
            // Full-state replace, idempotent on lossy broadcast: a
            // re-delivered event with the same items is a no-op (Vec
            // equality), and a stale event landing after a newer one
            // is a hold-down (the next live mutation re-broadcasts
            // the canonical state). Frontends key off `enqueued_seq`
            // for ordering; the mirror just preserves wire order.
            InstanceEvent::QueueChanged { items, .. } => {
                g.queue = items.clone();
            }

            InstanceEvent::State { .. }
            | InstanceEvent::InstancesChanged { .. }
            | InstanceEvent::InstancesFocused { .. }
            | InstanceEvent::InstanceRenamed { .. }
            | InstanceEvent::DaemonReloaded { .. }
            | InstanceEvent::SelectedProfileChanged { .. }
            | InstanceEvent::SystemPromptInjected { .. } => {}
        }
        minted_seq
    }

    /// Read a [`MetaSnapshot`] off the cache.
    pub async fn meta_snapshot(&self) -> MetaSnapshot {
        let g = self.inner.read().await;
        let latest_seq = g.transcript.back().map(|e| e.seq);
        MetaSnapshot {
            profile_id: g.meta.profile_id.clone(),
            session_id: g.meta.session_id.clone(),
            cwd: g.meta.cwd.clone(),
            current_mode_id: g.meta.current_mode_id.clone(),
            current_model_id: g.meta.current_model_id.clone(),
            available_modes: g.meta.available_modes.clone(),
            available_models: g.meta.available_models.clone(),
            config_options: g.meta.config_options.clone(),
            mcps_count: g.meta.mcps_count,
            current_turn_event: g.last_turn_event,
            pending_permissions: g.pending_permissions.clone(),
            queue: g.queue.clone(),
            usage: g.usage.clone(),
            turns: g.turns.clone(),
            latest_seq,
        }
    }

    /// Read a windowed [`ChatSnapshot`].
    ///
    /// Three cursor modes, mutually exclusive:
    ///
    /// - `before = None, after = None` → latest `limit` entries
    ///   anchored at the head.
    /// - `before = Some(seq), after = None` → latest `limit` entries
    ///   strictly older than `seq` — backward pagination for the UI's
    ///   infinite-query.
    /// - `before = None, after = Some(seq)` → up to `limit` entries
    ///   strictly newer than `seq`, oldest-first — delta-replay for
    ///   reconnecting clients (mobile WS regaining focus after a tab
    ///   suspension). Hits when the mirror still holds every missed
    ///   item; if `after < oldest_seq_in_buffer` the response is
    ///   complete only up to the bounded ring, so [`ChatSnapshot::has_more`]
    ///   stays `true` and the client should fall back to a fresh head
    ///   fetch.
    ///
    /// Passing both `before` and `after` is rejected at the RPC layer
    /// (`-32602`). This method preserves backward semantics when both
    /// are passed — `before` wins — but callers should never get here
    /// with both set.
    ///
    /// `limit = 0` falls through to [`DEFAULT_CHAT_LIMIT`]. Any other
    /// value is honoured verbatim — frontends compute their own
    /// viewport-relative page size, and clamping daemon-side would
    /// force a one-size-fits-all heuristic that doesn't suit every
    /// consumer (phone vs 4K monitor vs neovim-side reader).
    pub async fn chat_snapshot(&self, before: Option<u64>, after: Option<u64>, limit: usize) -> ChatSnapshot {
        let limit = if limit == 0 { DEFAULT_CHAT_LIMIT } else { limit };
        let g = self.inner.read().await;

        // Forward window (delta-replay): items strictly newer than
        // `after`, oldest-first. Only consulted when `before` is
        // unset; callers that supply both are pinned to backward
        // semantics for legacy safety.
        if let (None, Some(cursor)) = (before, after) {
            let start_idx = g
                .transcript
                .iter()
                .position(|e| e.seq > cursor)
                .unwrap_or(g.transcript.len());
            let end_idx = (start_idx + limit).min(g.transcript.len());
            let items: Vec<SeqTranscriptItem> = g.transcript.range(start_idx..end_idx).cloned().collect();
            let oldest_seq = items.first().map(|e| e.seq);
            let latest_seq = items.last().map(|e| e.seq);
            // `has_more` here means strictly newer entries exist
            // beyond the returned window — the captain should keep
            // pulling.
            let has_more = end_idx < g.transcript.len();

            return ChatSnapshot {
                items,
                oldest_seq,
                latest_seq,
                has_more,
            };
        }

        let upper_idx = match before {
            // Find the first index whose seq >= cursor; everything
            // before it is strictly older.
            Some(cursor) => g
                .transcript
                .iter()
                .position(|e| e.seq >= cursor)
                .unwrap_or(g.transcript.len()),
            None => g.transcript.len(),
        };
        let lower_idx = upper_idx.saturating_sub(limit);

        let items: Vec<SeqTranscriptItem> = g.transcript.range(lower_idx..upper_idx).cloned().collect();
        let oldest_seq = items.first().map(|e| e.seq);
        let latest_seq = items.last().map(|e| e.seq);
        // `has_more` is true iff entries strictly older than the
        // returned window still exist in the buffer.
        let has_more = lower_idx > 0;

        ChatSnapshot {
            items,
            oldest_seq,
            latest_seq,
            has_more,
        }
    }

    /// Read a [`TerminalsSnapshot`].
    pub async fn terminals_snapshot(&self) -> TerminalsSnapshot {
        let g = self.inner.read().await;
        TerminalsSnapshot {
            terminals: g.terminals.clone(),
        }
    }

    /// Read the per-instance queue snapshot. Cheap clone of the cached
    /// `Vec<QueueItem>` — same as `chat_snapshot` / `terminals_snapshot`,
    /// no actor round-trip. Serves `instance/snapshot/queue` RPC +
    /// `instance_snapshot_queue` Tauri command + first-observation
    /// hydration in the Vue UI's `use-queue.ts::refreshQueue`.
    pub async fn queue_snapshot(&self) -> Vec<crate::adapters::queue::QueueItem> {
        let g = self.inner.read().await;
        g.queue.clone()
    }
}

/// Apply-then-broadcast helper. Every actor-side emit pairs a
/// [`InstanceMirror::apply`] with a `events_tx.send(...)`; the
/// **apply must come first** so that any reader pulling a snapshot
/// after `events_tx.send` returns sees a state that includes the
/// just-emitted event. See the module-level "Why not a broadcast
/// subscriber?" note for why the mirror cannot live downstream of
/// the broadcast.
///
/// Use this at every actor emit site. The `Drop` path in
/// `acp::instance::TurnGuard` is the documented exception — Drop
/// is sync, so the apply spawns onto the runtime separately.
pub async fn publish(
    mirror: &InstanceMirror,
    events_tx: &tokio::sync::broadcast::Sender<InstanceEvent>,
    mut event: InstanceEvent,
) {
    // Mint seq under the mirror's write lock, then stamp it onto the
    // event before broadcasting so subscribers see the canonical
    // value. The mirror is the single writer (no two `publish` calls
    // race the counter — every actor emit funnels through here on the
    // same async task). External WS / Tauri subscribers use the seq
    // as their delta-replay cursor on reconnect.
    if let Some(seq) = mirror.apply(&event).await {
        if let InstanceEvent::Transcript {
            seq: ref mut event_seq, ..
        } = &mut event
        {
            *event_seq = seq;
        }
    }
    let _ = events_tx.send(event);
}

// ─── snapshot wire shapes ─────────────────────────────────────────

/// Closed family of snapshot shapes the `instance/snapshot/*` RPC
/// returns. Tagged enum so a single Tauri command can dispatch on
/// `kind`. Variants differ in size (`MetaSnapshot` carries inline
/// pending-permissions + advertised mode/model lists; `Chat`'s
/// `Vec<SeqTranscriptItem>` is heap-stored) — boxing the largest
/// would force a heap allocation per dispatch, costing more than
/// the size disparity. Mirrors the same trade-off in
/// `acp::instance::MappedUpdate`.
#[allow(clippy::large_enum_variant)]
#[allow(dead_code)] // Phase A5 wires the RPC handlers.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum InstanceSnapshot {
    Meta(MetaSnapshot),
    Chat(ChatSnapshot),
    Terminals(TerminalsSnapshot),
}

/// Header / chrome view: cheap to fetch on focus-switch + on
/// `acp:instances-changed`. Mirrors `instance_meta` plus the
/// snapshot-only fields (`current_turn_event`, `pending_permissions`,
/// `usage`, `latest_seq`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaSnapshot {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_mode_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub available_modes: Vec<SessionModeInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub available_models: Vec<SessionModelInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub config_options: Vec<SessionConfigOptionCategory>,
    pub mcps_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_turn_event: Option<TurnEventMarker>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_permissions: Vec<PermissionRequestSnapshot>,
    /// Per-instance submit queue. Hydrates the UI's queue strip on
    /// snapshot load (mobile remote, refresh-mid-turn, second-frontend
    /// hand-off) so captains see "queued behind running turn" without
    /// waiting for the next `QueueChanged` event. Empty when no
    /// prompt is queued.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub queue: Vec<crate::adapters::queue::QueueItem>,
    pub usage: UsageSnapshot,
    /// Per-turn records (oldest-first). Hydrates the UI's `useTurns`
    /// store on snapshot load so the chat header's elapsed + usage
    /// chips render against the daemon's truth, not just the live
    /// event stream that may have run before the UI mounted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub turns: Vec<TurnSnapshot>,
    /// Latest [`SeqTranscriptItem::seq`] in the mirror; `None` when
    /// the transcript is empty. UI seeds the chat infinite-query off
    /// this so the first page request anchors at the right cursor.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_seq: Option<u64>,
}

/// Windowed transcript page. `oldest_seq` / `latest_seq` are absent
/// when `items` is empty.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSnapshot {
    pub items: Vec<SeqTranscriptItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oldest_seq: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_seq: Option<u64>,
    pub has_more: bool,
}

/// Full per-`terminal_id` map. Small enough that windowing buys
/// nothing for v1; revisit when a session accumulates dozens of
/// long-running terminals.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalsSnapshot {
    pub terminals: HashMap<String, TerminalSnapshot>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::permission::PermissionOptionView;
    use crate::adapters::transcript::TranscriptItem;
    use serde_json::json;

    /// `publish` must apply to the mirror BEFORE broadcasting — any
    /// reader that pulls a snapshot after the broadcast subscriber
    /// observes the event must already see the post-apply state.
    /// Pinning this via the only ordering observable from outside:
    /// after `publish` returns, `mirror.chat_snapshot()` already
    /// includes the event, AND the broadcast receiver has it queued.
    #[tokio::test]
    async fn publish_applies_before_broadcast() {
        let mirror = InstanceMirror::new();
        let (tx, mut rx) = tokio::sync::broadcast::channel::<InstanceEvent>(8);
        let event = transcript_event("hello");

        publish(&mirror, &tx, event).await;

        // Mirror has the event already.
        let snap = mirror.chat_snapshot(None, None, 0).await;
        assert_eq!(snap.items.len(), 1);

        // Broadcast queued the event for any subscriber.
        let received = rx.try_recv().expect("broadcast must have the event");
        match received {
            InstanceEvent::Transcript { item, .. } => match item {
                TranscriptItem::AgentText { text } => assert_eq!(text, "hello"),
                _ => panic!("wrong item variant"),
            },
            _ => panic!("wrong event variant"),
        }
    }

    /// Broadcast send returning Err (no subscribers) must not block
    /// the apply. The mirror is the daemon's truth; UI subscribers
    /// are best-effort.
    #[tokio::test]
    async fn publish_succeeds_with_zero_subscribers() {
        let mirror = InstanceMirror::new();
        let (tx, _rx) = tokio::sync::broadcast::channel::<InstanceEvent>(8);
        // Drop the subscriber — `tx.send` will return Err but `publish`
        // ignores it (the mirror still sees the event).
        drop(_rx);

        publish(&mirror, &tx, transcript_event("only-mirror")).await;

        let snap = mirror.chat_snapshot(None, None, 0).await;
        assert_eq!(snap.items.len(), 1);
    }

    fn transcript_event(text: &str) -> InstanceEvent {
        transcript_event_with_turn(text, None)
    }

    fn transcript_event_with_turn(text: &str, turn_id: Option<&str>) -> InstanceEvent {
        InstanceEvent::Transcript {
            agent_id: "claude-code".into(),
            instance_id: "i-1".into(),
            session_id: "s-1".into(),
            turn_id: turn_id.map(str::to_string),
            item: TranscriptItem::AgentText { text: text.into() },
            // Placeholder; `mirror.apply` mints the real value at
            // insertion time. Test helpers never assert on the
            // event's `seq` field directly — they read mirror state.
            seq: 0,
            meta: None,
        }
    }

    fn meta_event(cwd: &str, mode: Option<&str>, model: Option<&str>) -> InstanceEvent {
        InstanceEvent::InstanceMeta {
            agent_id: "claude-code".into(),
            instance_id: "i-1".into(),
            profile_id: Some("default".into()),
            session_id: Some("s-1".into()),
            cwd: cwd.into(),
            current_mode_id: mode.map(str::to_string),
            current_model_id: model.map(str::to_string),
            available_modes: Vec::new(),
            available_models: Vec::new(),
            mcps_count: 3,
        }
    }

    /// Inserting `cap + N` items drops the oldest `N` off the front
    /// while seq numbers keep climbing monotonically. Pin: surviving
    /// items' seqs are the last `cap` values issued.
    #[tokio::test]
    async fn ring_buffer_evicts_oldest_on_overflow() {
        let cap = 50;
        let overflow = 100;
        let mirror = InstanceMirror::with_cap(cap);
        for i in 0..(cap + overflow) {
            mirror.apply(&transcript_event(&format!("msg-{i}"))).await;
        }
        let snap = mirror.chat_snapshot(None, None, cap + overflow).await;
        assert_eq!(snap.items.len(), cap, "ring buffer caps at `cap` entries");
        let first_seq = snap.items.first().expect("non-empty").seq;
        let last_seq = snap.items.last().expect("non-empty").seq;
        // After 150 inserts capped at 50, surviving seqs are 100..=149.
        assert_eq!(first_seq, overflow as u64, "oldest survivor's seq");
        assert_eq!(last_seq, (cap + overflow - 1) as u64, "latest seq");
        assert!(!snap.has_more, "no entries older than the window");
    }

    /// 200 items, page backward in 50-item windows. Pin every
    /// boundary's `oldest_seq` / `latest_seq` / `has_more`.
    #[tokio::test]
    async fn chat_snapshot_paginates_backwards() {
        let mirror = InstanceMirror::with_cap(1_000);
        for i in 0..200 {
            mirror.apply(&transcript_event(&format!("msg-{i}"))).await;
        }

        // Latest 50: seqs 150..=199.
        let page1 = mirror.chat_snapshot(None, None, 50).await;
        assert_eq!(page1.items.len(), 50);
        assert_eq!(page1.oldest_seq, Some(150));
        assert_eq!(page1.latest_seq, Some(199));
        assert!(page1.has_more);

        // Older than 150: seqs 100..=149.
        let page2 = mirror.chat_snapshot(Some(150), None, 50).await;
        assert_eq!(page2.items.len(), 50);
        assert_eq!(page2.oldest_seq, Some(100));
        assert_eq!(page2.latest_seq, Some(149));
        assert!(page2.has_more);

        // Older than 50: seqs 0..=49 — last page.
        let page4 = mirror.chat_snapshot(Some(50), None, 50).await;
        assert_eq!(page4.items.len(), 50);
        assert_eq!(page4.oldest_seq, Some(0));
        assert_eq!(page4.latest_seq, Some(49));
        assert!(!page4.has_more, "exhausted the buffer");

        // Older than 0: empty.
        let page5 = mirror.chat_snapshot(Some(0), None, 50).await;
        assert!(page5.items.is_empty());
        assert!(!page5.has_more);
        assert_eq!(page5.oldest_seq, None);
        assert_eq!(page5.latest_seq, None);
    }

    /// `after` cursor: items strictly newer than the supplied seq,
    /// oldest-first. Models the remote reconnect path — captain saw
    /// up to seq 80, comes back online, asks for everything after.
    #[tokio::test]
    async fn chat_snapshot_paginates_forward_via_after_cursor() {
        let mirror = InstanceMirror::with_cap(1_000);
        for i in 0..100 {
            mirror.apply(&transcript_event(&format!("msg-{i}"))).await;
        }

        // Newer than 80, capped at 10: seqs 81..=90.
        let page = mirror.chat_snapshot(None, Some(80), 10).await;
        assert_eq!(page.items.len(), 10);
        assert_eq!(page.oldest_seq, Some(81));
        assert_eq!(page.latest_seq, Some(90));
        assert!(page.has_more, "still 9 more items past seq 90");

        // Newer than 90, drain remainder: seqs 91..=99.
        let page2 = mirror.chat_snapshot(None, Some(90), 100).await;
        assert_eq!(page2.items.len(), 9);
        assert_eq!(page2.oldest_seq, Some(91));
        assert_eq!(page2.latest_seq, Some(99));
        assert!(!page2.has_more, "no entries past seq 99");

        // Newer than the latest: empty.
        let page3 = mirror.chat_snapshot(None, Some(99), 50).await;
        assert!(page3.items.is_empty());
        assert!(!page3.has_more);
        assert_eq!(page3.oldest_seq, None);
        assert_eq!(page3.latest_seq, None);

        // Newer than seq=0: everything from seq 1 onwards.
        let from_one = mirror.chat_snapshot(None, Some(0), 200).await;
        assert_eq!(from_one.items.len(), 99);
        assert_eq!(from_one.oldest_seq, Some(1));
        assert_eq!(from_one.latest_seq, Some(99));
    }

    /// `publish` stamps the minted seq onto the broadcast event so
    /// remote subscribers can use it as their delta-replay cursor.
    /// Pinning this guarantees the wire-level `seq` matches what the
    /// snapshot side returns for the same item.
    #[tokio::test]
    async fn publish_stamps_seq_onto_broadcast_event() {
        let mirror = InstanceMirror::new();
        let (tx, mut rx) = tokio::sync::broadcast::channel::<InstanceEvent>(16);

        for (i, body) in ["one", "two", "three"].iter().enumerate() {
            publish(&mirror, &tx, transcript_event(body)).await;
            let received = rx.recv().await.expect("broadcast received");

            match received {
                InstanceEvent::Transcript { seq, .. } => {
                    assert_eq!(seq, i as u64, "broadcast seq mismatched mirror seq for item #{i}");
                }
                other => panic!("expected Transcript variant, got {other:?}"),
            }
        }
    }

    /// `limit = 0` falls through to the default page size.
    #[tokio::test]
    async fn chat_snapshot_zero_limit_uses_default() {
        let mirror = InstanceMirror::new();
        for i in 0..(DEFAULT_CHAT_LIMIT + 10) {
            mirror.apply(&transcript_event(&format!("msg-{i}"))).await;
        }
        let snap = mirror.chat_snapshot(None, None, 0).await;
        assert_eq!(snap.items.len(), DEFAULT_CHAT_LIMIT);
    }

    /// No-op variants leave the mirror unchanged.
    #[tokio::test]
    async fn apply_noops_do_not_mutate_state() {
        let mirror = InstanceMirror::new();
        mirror.apply(&transcript_event("hello")).await;

        let baseline_meta = mirror.meta_snapshot().await;
        let baseline_chat = mirror.chat_snapshot(None, None, 100).await;

        let noops = vec![
            InstanceEvent::State {
                agent_id: "claude-code".into(),
                instance_id: "i-1".into(),
                session_id: None,
                state: crate::adapters::InstanceState::Running,
            },
            InstanceEvent::InstancesChanged {
                instance_ids: vec!["i-1".into()],
                focused_id: Some("i-1".into()),
            },
            InstanceEvent::InstancesFocused {
                instance_id: Some("i-1".into()),
            },
            InstanceEvent::InstanceRenamed {
                instance_id: "i-1".into(),
                name: Some("alpha".into()),
            },
            InstanceEvent::DaemonReloaded {
                profiles: 0,
                skills_count: 0,
                mcps_count: 0,
            },
            InstanceEvent::SystemPromptInjected {
                agent_id: "claude-code".into(),
                instance_id: "i-1".into(),
                files: vec!["/tmp/p.md".into()],
            },
            InstanceEvent::SessionInfoUpdate {
                agent_id: "claude-code".into(),
                instance_id: "i-1".into(),
                session_id: "s-1".into(),
                title: Some("renamed".into()),
                updated_at: None,
            },
        ];
        for ev in noops {
            mirror.apply(&ev).await;
        }

        let after_meta = mirror.meta_snapshot().await;
        let after_chat = mirror.chat_snapshot(None, None, 100).await;

        // Pin every visible field — serializing makes the comparison
        // structural without manually unpacking each enum / vec.
        assert_eq!(
            serde_json::to_value(&baseline_meta).unwrap(),
            serde_json::to_value(&after_meta).unwrap(),
            "meta snapshot unchanged after no-op events"
        );
        assert_eq!(
            serde_json::to_value(&baseline_chat).unwrap(),
            serde_json::to_value(&after_chat).unwrap(),
            "chat snapshot unchanged after no-op events"
        );
    }

    /// `meta_snapshot` reflects the latest applied `InstanceMeta`
    /// event end-to-end (cwd, mode, model, mcps_count, profile_id).
    #[tokio::test]
    async fn meta_snapshot_reflects_latest_instance_meta() {
        let mirror = InstanceMirror::new();
        mirror
            .apply(&meta_event("/tmp/proj", Some("plan"), Some("sonnet")))
            .await;

        let snap = mirror.meta_snapshot().await;
        assert_eq!(snap.profile_id.as_deref(), Some("default"));
        assert_eq!(snap.session_id.as_deref(), Some("s-1"));
        assert_eq!(snap.cwd.as_deref(), Some("/tmp/proj"));
        assert_eq!(snap.current_mode_id.as_deref(), Some("plan"));
        assert_eq!(snap.current_model_id.as_deref(), Some("sonnet"));
        assert_eq!(snap.mcps_count, 3);

        // Latest InstanceMeta wins.
        mirror
            .apply(&meta_event("/tmp/other", Some("edit"), Some("opus")))
            .await;
        let snap2 = mirror.meta_snapshot().await;
        assert_eq!(snap2.cwd.as_deref(), Some("/tmp/other"));
        assert_eq!(snap2.current_mode_id.as_deref(), Some("edit"));
        assert_eq!(snap2.current_model_id.as_deref(), Some("opus"));

        // `CurrentModeUpdate` overlays the mode without resetting cwd.
        mirror
            .apply(&InstanceEvent::CurrentModeUpdate {
                agent_id: "claude-code".into(),
                instance_id: "i-1".into(),
                session_id: "s-1".into(),
                current_mode_id: "ask".into(),
            })
            .await;
        let snap3 = mirror.meta_snapshot().await;
        assert_eq!(snap3.current_mode_id.as_deref(), Some("ask"));
        assert_eq!(snap3.cwd.as_deref(), Some("/tmp/other"), "cwd unchanged");
    }

    /// Permission rows accumulate; turn markers track the latest
    /// boundary; usage tally resets per turn.
    #[tokio::test]
    async fn turn_permission_and_usage_state_track_correctly() {
        let mirror = InstanceMirror::new();
        let perm = InstanceEvent::PermissionRequest {
            agent_id: "claude-code".into(),
            instance_id: "i-1".into(),
            session_id: "s-1".into(),
            turn_id: Some("t-1".into()),
            request_id: "req-1".into(),
            tool: "Bash".into(),
            kind: "execute".into(),
            args: "ls".into(),
            raw_input: Some(json!({ "command": "ls" })),
            content: Vec::new(),
            options: vec![PermissionOptionView {
                option_id: "allow".into(),
                name: "Allow".into(),
                kind: "allow_once".into(),
            }],
            default_option_id: Some("allow".into()),
            formatted: crate::tools::formatter::types::FormattedToolCall {
                title: "Bash".into(),
                stats: Vec::new(),
                description: None,
                diff: None,
                output: None,
                fields: vec![],
            },
        };
        mirror.apply(&perm).await;
        // Idempotent: re-applying the same request_id is a no-op.
        mirror.apply(&perm).await;

        let snap = mirror.meta_snapshot().await;
        assert_eq!(snap.pending_permissions.len(), 1);
        assert_eq!(snap.pending_permissions[0].request_id, "req-1");

        // TurnStarted sets the marker + resets usage.
        mirror
            .apply(&InstanceEvent::TurnStarted {
                agent_id: "claude-code".into(),
                instance_id: "i-1".into(),
                session_id: "s-1".into(),
                turn_id: "t-1".into(),
                started_at: 1_700_000_000_000,
            })
            .await;
        let snap = mirror.meta_snapshot().await;
        assert!(matches!(
            snap.current_turn_event,
            Some(TurnEventMarker::Started { started_at }) if started_at == 1_700_000_000_000
        ));
        assert_eq!(snap.usage.turn_id.as_deref(), Some("t-1"));

        mirror
            .apply(&InstanceEvent::UsageUpdate {
                agent_id: "claude-code".into(),
                instance_id: "i-1".into(),
                session_id: "s-1".into(),
                turn_id: Some("t-1".into()),
                used: 1234,
                size: 200_000,
                cost: None,
            })
            .await;
        let snap = mirror.meta_snapshot().await;
        assert_eq!(snap.usage.used, 1234);
        assert_eq!(snap.usage.size, 200_000);

        mirror
            .apply(&InstanceEvent::TurnEnded {
                agent_id: "claude-code".into(),
                instance_id: "i-1".into(),
                session_id: "s-1".into(),
                turn_id: "t-1".into(),
                stop_reason: Some("end_turn".into()),
                error: None,
                ended_at: 1_700_000_001_000,
            })
            .await;
        let snap = mirror.meta_snapshot().await;
        assert!(matches!(
            snap.current_turn_event,
            Some(TurnEventMarker::Ended { ended_at }) if ended_at == 1_700_000_001_000
        ));
    }

    /// Phase C3: `SeqTranscriptItem.turn_id` mirrors the `turn_id`
    /// stamped on the originating `InstanceEvent::Transcript`. Items
    /// emitted between TurnStarted and TurnEnded carry the active
    /// turn id; items emitted outside a turn carry `None`.
    #[tokio::test]
    async fn transcript_items_stamp_active_turn_id() {
        let mirror = InstanceMirror::new();

        // Pre-turn item — no active turn, turn_id stays None.
        mirror.apply(&transcript_event_with_turn("pre-turn", None)).await;

        // Turn opens. (The marker isn't used for stamping; the actor
        // supplies the turn_id directly on the Transcript event.)
        mirror
            .apply(&InstanceEvent::TurnStarted {
                agent_id: "claude-code".into(),
                instance_id: "i-1".into(),
                session_id: "s-1".into(),
                turn_id: "t-1".into(),
                started_at: 1_700_000_000_000,
            })
            .await;

        mirror
            .apply(&transcript_event_with_turn("inside-turn-1", Some("t-1")))
            .await;
        mirror
            .apply(&transcript_event_with_turn("inside-turn-2", Some("t-1")))
            .await;

        // Turn ends.
        mirror
            .apply(&InstanceEvent::TurnEnded {
                agent_id: "claude-code".into(),
                instance_id: "i-1".into(),
                session_id: "s-1".into(),
                turn_id: "t-1".into(),
                stop_reason: Some("end_turn".into()),
                error: None,
                ended_at: 1_700_000_001_000,
            })
            .await;

        // Post-turn item — no active turn again.
        mirror.apply(&transcript_event_with_turn("post-turn", None)).await;

        // Second turn.
        mirror
            .apply(&InstanceEvent::TurnStarted {
                agent_id: "claude-code".into(),
                instance_id: "i-1".into(),
                session_id: "s-1".into(),
                turn_id: "t-2".into(),
                started_at: 1_700_000_002_000,
            })
            .await;
        mirror
            .apply(&transcript_event_with_turn("inside-turn-2-a", Some("t-2")))
            .await;

        let snap = mirror.chat_snapshot(None, None, 100).await;
        let turn_ids: Vec<Option<String>> = snap.items.iter().map(|i| i.turn_id.clone()).collect();
        assert_eq!(
            turn_ids,
            vec![
                None,
                Some("t-1".to_string()),
                Some("t-1".to_string()),
                None,
                Some("t-2".to_string()),
            ],
            "turn_id stamping: pre-turn → t-1 (×2) → post-turn → t-2"
        );
    }

    /// Per-turn records accumulate across `TurnStarted` /
    /// `UsageUpdate` / `TurnEnded` so the meta snapshot can ship them
    /// to the UI for `useTurns` hydration.
    #[tokio::test]
    async fn turns_accumulate_per_lifecycle_events() {
        let mirror = InstanceMirror::new();

        // First turn opens.
        mirror
            .apply(&InstanceEvent::TurnStarted {
                agent_id: "claude-code".into(),
                instance_id: "i-1".into(),
                session_id: "s-1".into(),
                turn_id: "t-1".into(),
                started_at: 1_000,
            })
            .await;

        let snap = mirror.meta_snapshot().await;
        assert_eq!(snap.turns.len(), 1);
        assert_eq!(snap.turns[0].id, "t-1");
        assert_eq!(snap.turns[0].started_at_ms, 1_000);
        assert!(snap.turns[0].ended_at_ms.is_none());
        assert!(snap.turns[0].usage.is_none());

        // Mid-turn usage update.
        mirror
            .apply(&InstanceEvent::UsageUpdate {
                agent_id: "claude-code".into(),
                instance_id: "i-1".into(),
                session_id: "s-1".into(),
                turn_id: Some("t-1".into()),
                used: 100,
                size: 200_000,
                cost: None,
            })
            .await;

        let snap = mirror.meta_snapshot().await;
        let usage = snap.turns[0].usage.as_ref().expect("usage attached");
        assert_eq!(usage.used, 100);
        assert_eq!(usage.size, 200_000);

        // Turn closes.
        mirror
            .apply(&InstanceEvent::TurnEnded {
                agent_id: "claude-code".into(),
                instance_id: "i-1".into(),
                session_id: "s-1".into(),
                turn_id: "t-1".into(),
                stop_reason: Some("end_turn".into()),
                error: None,
                ended_at: 2_000,
            })
            .await;

        let snap = mirror.meta_snapshot().await;
        assert_eq!(snap.turns[0].ended_at_ms, Some(2_000));
        assert_eq!(snap.turns[0].stop_reason.as_deref(), Some("end_turn"));

        // Second turn opens — appends without disturbing the first.
        mirror
            .apply(&InstanceEvent::TurnStarted {
                agent_id: "claude-code".into(),
                instance_id: "i-1".into(),
                session_id: "s-1".into(),
                turn_id: "t-2".into(),
                started_at: 3_000,
            })
            .await;

        let snap = mirror.meta_snapshot().await;
        assert_eq!(snap.turns.len(), 2);
        assert_eq!(snap.turns[1].id, "t-2");
        assert_eq!(snap.turns[1].started_at_ms, 3_000);

        // Idempotent: re-applying the same TurnStarted is a no-op.
        mirror
            .apply(&InstanceEvent::TurnStarted {
                agent_id: "claude-code".into(),
                instance_id: "i-1".into(),
                session_id: "s-1".into(),
                turn_id: "t-2".into(),
                started_at: 3_000,
            })
            .await;
        let snap = mirror.meta_snapshot().await;
        assert_eq!(snap.turns.len(), 2, "duplicate TurnStarted is a no-op");
    }

    /// Phase A4: a `PermissionResolved` event removes the matching row
    /// from `pending_permissions` so desktop ↔ remote subscribers don't
    /// keep showing a prompt the other transport already answered.
    #[tokio::test]
    async fn permission_resolved_removes_pending_row() {
        let mirror = InstanceMirror::new();
        let perm = |req_id: &str| InstanceEvent::PermissionRequest {
            agent_id: "claude-code".into(),
            instance_id: "i-1".into(),
            session_id: "s-1".into(),
            turn_id: Some("t-1".into()),
            request_id: req_id.into(),
            tool: "Bash".into(),
            kind: "execute".into(),
            args: "ls".into(),
            raw_input: None,
            content: Vec::new(),
            options: vec![PermissionOptionView {
                option_id: "allow".into(),
                name: "Allow".into(),
                kind: "allow_once".into(),
            }],
            default_option_id: Some("allow".into()),
            formatted: crate::tools::formatter::types::FormattedToolCall {
                title: "Bash".into(),
                stats: Vec::new(),
                description: None,
                diff: None,
                output: None,
                fields: vec![],
            },
        };
        mirror.apply(&perm("req-a")).await;
        mirror.apply(&perm("req-b")).await;
        assert_eq!(mirror.meta_snapshot().await.pending_permissions.len(), 2);

        // Resolve req-a — only its row drops.
        mirror
            .apply(&InstanceEvent::PermissionResolved {
                instance_id: "i-1".into(),
                request_id: "req-a".into(),
                option_id: "allow".into(),
            })
            .await;
        let snap = mirror.meta_snapshot().await;
        assert_eq!(snap.pending_permissions.len(), 1);
        assert_eq!(snap.pending_permissions[0].request_id, "req-b");

        // Idempotent: a second resolve for the same id is a no-op.
        mirror
            .apply(&InstanceEvent::PermissionResolved {
                instance_id: "i-1".into(),
                request_id: "req-a".into(),
                option_id: "allow".into(),
            })
            .await;
        assert_eq!(mirror.meta_snapshot().await.pending_permissions.len(), 1);
    }

    /// End-to-end write-through pin: simulate the actor's emit
    /// pattern by pairing every `mirror.apply(&event).await` with a
    /// `broadcast.send(event)` (mirroring the
    /// `mirror.apply(&event).await; let _ = events_tx.send(event)`
    /// pattern wired in `acp/instance.rs::run`). A fresh mirror that
    /// only consumes the broadcast stream MUST agree with the actor's
    /// mirror snapshot — that's the contract Phase A3 guarantees.
    #[tokio::test]
    async fn write_through_matches_subscriber_replay() {
        use tokio::sync::broadcast;

        let actor_mirror = InstanceMirror::new();
        let (events_tx, mut events_rx) = broadcast::channel::<InstanceEvent>(64);

        // Stream covers every variant the mirror's apply mutates on
        // (transcript, turn lifecycle, permission, instance meta,
        // current-mode update, usage, terminal, plus a no-op state
        // event to confirm noops don't desync).
        let stream: Vec<InstanceEvent> = vec![
            // Pre-turn meta refresh.
            meta_event("/tmp/proj", Some("plan"), Some("sonnet")),
            // Lifecycle (no-op for mirror, but rides the broadcast).
            InstanceEvent::State {
                agent_id: "claude-code".into(),
                instance_id: "i-1".into(),
                session_id: Some("s-1".into()),
                state: crate::adapters::InstanceState::Running,
            },
            // Turn opens.
            InstanceEvent::TurnStarted {
                agent_id: "claude-code".into(),
                instance_id: "i-1".into(),
                session_id: "s-1".into(),
                turn_id: "t-1".into(),
                started_at: 1_700_000_000_000,
            },
            // Two transcript chunks.
            transcript_event("first thought"),
            transcript_event("second thought"),
            // Mid-turn usage update.
            InstanceEvent::UsageUpdate {
                agent_id: "claude-code".into(),
                instance_id: "i-1".into(),
                session_id: "s-1".into(),
                turn_id: Some("t-1".into()),
                used: 5_000,
                size: 200_000,
                cost: None,
            },
            // Permission request.
            InstanceEvent::PermissionRequest {
                agent_id: "claude-code".into(),
                instance_id: "i-1".into(),
                session_id: "s-1".into(),
                turn_id: Some("t-1".into()),
                request_id: "req-write-through".into(),
                tool: "Bash".into(),
                kind: "execute".into(),
                args: "ls".into(),
                raw_input: Some(json!({ "command": "ls" })),
                content: Vec::new(),
                options: vec![PermissionOptionView {
                    option_id: "allow".into(),
                    name: "Allow".into(),
                    kind: "allow_once".into(),
                }],
                default_option_id: Some("allow".into()),
                formatted: crate::tools::formatter::types::FormattedToolCall {
                    title: "Bash".into(),
                    stats: Vec::new(),
                    description: None,
                    diff: None,
                    output: None,
                    fields: vec![],
                },
            },
            // Mode-switch overlay.
            InstanceEvent::CurrentModeUpdate {
                agent_id: "claude-code".into(),
                instance_id: "i-1".into(),
                session_id: "s-1".into(),
                current_mode_id: "edit".into(),
            },
            // Terminal output + exit pair.
            InstanceEvent::Terminal {
                agent_id: "claude-code".into(),
                instance_id: "i-1".into(),
                session_id: "s-1".into(),
                turn_id: Some("t-1".into()),
                terminal_id: "term-write-through".into(),
                chunk: TerminalChunk::Output {
                    stream: TerminalStream::Stdout,
                    data: "ok\n".into(),
                },
            },
            InstanceEvent::Terminal {
                agent_id: "claude-code".into(),
                instance_id: "i-1".into(),
                session_id: "s-1".into(),
                turn_id: Some("t-1".into()),
                terminal_id: "term-write-through".into(),
                chunk: TerminalChunk::Exit {
                    exit_code: Some(0),
                    signal: None,
                },
            },
            // Turn closes.
            InstanceEvent::TurnEnded {
                agent_id: "claude-code".into(),
                instance_id: "i-1".into(),
                session_id: "s-1".into(),
                turn_id: "t-1".into(),
                stop_reason: Some("end_turn".into()),
                error: None,
                ended_at: 1_700_000_001_000,
            },
        ];

        // Same shape as `acp/instance.rs::run`: apply first (so a
        // concurrent snapshot read sees consistent state), THEN
        // broadcast for downstream consumers.
        for event in &stream {
            actor_mirror.apply(event).await;
            events_tx.send(event.clone()).expect("subscriber alive");
        }

        // Replay through a fresh mirror exactly the way a snapshot-
        // hydrating consumer would: drain the broadcast and apply
        // each event in arrival order.
        let replay_mirror = InstanceMirror::new();
        for _ in 0..stream.len() {
            let evt = events_rx.recv().await.expect("broadcast healthy");
            replay_mirror.apply(&evt).await;
        }

        let actor_meta = actor_mirror.meta_snapshot().await;
        let replay_meta = replay_mirror.meta_snapshot().await;
        let actor_chat = actor_mirror.chat_snapshot(None, None, 100).await;
        let replay_chat = replay_mirror.chat_snapshot(None, None, 100).await;
        let actor_terms = actor_mirror.terminals_snapshot().await;
        let replay_terms = replay_mirror.terminals_snapshot().await;

        // Structural equality via JSON keeps the comparison stable
        // across the enum / vec / map nesting without unpacking by
        // hand. Same trick the no-op test uses.
        assert_eq!(
            serde_json::to_value(&actor_meta).unwrap(),
            serde_json::to_value(&replay_meta).unwrap(),
            "meta snapshot diverges between actor and replay"
        );
        assert_eq!(
            serde_json::to_value(&actor_chat).unwrap(),
            serde_json::to_value(&replay_chat).unwrap(),
            "chat snapshot diverges between actor and replay"
        );
        assert_eq!(
            serde_json::to_value(&actor_terms).unwrap(),
            serde_json::to_value(&replay_terms).unwrap(),
            "terminals snapshot diverges between actor and replay"
        );

        // Sanity-check the actor mirror's accumulated state matches
        // the events that flowed through — replay agreement above is
        // strong, but pin the load-bearing fields so a refactor
        // can't silently turn both halves into matching no-ops.
        assert_eq!(actor_meta.cwd.as_deref(), Some("/tmp/proj"));
        assert_eq!(actor_meta.current_mode_id.as_deref(), Some("edit"));
        assert_eq!(actor_meta.current_model_id.as_deref(), Some("sonnet"));
        assert_eq!(actor_meta.usage.used, 5_000);
        assert_eq!(actor_meta.pending_permissions.len(), 1);
        assert!(matches!(
            actor_meta.current_turn_event,
            Some(TurnEventMarker::Ended { .. })
        ));
        assert_eq!(actor_chat.items.len(), 2, "two transcript items");
        let term = actor_terms
            .terminals
            .get("term-write-through")
            .expect("terminal recorded");
        assert_eq!(term.stdout, "ok\n");
        assert!(!term.running);
        assert_eq!(term.exit_code, Some(0));
    }

    /// Terminal output accumulates per-stream; `Exit` flips `running`.
    #[tokio::test]
    async fn terminal_state_accumulates_then_exits() {
        let mirror = InstanceMirror::new();
        let chunk_out = InstanceEvent::Terminal {
            agent_id: "claude-code".into(),
            instance_id: "i-1".into(),
            session_id: "s-1".into(),
            turn_id: None,
            terminal_id: "term-1".into(),
            chunk: TerminalChunk::Output {
                stream: TerminalStream::Stdout,
                data: "hello".into(),
            },
        };
        mirror.apply(&chunk_out).await;
        mirror.apply(&chunk_out).await;
        let chunk_err = InstanceEvent::Terminal {
            agent_id: "claude-code".into(),
            instance_id: "i-1".into(),
            session_id: "s-1".into(),
            turn_id: None,
            terminal_id: "term-1".into(),
            chunk: TerminalChunk::Output {
                stream: TerminalStream::Stderr,
                data: "ERR".into(),
            },
        };
        mirror.apply(&chunk_err).await;
        mirror
            .apply(&InstanceEvent::Terminal {
                agent_id: "claude-code".into(),
                instance_id: "i-1".into(),
                session_id: "s-1".into(),
                turn_id: None,
                terminal_id: "term-1".into(),
                chunk: TerminalChunk::Exit {
                    exit_code: Some(0),
                    signal: None,
                },
            })
            .await;

        let snap = mirror.terminals_snapshot().await;
        let term = snap.terminals.get("term-1").expect("term-1 exists");
        assert_eq!(term.stdout, "hellohello");
        assert_eq!(term.stderr, "ERR");
        assert!(!term.running);
        assert_eq!(term.exit_code, Some(0));
    }

    fn sample_queue_item(id: &str, seq: u64) -> crate::adapters::queue::QueueItem {
        crate::adapters::queue::QueueItem {
            id: id.into(),
            text: format!("text-{id}"),
            attachments: vec![],
            enqueued_seq: seq,
            enqueued_at: seq as i64,
        }
    }

    /// Each `QueueChanged` replaces the cache wholesale. The mirror
    /// does not merge / preserve old entries — clients reconcile by
    /// replacement.
    #[tokio::test]
    async fn mirror_queue_apply_overwrites_with_full_state() {
        let mirror = InstanceMirror::new();
        mirror
            .apply(&InstanceEvent::QueueChanged {
                agent_id: "claude-code".into(),
                instance_id: "i-1".into(),
                items: vec![sample_queue_item("q-1", 1)],
            })
            .await;
        assert_eq!(mirror.queue_snapshot().await.len(), 1);

        mirror
            .apply(&InstanceEvent::QueueChanged {
                agent_id: "claude-code".into(),
                instance_id: "i-1".into(),
                items: vec![sample_queue_item("q-2", 2), sample_queue_item("q-3", 3)],
            })
            .await;
        let snap = mirror.queue_snapshot().await;
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].id, "q-2");
        assert_eq!(snap[1].id, "q-3");
    }

    /// Empty `QueueChanged` clears the cache (captain's `queue/clear`
    /// path). Mirror remains a faithful echo of the wire-side state.
    #[tokio::test]
    async fn mirror_queue_apply_with_empty_items_clears_the_cache() {
        let mirror = InstanceMirror::new();
        mirror
            .apply(&InstanceEvent::QueueChanged {
                agent_id: "claude-code".into(),
                instance_id: "i-1".into(),
                items: vec![sample_queue_item("q-1", 1)],
            })
            .await;
        mirror
            .apply(&InstanceEvent::QueueChanged {
                agent_id: "claude-code".into(),
                instance_id: "i-1".into(),
                items: vec![],
            })
            .await;
        assert_eq!(mirror.queue_snapshot().await.len(), 0);
    }
}

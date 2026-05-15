//! Daemon-side per-instance submit queue.
//!
//! Each running `AcpInstance` owns a FIFO of [`QueueItem`]s captains
//! enqueue while a turn is mid-flight. Mutations flow through the
//! actor's mailbox (so concurrent clients can't corrupt order); every
//! change emits `InstanceEvent::QueueChanged` carrying the full new
//! state, so subscribers (desktop overlay, mobile WS, hyprpilot-nvim)
//! reconcile by replacement, not by deltas.
//!
//! Pinned invariants:
//!
//! - **Captain controls**: the daemon never auto-dispatches the head on
//!   `TurnEnded`. Dispatch happens only via an explicit
//!   `queue/dispatch` RPC.
//! - **Cancel does not flush.** `prompts/cancel` only aborts the
//!   in-flight turn; queued items survive.
//! - **Cancel-during-dispatch loses the item.** Once `queue/dispatch`
//!   pops an item and starts its prompt future, a subsequent
//!   `prompts/cancel` kills the turn but does NOT re-enqueue the
//!   popped item — captain explicit re-submits if they wanted it.
//! - **Ordering is keyed off `enqueued_seq`**, the per-instance
//!   monotonic counter the actor mints under its single-mailbox lock.
//!   `enqueued_at` is informational (display-only).

use serde::{Deserialize, Serialize};

use crate::adapters::transcript::Attachment;

/// One entry in the daemon-side queue. Mirrors the UI's `QueuedItem`
/// shape minus the local-only `pills` field (image previews live
/// alongside the wire-shape `attachments` now).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QueueItem {
    /// Server-minted UUID v4. Stable across re-renders + reorders.
    /// Frontends use this as the `:key` for list rendering.
    pub id: String,
    pub text: String,
    /// Wire-shape attachments — skill resources, image data, etc.
    /// Same shape `session/prompt` expects. Empty by default and
    /// dropped from the wire when empty so the typical text-only
    /// enqueue stays compact.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
    /// Per-instance monotonic counter. Frontends order by this; ties
    /// can't happen because the actor mints it under a single-mailbox
    /// lock. Drives the QueueStrip `:key` so re-renders stay stable
    /// across reorder operations.
    pub enqueued_seq: u64,
    /// ms-epoch at enqueue time. Display-only ("queued 4s ago"). NOT
    /// load-bearing for ordering — that's `enqueued_seq`'s job. The
    /// wire ships an absolute value so clients with clock drift can
    /// still render consistent relative times against their own
    /// wall-clock.
    pub enqueued_at: i64,
}

/// Captain-supplied draft an enqueue call carries. The actor folds
/// it into a full `QueueItem` by minting `id`, `enqueued_seq`, and
/// `enqueued_at`.
#[derive(Debug, Clone)]
pub struct QueueItemDraft {
    pub text: String,
    pub attachments: Vec<Attachment>,
}

/// Reply shape for `queue/dispatch`. Mirrors the `prompts/send` reply
/// fields so second-frontends can reuse their existing "did this
/// land?" handling.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QueueDispatchResult {
    /// The item that was popped + dispatched. Frontends echo this on
    /// the local mirror's optimistic remove if they didn't wait for
    /// the matching `QueueChanged` broadcast.
    pub item: QueueItem,
    /// ACP session id the prompt landed on. Mirrors
    /// `SubmitResult.sessionId`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Turn id the agent assigned to the dispatched prompt. Mirrors
    /// `SubmitResult.turnId`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    /// `true` when the actor accepted the prompt for dispatch; `false`
    /// when the queue was empty / item id missing / dispatch failed
    /// before the request left the wire.
    pub accepted: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(seq: u64) -> QueueItem {
        QueueItem {
            id: format!("q-{seq}"),
            text: "hello".into(),
            attachments: vec![],
            enqueued_seq: seq,
            enqueued_at: 1700000000,
        }
    }

    /// Pin the camelCase wire shape — frontends key off `enqueuedAt`
    /// + `enqueuedSeq` literally; a snake_case slip would silently
    /// strand the ordering.
    #[test]
    fn queue_item_serialises_to_camel_case() {
        let v = serde_json::to_value(sample(7)).expect("serialise");

        assert_eq!(v["id"], "q-7");
        assert_eq!(v["text"], "hello");
        assert_eq!(v["enqueuedSeq"], 7);
        assert_eq!(v["enqueuedAt"], 1700000000);
        assert!(
            v.get("attachments").is_none(),
            "empty attachments must drop off the wire so text-only enqueues stay compact"
        );
    }

    /// Round-trip identity — what we serialise we deserialise.
    #[test]
    fn queue_item_round_trips() {
        let original = sample(42);
        let json = serde_json::to_string(&original).unwrap();
        let parsed: QueueItem = serde_json::from_str(&json).unwrap();

        assert_eq!(original, parsed);
    }

    /// `QueueDispatchResult` mirrors the `prompts/send` reply: when no
    /// session is open yet the optional fields drop off the wire so
    /// thin clients don't have to type-narrow.
    #[test]
    fn dispatch_result_drops_empty_optionals() {
        let v = serde_json::to_value(QueueDispatchResult {
            item: sample(0),
            session_id: None,
            turn_id: None,
            accepted: false,
        })
        .unwrap();

        assert!(v.get("sessionId").is_none());
        assert!(v.get("turnId").is_none());
        assert_eq!(v["accepted"], false);
    }
}

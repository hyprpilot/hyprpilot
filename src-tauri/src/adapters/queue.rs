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

/// Reply shape for `queue/dispatch`. Mirrors the `prompts/send` reply's
/// "did the actor accept this?" semantics. `accepted` flips immediately
/// on actor-accept (NOT on turn completion) so the captain's UI spinner
/// resolves in milliseconds; the eventual `acp:turn-ended` event
/// (carrying the real `turnId`) is the completion signal.
///
/// `item` is `None` when the queue was empty (head dispatch) or the
/// named id wasn't in the queue — frontends render "queue empty" /
/// "item already drained" without unwrapping a sentinel.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QueueDispatchResult {
    /// The item that was popped + dispatched. `None` when the dispatch
    /// found nothing (empty queue / unknown id) — `accepted` is `false`
    /// in that case.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<QueueItem>,
    /// ACP session id the prompt will land on. Mirrors
    /// `SubmitResult.sessionId`. `None` on dispatch-failure paths or
    /// pre-`session/new` instances.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// `true` when the actor took the popped item and forwarded it
    /// down the prompt path. `false` when the queue was empty / item
    /// id missing / the actor channel was closed.
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
    /// and `enqueuedSeq` literally; a snake_case slip would silently
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

    /// `QueueDispatchResult` mirrors the `prompts/send` reply: when
    /// nothing was popped (empty queue / unknown id) the `item` drops
    /// off the wire so thin clients don't have to type-narrow against
    /// a sentinel. `session_id` drops similarly when the dispatch
    /// failed pre-`session/new`.
    #[test]
    fn dispatch_result_drops_empty_optionals_when_unaccepted() {
        let v = serde_json::to_value(QueueDispatchResult {
            item: None,
            session_id: None,
            accepted: false,
        })
        .unwrap();

        assert!(v.get("item").is_none(), "item must drop on empty-queue: {v}");
        assert!(v.get("sessionId").is_none(), "sessionId must drop pre-spawn: {v}");
        assert_eq!(v["accepted"], false);
    }

    /// Happy path: queue had an item, actor accepted, session id known.
    #[test]
    fn dispatch_result_carries_item_and_session_on_accept() {
        let v = serde_json::to_value(QueueDispatchResult {
            item: Some(sample(3)),
            session_id: Some("s-1".into()),
            accepted: true,
        })
        .unwrap();

        assert_eq!(v["item"]["id"], "q-3");
        assert_eq!(v["sessionId"], "s-1");
        assert_eq!(v["accepted"], true);
    }
}

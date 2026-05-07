//! Tauri commands the desktop overlay calls during the pair-confirm
//! flow. The phone-side WS handler in `crate::remote::ws` waits on
//! the `oneshot` signal these commands fire; matching code → unblocks
//! the WS task and the connection upgrades to authenticated.

use serde::Serialize;
use tauri::State;
use uuid::Uuid;

use crate::remote::pair::{ConfirmSide, PairError, PairStore, PendingPairView};

/// `remote_confirm_pair` — captain typed (or scanned) the **device's**
/// code on the desktop modal. The candidate is matched against the
/// device's code only; presenting the desktop's own code (the one
/// already visible on this side) would prove nothing and is rejected.
///
/// Returns `{ confirmed: true }` on match, or a structured error
/// describing why confirmation failed (mismatch, expired, burned).
#[tauri::command]
pub async fn remote_confirm_pair(
    pairs: State<'_, PairStore>,
    pending_id: String,
    code: String,
) -> Result<ConfirmResult, String> {
    let id = Uuid::parse_str(&pending_id).map_err(|e| format!("invalid pending_id: {e}"))?;
    match pairs.confirm(&id, &code, ConfirmSide::Desktop) {
        Ok(()) => Ok(ConfirmResult { confirmed: true }),
        Err(err) => Err(err_message(err)),
    }
}

/// `remote_reject_pair` — captain dismissed the modal. Burns the
/// pending state; the WS task observes the dropped sender via its
/// `oneshot::Receiver` and closes.
#[tauri::command]
pub async fn remote_reject_pair(pairs: State<'_, PairStore>, pending_id: String) -> Result<(), String> {
    let id = Uuid::parse_str(&pending_id).map_err(|e| format!("invalid pending_id: {e}"))?;
    pairs.reject(&id);
    Ok(())
}

/// `remote_pending_pairs` — current pending requests, surfaced for
/// the desktop palette to render. Mostly diagnostic / for future
/// "queue of waiting devices" UX.
#[tauri::command]
pub async fn remote_pending_pairs(pairs: State<'_, PairStore>) -> Result<Vec<PendingPairView>, String> {
    Ok(pairs.snapshot())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmResult {
    pub confirmed: bool,
}

fn err_message(err: PairError) -> String {
    match err {
        PairError::Unknown => "unknown pending pair id".into(),
        PairError::Mismatch => "pair code does not match".into(),
        PairError::Expired => "pair request expired".into(),
        PairError::TooManyAttempts => "too many failed attempts; pair request burned".into(),
    }
}

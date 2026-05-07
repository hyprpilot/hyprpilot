//! Pending-pair store + BIP39 code generation.
//!
//! Each WS connection mints **two** distinct 4-word phrases — one
//! shown on the connecting device (`device_code`) and one shown on
//! the desktop (`desktop_code`). True proof-of-presence pairing:
//!
//! - To confirm from the desktop side, the captain must read /
//!   scan the **device's** code (only visible on the device).
//! - To confirm from the device side, the captain must read /
//!   scan the **desktop's** code (only visible on the desktop).
//!
//! A single shared code is what naive pairing UIs ship; that just
//! proves the captain can read whichever screen they're already
//! looking at, not that they have eyes on **both** devices.
//!
//! Expiry: 60 seconds without confirmation → auto-removed and the
//! WS task closes the connection. Three failed confirm attempts
//! also burn the pending state.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use bip39::Mnemonic;
use rand::RngCore;
use tokio::sync::oneshot;
use uuid::Uuid;

/// How long a pending pair request lives before auto-expiring.
pub const PAIR_EXPIRY: Duration = Duration::from_secs(60);

/// Maximum confirm attempts per pending request before it's burned.
pub const PAIR_MAX_ATTEMPTS: u32 = 3;

/// 4-word BIP39 phrase. 4 words from the BIP39 word list = ~44 bits
/// of entropy — practically irrelevant for brute force given the
/// 60s window + 3-attempt cap, but bigger than a 6-digit numeric
/// while staying friendly to read aloud / type on a phone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairCode(String);

impl PairCode {
    pub fn generate() -> Self {
        // bip39 only mints whole-mnemonic counts (12/15/18/21/24);
        // we pull a 12-word mnemonic and slice the first 4. Simpler
        // than building our own 44-bit-from-wordlist primitive.
        let mut entropy = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut entropy);
        let mnemonic = Mnemonic::from_entropy(&entropy).expect("16 bytes is a valid bip39 entropy length");
        let words: Vec<&str> = mnemonic.words().collect();
        let chosen: Vec<&str> = words.into_iter().take(4).collect();
        Self(chosen.join(" "))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Comparison normalises whitespace + case so the captain typing
    /// `Alpha   bravo Charlie  delta` matches the canonical
    /// `alpha bravo charlie delta`.
    pub fn matches(&self, candidate: &str) -> bool {
        normalise(candidate) == normalise(&self.0)
    }
}

fn normalise(s: &str) -> String {
    s.split_whitespace()
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Which side the confirmation came from. Each side's code is the
/// *other* side's expected input — desktop matches against the
/// device's code, device matches against the desktop's code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmSide {
    /// Desktop captain typed / scanned the device's code.
    Desktop,
    /// Connecting device sent a `{type:"confirm"}` frame carrying
    /// the desktop's code (mostly: device camera scanned the
    /// desktop modal's QR).
    Device,
}

/// One pending pair request — keyed by `pending_id`. Owns a
/// `oneshot::Sender<()>` the WS task awaits; confirming the pair
/// fires it and the WS upgrades to authenticated.
pub struct PairRequest {
    /// Code rendered on the connecting device's pair screen. The
    /// desktop must present this (typed or scanned from the device's
    /// QR) to confirm from the desktop side.
    pub device_code: PairCode,
    /// Code rendered on the desktop modal. The device must present
    /// this (typed in its own input or scanned from the desktop's
    /// QR) to confirm from the device side.
    pub desktop_code: PairCode,
    pub created_at: Instant,
    pub attempts: u32,
    pub remote_addr: String,
    pub confirm_tx: Option<oneshot::Sender<()>>,
}

/// Process-wide pending-pair state. Cheap `Arc<RwLock<...>>` since
/// pair traffic is rare relative to RPC traffic.
#[derive(Clone, Default)]
pub struct PairStore {
    inner: Arc<RwLock<HashMap<Uuid, PairRequest>>>,
}

/// Tuple returned by `PairStore::create` to make call sites readable —
/// a long unnamed quadruple here would shift around silently when we
/// ever extend the wire shape.
pub struct CreatedPair {
    pub pending_id: Uuid,
    pub device_code: PairCode,
    pub desktop_code: PairCode,
    pub confirm_rx: oneshot::Receiver<()>,
}

impl PairStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mint a new pending request with both codes. Caller awaits the
    /// returned `oneshot::Receiver<()>` to know when either side
    /// confirmed.
    pub fn create(&self, remote_addr: String) -> CreatedPair {
        let id = Uuid::new_v4();
        let device_code = PairCode::generate();
        let desktop_code = PairCode::generate();
        let (tx, rx) = oneshot::channel();
        let req = PairRequest {
            device_code: device_code.clone(),
            desktop_code: desktop_code.clone(),
            created_at: Instant::now(),
            attempts: 0,
            remote_addr,
            confirm_tx: Some(tx),
        };
        {
            let mut map = self.inner.write().expect("PairStore poisoned");
            map.insert(id, req);
        }
        CreatedPair {
            pending_id: id,
            device_code,
            desktop_code,
            confirm_rx: rx,
        }
    }

    /// Submit a candidate code for `pending_id` from `side`. The
    /// match target depends on the side: desktop side matches against
    /// `device_code` (proves the captain saw the device); device side
    /// matches against `desktop_code` (proves the device's user saw
    /// the desktop).
    pub fn confirm(&self, pending_id: &Uuid, candidate: &str, side: ConfirmSide) -> Result<(), PairError> {
        let mut map = self.inner.write().expect("PairStore poisoned");
        let req = map.get_mut(pending_id).ok_or(PairError::Unknown)?;

        if req.created_at.elapsed() > PAIR_EXPIRY {
            map.remove(pending_id);
            return Err(PairError::Expired);
        }

        req.attempts += 1;
        let expected = match side {
            ConfirmSide::Desktop => &req.device_code,
            ConfirmSide::Device => &req.desktop_code,
        };
        if expected.matches(candidate) {
            // Fire the confirm signal. Removing the entry from the
            // map drops the sender, but we explicitly take + send so
            // the WS task observes a clean signal.
            let tx = req.confirm_tx.take();
            map.remove(pending_id);
            drop(map);
            if let Some(tx) = tx {
                let _ = tx.send(());
            }
            Ok(())
        } else if req.attempts >= PAIR_MAX_ATTEMPTS {
            map.remove(pending_id);
            Err(PairError::TooManyAttempts)
        } else {
            Err(PairError::Mismatch)
        }
    }

    /// Captain rejected — burn the pending state. WS task observes
    /// the dropped sender via its `oneshot::Receiver`.
    pub fn reject(&self, pending_id: &Uuid) {
        let mut map = self.inner.write().expect("PairStore poisoned");
        map.remove(pending_id);
    }

    /// Fire the confirm signal without checking either code. Used
    /// by the session-token (`hello`) reauth path: the token is the
    /// proof-of-presence here, not the BIP39 phrase. No-op when the
    /// pending is unknown or already burned.
    pub fn fast_confirm(&self, pending_id: &Uuid) {
        let tx = {
            let mut map = self.inner.write().expect("PairStore poisoned");
            match map.get_mut(pending_id) {
                Some(req) => req.confirm_tx.take().inspect(|_| {
                    map.remove(pending_id);
                }),
                None => None,
            }
        };
        if let Some(tx) = tx {
            let _ = tx.send(());
        }
    }

    /// Current pending requests, snapshot. Surfaced via
    /// `remote_pending_pairs` Tauri command. Renders both codes so
    /// the desktop palette can match what's on screen.
    pub fn snapshot(&self) -> Vec<PendingPairView> {
        let map = self.inner.read().expect("PairStore poisoned");
        map.iter()
            .map(|(id, r)| PendingPairView {
                pending_id: *id,
                device_code: r.device_code.as_str().to_string(),
                desktop_code: r.desktop_code.as_str().to_string(),
                remote_addr: r.remote_addr.clone(),
                created_at_ms: epoch_ms(r.created_at),
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairError {
    Unknown,
    Mismatch,
    Expired,
    TooManyAttempts,
}

impl std::fmt::Display for PairError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown => write!(f, "unknown pending pair id"),
            Self::Mismatch => write!(f, "pair code does not match"),
            Self::Expired => write!(f, "pair request expired"),
            Self::TooManyAttempts => write!(f, "too many failed attempts; pair request burned"),
        }
    }
}

impl std::error::Error for PairError {}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingPairView {
    pub pending_id: Uuid,
    /// Code rendered on the connecting device.
    pub device_code: String,
    /// Code rendered on the desktop modal.
    pub desktop_code: String,
    pub remote_addr: String,
    pub created_at_ms: u64,
}

fn epoch_ms(_: Instant) -> u64 {
    // `Instant` doesn't expose epoch — for the wire we pair with
    // a captured wall-clock time when a request lands. Today the
    // UI just renders "moments ago" so a relative offset would do,
    // but the simplest wire shape is "now-when-sampled". Tradeoff:
    // a snapshot-time timestamp is approximate; close enough for
    // the modal's "device wants to pair" ephemeral display.
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pair_code_generates_four_words() {
        let code = PairCode::generate();
        assert_eq!(code.as_str().split_whitespace().count(), 4);
    }

    #[test]
    fn pair_create_mints_two_distinct_codes() {
        let store = PairStore::new();
        let CreatedPair {
            device_code,
            desktop_code,
            ..
        } = store.create("phone".into());
        assert_ne!(
            device_code, desktop_code,
            "device + desktop codes must differ — same code defeats pairing"
        );
        assert_eq!(device_code.as_str().split_whitespace().count(), 4);
        assert_eq!(desktop_code.as_str().split_whitespace().count(), 4);
    }

    #[test]
    fn pair_code_match_is_case_insensitive_and_whitespace_lenient() {
        let code = PairCode("alpha bravo charlie delta".to_string());
        assert!(code.matches("alpha bravo charlie delta"));
        assert!(code.matches("ALPHA   bravo Charlie  delta"));
        assert!(code.matches("\talpha bravo  charlie delta\n"));
        assert!(!code.matches("alpha bravo charlie"));
        assert!(!code.matches("alpha bravo charlie zeta"));
    }

    #[test]
    fn confirm_from_desktop_matches_device_code_only() {
        let store = PairStore::new();
        let CreatedPair {
            pending_id,
            device_code,
            desktop_code,
            confirm_rx,
        } = store.create("phone".into());

        // Desktop sending the desktop's own code = wrong; that code
        // never left the desktop, so anyone presenting it from the
        // desktop side proves nothing.
        assert_eq!(
            store.confirm(&pending_id, desktop_code.as_str(), ConfirmSide::Desktop),
            Err(PairError::Mismatch)
        );

        // Desktop sending the device's code = right; only obtainable
        // by reading or scanning the device's screen.
        store
            .confirm(&pending_id, device_code.as_str(), ConfirmSide::Desktop)
            .expect("device code from desktop side should match");
        let _ = confirm_rx.blocking_recv();
    }

    #[test]
    fn confirm_from_device_matches_desktop_code_only() {
        let store = PairStore::new();
        let CreatedPair {
            pending_id,
            device_code,
            desktop_code,
            confirm_rx,
        } = store.create("phone".into());

        assert_eq!(
            store.confirm(&pending_id, device_code.as_str(), ConfirmSide::Device),
            Err(PairError::Mismatch)
        );

        store
            .confirm(&pending_id, desktop_code.as_str(), ConfirmSide::Device)
            .expect("desktop code from device side should match");
        let _ = confirm_rx.blocking_recv();
    }

    #[test]
    fn store_confirm_wrong_code_increments_attempts_then_burns() {
        let store = PairStore::new();
        let CreatedPair { pending_id, .. } = store.create("phone".into());
        for _ in 0..(PAIR_MAX_ATTEMPTS - 1) {
            assert_eq!(
                store.confirm(&pending_id, "wrong words here please", ConfirmSide::Desktop),
                Err(PairError::Mismatch)
            );
        }
        // Final wrong attempt burns the pending state.
        assert_eq!(
            store.confirm(&pending_id, "still wrong words here", ConfirmSide::Desktop),
            Err(PairError::TooManyAttempts)
        );
        // Subsequent attempts return Unknown.
        assert_eq!(
            store.confirm(&pending_id, "anything goes", ConfirmSide::Desktop),
            Err(PairError::Unknown)
        );
    }

    #[test]
    fn store_reject_clears_pending_state() {
        let store = PairStore::new();
        let CreatedPair { pending_id, .. } = store.create("phone".into());
        store.reject(&pending_id);
        assert_eq!(
            store.confirm(&pending_id, "anything", ConfirmSide::Desktop),
            Err(PairError::Unknown)
        );
    }
}

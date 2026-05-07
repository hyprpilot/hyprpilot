//! Pending-pair store + BIP39 code generation.
//!
//! Each WS connection that doesn't carry an authenticated session
//! mints a `PairRequest`: a UUID `pending_id` + a 4-word BIP39
//! phrase. Pending requests live in memory only — restart wipes them.
//! The captain confirms by typing (or scanning) the phrase on the
//! desktop; the daemon matches against the pending request and
//! upgrades the WS to authenticated.
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

/// One pending pair request — keyed by `pending_id`. Owns a
/// `oneshot::Sender<()>` the WS task awaits; confirming the pair
/// fires it and the WS upgrades to authenticated.
pub struct PairRequest {
    pub code: PairCode,
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

impl PairStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mint a new pending request. Caller awaits the returned
    /// `oneshot::Receiver<()>` to know when the captain confirmed.
    pub fn create(&self, remote_addr: String) -> (Uuid, PairCode, oneshot::Receiver<()>) {
        let id = Uuid::new_v4();
        let code = PairCode::generate();
        let (tx, rx) = oneshot::channel();
        let req = PairRequest {
            code: code.clone(),
            created_at: Instant::now(),
            attempts: 0,
            remote_addr,
            confirm_tx: Some(tx),
        };
        {
            let mut map = self.inner.write().expect("PairStore poisoned");
            map.insert(id, req);
        }
        (id, code, rx)
    }

    /// Captain submits a candidate code for `pending_id`. Returns:
    /// - `Ok(())` on match — fires the WS task's confirm signal.
    /// - `Err(PairError::Mismatch)` — wrong code, attempts incremented.
    /// - `Err(PairError::Expired)` — request aged out.
    /// - `Err(PairError::TooManyAttempts)` — burned.
    /// - `Err(PairError::Unknown)` — pending_id not found.
    pub fn confirm(&self, pending_id: &Uuid, candidate: &str) -> Result<(), PairError> {
        let mut map = self.inner.write().expect("PairStore poisoned");
        let req = map.get_mut(pending_id).ok_or(PairError::Unknown)?;

        if req.created_at.elapsed() > PAIR_EXPIRY {
            map.remove(pending_id);
            return Err(PairError::Expired);
        }

        req.attempts += 1;
        if req.code.matches(candidate) {
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

    /// Current pending requests, snapshot. Surfaced via
    /// `remote/pending-pairs` RPC for the desktop palette to render.
    pub fn snapshot(&self) -> Vec<PendingPairView> {
        let map = self.inner.read().expect("PairStore poisoned");
        map.iter()
            .map(|(id, r)| PendingPairView {
                pending_id: *id,
                code: r.code.as_str().to_string(),
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
    /// Captain-facing code — surfaced so the palette modal can show
    /// it alongside the QR for read-aloud confirmation.
    pub code: String,
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
    fn pair_code_match_is_case_insensitive_and_whitespace_lenient() {
        let code = PairCode("alpha bravo charlie delta".to_string());
        assert!(code.matches("alpha bravo charlie delta"));
        assert!(code.matches("ALPHA   bravo Charlie  delta"));
        assert!(code.matches("\talpha bravo  charlie delta\n"));
        assert!(!code.matches("alpha bravo charlie"));
        assert!(!code.matches("alpha bravo charlie zeta"));
    }

    #[test]
    fn store_create_then_confirm_fires_signal() {
        let store = PairStore::new();
        let (id, code, rx) = store.create("phone".into());
        store.confirm(&id, code.as_str()).expect("matches");
        let _ = rx.blocking_recv();
    }

    #[test]
    fn store_confirm_wrong_code_increments_attempts_then_burns() {
        let store = PairStore::new();
        let (id, _, _rx) = store.create("phone".into());
        for _ in 0..(PAIR_MAX_ATTEMPTS - 1) {
            assert_eq!(store.confirm(&id, "wrong words here please"), Err(PairError::Mismatch));
        }
        // Final wrong attempt burns the pending state.
        assert_eq!(
            store.confirm(&id, "still wrong words here"),
            Err(PairError::TooManyAttempts)
        );
        // Subsequent attempts return Unknown.
        assert_eq!(store.confirm(&id, "anything goes"), Err(PairError::Unknown));
    }

    #[test]
    fn store_reject_clears_pending_state() {
        let store = PairStore::new();
        let (id, _, _rx) = store.create("phone".into());
        store.reject(&id);
        assert_eq!(store.confirm(&id, "anything"), Err(PairError::Unknown));
    }
}

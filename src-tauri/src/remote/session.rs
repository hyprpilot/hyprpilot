//! In-memory session tokens — survive WS reconnects within the same
//! daemon run so the captain pairs once per `hyprpilot daemon` rather
//! than once per page reload. Daemon restart wipes the table; mirrors
//! the rest of the remote bridge's "no disk persistence" model.
//!
//! Wire shape:
//! - On successful pair confirm, the daemon mints a fresh UUID token
//!   and sends it in the `authenticated` frame:
//!   `{ "type": "authenticated", "sessionToken": "<uuid>" }`
//! - The browser stores it in `localStorage` and on next reconnect
//!   sends a `hello` frame during the pending window:
//!   `{ "type": "hello", "sessionToken": "<uuid>" }`
//! - The daemon validates against the in-memory store; valid → fire
//!   the same oneshot a normal confirm fires, WS upgrades. Invalid
//!   tokens are silently ignored — pair flow continues, captain
//!   confirms manually, fresh token replaces the stale one.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use uuid::Uuid;

/// Tokens older than this are evicted opportunistically. Captures the
/// "if a phone hasn't checked in for this long, the captain has either
/// stopped using it or paired anew already" intuition. Daemon-lifetime
/// growth from repeat reconnects compounds without this — every
/// failed-then-retried pair leaves another stale token behind.
const TOKEN_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Diagnostic metadata for an active token. Not used in any decision
/// today — `validate` only checks the key — but plumbed through for
/// a future "what's currently paired" surface (palette leaf / ctl
/// command). `#[allow(dead_code)]` documents the intent.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SessionMeta {
    pub remote_addr: String,
    pub created_at: Instant,
}

#[derive(Clone, Default)]
pub struct SessionTokens {
    inner: Arc<RwLock<HashMap<String, SessionMeta>>>,
}

impl SessionTokens {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mint a fresh token bound to the (in-memory) `remote_addr` for
    /// diagnostics. Returns the opaque token string the WS handler
    /// includes in the `authenticated` frame. Opportunistically prunes
    /// stale tokens past `TOKEN_MAX_AGE` so the table doesn't grow
    /// unbounded across daemon-lifetime reconnects.
    pub fn mint(&self, remote_addr: String) -> String {
        let token = Uuid::new_v4().to_string();
        let mut map = self.inner.write().expect("SessionTokens poisoned");
        prune_expired(&mut map);
        map.insert(
            token.clone(),
            SessionMeta {
                remote_addr,
                created_at: Instant::now(),
            },
        );
        token
    }

    /// True when `token` is in the live set and within the age window.
    /// Stale tokens are dropped on first observation so the next
    /// `validate` after expiry already returns false.
    pub fn validate(&self, token: &str) -> bool {
        let mut map = self.inner.write().expect("SessionTokens poisoned");
        prune_expired(&mut map);
        map.contains_key(token)
    }

    /// Remove a token (e.g. on captain-driven revoke). Today nothing
    /// calls this — daemon restart is the only revoke path the
    /// captain has — but the surface is here for the future.
    #[allow(dead_code)]
    pub fn revoke(&self, token: &str) {
        let mut map = self.inner.write().expect("SessionTokens poisoned");
        map.remove(token);
    }

    /// Drop every token. Future hook for a `remote/revoke-all` RPC.
    #[allow(dead_code)]
    pub fn clear(&self) {
        let mut map = self.inner.write().expect("SessionTokens poisoned");
        map.clear();
    }
}

fn prune_expired(map: &mut HashMap<String, SessionMeta>) {
    map.retain(|_, meta| meta.created_at.elapsed() < TOKEN_MAX_AGE);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mint_then_validate() {
        let tokens = SessionTokens::new();
        let token = tokens.mint("phone".into());
        assert!(tokens.validate(&token));
    }

    #[test]
    fn unknown_token_does_not_validate() {
        let tokens = SessionTokens::new();
        assert!(!tokens.validate("not-a-token"));
    }

    #[test]
    fn revoke_invalidates() {
        let tokens = SessionTokens::new();
        let token = tokens.mint("phone".into());
        tokens.revoke(&token);
        assert!(!tokens.validate(&token));
    }

    #[test]
    fn each_mint_is_unique() {
        let tokens = SessionTokens::new();
        let a = tokens.mint("phone".into());
        let b = tokens.mint("phone".into());
        assert_ne!(a, b);
        assert!(tokens.validate(&a));
        assert!(tokens.validate(&b));
    }
}

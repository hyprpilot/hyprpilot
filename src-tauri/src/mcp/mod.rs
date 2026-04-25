//! MCP server catalogue. Owns the `[[mcps]]` entries from config and
//! exposes them through `MCPsRegistry` for the socket RPC handlers
//! (`mcps/list`, `mcps/set`) plus the per-instance override store on
//! `AcpAdapter`.
//!
//! The catalog is the single global source. Profiles reference entries
//! by `name` (validated at config load); per-instance overrides
//! filter the visible subset for one running instance without
//! mutating the catalog itself.
//!
//! Vendor-specific MCP injection at agent spawn lands incrementally
//! per agent — the wire surface (catalog read + per-instance set +
//! restart hook) is what this module owns.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use serde::Serialize;
use tokio::sync::broadcast;
use tracing::info;

pub use crate::config::MCPDefinition;

const MCPS_BROADCAST_CAPACITY: usize = 32;

/// Broadcast event published every time the catalog reloads.
/// Consumers must handle `RecvError::Lagged` — same contract as
/// `SkillsBroadcast` and the adapter registry.
#[derive(Debug, Clone, Serialize)]
pub struct MCPsChanged {
    pub count: usize,
}

#[derive(Debug)]
pub struct MCPsBroadcast {
    sender: broadcast::Sender<MCPsChanged>,
}

impl MCPsBroadcast {
    #[must_use]
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(MCPS_BROADCAST_CAPACITY);
        Self { sender }
    }

    #[cfg(test)]
    pub(crate) fn from_sender(sender: broadcast::Sender<MCPsChanged>) -> Self {
        Self { sender }
    }

    #[allow(dead_code)]
    pub fn publish(&self, evt: MCPsChanged) {
        if let Err(err) = self.sender.send(evt) {
            tracing::trace!(%err, "mcps broadcast: no active subscribers");
        }
    }

    /// Subscribe to catalog reload events. Future socket subscribe
    /// path consumes this; every consumer MUST handle
    /// `RecvError::Lagged`.
    #[allow(dead_code)]
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<MCPsChanged> {
        self.sender.subscribe()
    }
}

impl Default for MCPsBroadcast {
    fn default() -> Self {
        Self::new()
    }
}

/// Owned MCP catalog. Constructed once at daemon boot from
/// `Config.mcps`; `reload` swaps the in-memory map. Tracks insertion
/// order for stable `list` output.
pub struct MCPsRegistry {
    catalog: RwLock<HashMap<String, MCPDefinition>>,
    order: RwLock<Vec<String>>,
    #[allow(dead_code)]
    broadcast: Arc<MCPsBroadcast>,
}

impl MCPsRegistry {
    /// Build a registry from a catalog snapshot. Caller is the daemon
    /// boot path; tests construct directly.
    #[must_use]
    pub fn new(defs: Vec<MCPDefinition>, broadcast: Arc<MCPsBroadcast>) -> Self {
        let mut order = Vec::with_capacity(defs.len());
        let mut catalog = HashMap::with_capacity(defs.len());
        for d in defs {
            order.push(d.name.clone());
            catalog.insert(d.name.clone(), d);
        }
        Self {
            catalog: RwLock::new(catalog),
            order: RwLock::new(order),
            broadcast,
        }
    }

    /// Snapshot of every catalog entry, sorted by insertion order.
    #[must_use]
    pub fn list(&self) -> Vec<MCPDefinition> {
        let catalog = self.catalog.read().expect("mcps catalog lock poisoned");
        let order = self.order.read().expect("mcps order lock poisoned");
        order.iter().filter_map(|name| catalog.get(name).cloned()).collect()
    }

    /// Lookup by name. Returns an owned clone so callers don't hold
    /// the read lock across their work.
    #[allow(dead_code)]
    #[must_use]
    pub fn get(&self, name: &str) -> Option<MCPDefinition> {
        let catalog = self.catalog.read().expect("mcps catalog lock poisoned");
        catalog.get(name).cloned()
    }

    /// Replace the in-memory catalog with `defs`. Future
    /// `daemon/reload` (K-279) hits this. Publishes a `MCPsChanged`
    /// event on success.
    #[allow(dead_code)]
    pub fn reload(&self, defs: Vec<MCPDefinition>) {
        let mut new_order = Vec::with_capacity(defs.len());
        let mut new_catalog = HashMap::with_capacity(defs.len());
        for d in defs {
            new_order.push(d.name.clone());
            new_catalog.insert(d.name.clone(), d);
        }
        let count = new_catalog.len();
        {
            let mut catalog = self.catalog.write().expect("mcps catalog lock poisoned");
            let mut order = self.order.write().expect("mcps order lock poisoned");
            *catalog = new_catalog;
            *order = new_order;
        }
        info!(count, "mcps registry: reloaded");
        self.broadcast.publish(MCPsChanged { count });
    }

    #[cfg(test)]
    #[must_use]
    pub fn count(&self) -> usize {
        self.catalog.read().expect("mcps catalog lock poisoned").len()
    }
}

impl std::fmt::Debug for MCPsRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MCPsRegistry").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(name: &str, command: &str) -> MCPDefinition {
        MCPDefinition {
            name: name.to_string(),
            command: command.to_string(),
            args: Vec::new(),
            env: Default::default(),
            scope: None,
        }
    }

    fn build_registry(defs: Vec<MCPDefinition>) -> (Arc<MCPsRegistry>, broadcast::Receiver<MCPsChanged>) {
        let (tx, rx) = broadcast::channel::<MCPsChanged>(8);
        let broadcast = Arc::new(MCPsBroadcast::from_sender(tx));
        let reg = Arc::new(MCPsRegistry::new(defs, broadcast));
        (reg, rx)
    }

    #[test]
    fn list_preserves_insertion_order() {
        let (reg, _rx) = build_registry(vec![def("alpha", "a"), def("beta", "b"), def("gamma", "c")]);
        let names: Vec<String> = reg.list().into_iter().map(|m| m.name).collect();
        assert_eq!(names, vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn get_returns_some_for_known_and_none_for_unknown() {
        let (reg, _rx) = build_registry(vec![def("known", "k")]);
        assert_eq!(reg.get("known").map(|d| d.command), Some("k".into()));
        assert!(reg.get("missing").is_none());
    }

    #[test]
    fn reload_replaces_catalog_and_emits_event() {
        let (reg, mut rx) = build_registry(vec![def("a", "x"), def("b", "y")]);
        assert_eq!(reg.count(), 2);
        reg.reload(vec![def("c", "z")]);
        assert_eq!(reg.count(), 1);
        assert!(reg.get("a").is_none());
        assert_eq!(reg.get("c").map(|d| d.command), Some("z".into()));
        let evt = rx.try_recv().expect("event fired");
        assert_eq!(evt.count, 1);
    }

    #[test]
    fn empty_catalog_is_valid() {
        let (reg, _rx) = build_registry(Vec::new());
        assert_eq!(reg.count(), 0);
        assert!(reg.list().is_empty());
        assert!(reg.get("anything").is_none());
    }
}

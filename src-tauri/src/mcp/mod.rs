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
//! restart hook) is what this module owns. Catalog state is static
//! after daemon boot — restart-to-reconfigure, no live propagation.

use std::collections::{BTreeMap, HashMap};
use std::sync::RwLock;

use garde::Validate;
use serde::{Deserialize, Serialize};

/// One `[[mcps]]` entry in the global catalog. `name` is the dedup
/// key + the value referenced from `profile.mcps`. `command` / `args`
/// / `env` describe the MCP server subprocess to spawn; `scope` is a
/// coarse classifier for UI grouping (non-load-bearing today).
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Validate)]
#[serde(deny_unknown_fields)]
pub struct MCPDefinition {
    #[garde(length(min = 1))]
    pub name: String,
    #[garde(length(min = 1))]
    pub command: String,
    #[garde(skip)]
    #[serde(default)]
    pub args: Vec<String>,
    #[garde(skip)]
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[garde(skip)]
    #[serde(default)]
    pub scope: Option<String>,
}

/// Owned MCP catalog. Constructed once at daemon boot from
/// `Config.mcps`. Tracks insertion order for stable `list` output.
pub struct MCPsRegistry {
    catalog: RwLock<HashMap<String, MCPDefinition>>,
    order: RwLock<Vec<String>>,
}

impl MCPsRegistry {
    /// Build a registry from a catalog snapshot. Caller is the daemon
    /// boot path; tests construct directly.
    #[must_use]
    pub fn new(defs: Vec<MCPDefinition>) -> Self {
        let mut order = Vec::with_capacity(defs.len());
        let mut catalog = HashMap::with_capacity(defs.len());
        for d in defs {
            order.push(d.name.clone());
            catalog.insert(d.name.clone(), d);
        }
        Self {
            catalog: RwLock::new(catalog),
            order: RwLock::new(order),
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
    use std::sync::Arc;

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

    fn build_registry(defs: Vec<MCPDefinition>) -> Arc<MCPsRegistry> {
        Arc::new(MCPsRegistry::new(defs))
    }

    #[test]
    fn list_preserves_insertion_order() {
        let reg = build_registry(vec![def("alpha", "a"), def("beta", "b"), def("gamma", "c")]);
        let names: Vec<String> = reg.list().into_iter().map(|m| m.name).collect();
        assert_eq!(names, vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn get_returns_some_for_known_and_none_for_unknown() {
        let reg = build_registry(vec![def("known", "k")]);
        assert_eq!(reg.get("known").map(|d| d.command), Some("k".into()));
        assert!(reg.get("missing").is_none());
    }

    #[test]
    fn empty_catalog_is_valid() {
        let reg = build_registry(Vec::new());
        assert_eq!(reg.count(), 0);
        assert!(reg.list().is_empty());
        assert!(reg.get("anything").is_none());
    }
}

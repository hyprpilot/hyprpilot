//! MCP server registry — JSON-file based.
//!
//! Captain's MCP config lives in JSON files under top-level `mcps`
//! (global, e.g. `mcps = ["~/.config/hyprpilot/mcps/base.json"]`) or
//! per-profile `mcps` (wholesale-replaces global). Each file follows
//! the standard `{ "mcpServers": { "name": { command, args, env, ... } } }`
//! shape used by Claude Code / Codex / Cursor / every MCP client. Drop
//! `~/.claude.json` straight in and it Just Works.
//!
//! hyprpilot extends the spec via a per-server `hyprpilot` namespace
//! key carrying our own fields (auto-accept / auto-reject tool globs
//! today; future fields slot in alongside without spec collision).
//! Everything else in the entry stays as opaque `serde_json::Value` —
//! daemon never inspects `command` / `args` / `env` / `url` / future
//! spec additions; they ride through to the agent verbatim at
//! `session/new` injection time.
//!
//! Resolution: profile's `mcps` (when set) wholesale-replaces the
//! global default. Within a file set, later files override same-name
//! entries (`work.json` shipping a personal `github` token after
//! `base.json`). One malformed file warns + skips — doesn't abort
//! daemon boot.

pub mod loader;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::RwLock;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// hyprpilot-namespace fields under each `mcpServers` entry. CamelCase
/// to match the surrounding `mcpServers` JSON style.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct HyprpilotExtension {
    /// Glob patterns matching tool names; matches auto-resolve as
    /// "allow once" through the permission controller.
    pub auto_accept_tools: Vec<String>,
    /// Glob patterns matching tool names; matches auto-resolve as
    /// "deny once". Reject beats accept on overlap.
    pub auto_reject_tools: Vec<String>,
}

/// One server entry. `name` is the `mcpServers` map key (used for
/// indexing + UI labels + tool→server attribution). `raw` carries the
/// untouched server entry minus the hyprpilot extension key — gets
/// projected onto `agent_client_protocol::schema::McpServer` at
/// `session/new` injection time. `hyprpilot` is the only typed slice;
/// everything else stays opaque so future MCP-spec additions ride
/// through without a hyprpilot release.
#[derive(Debug, Clone)]
pub struct MCPDefinition {
    pub name: String,
    pub raw: Value,
    pub hyprpilot: HyprpilotExtension,
    /// Source file the entry came from. UI surfaces this so the
    /// captain can trace "which file owns this server" without
    /// leaving the overlay.
    pub source: PathBuf,
}

/// Owned MCP catalog — the resolved set after merging every file.
/// Constructed at daemon boot from the global `mcps` paths. Profiles
/// with their own `mcps` field build per-profile registries on
/// demand at instance spawn time (the resolver in `loader.rs`).
pub struct MCPsRegistry {
    /// Resolved name → definition map. Order tracked separately so
    /// `list()` is stable.
    catalog: RwLock<HashMap<String, MCPDefinition>>,
    order: RwLock<Vec<String>>,
}

/// Project an opaque `MCPDefinition.raw` JSON value onto the ACP wire
/// shape. The `mcpServers` JSON spec encodes transport via field
/// presence (`command` → stdio, `url` + optional `transport` →
/// http/sse); ACP's typed `McpServer` enum carries the same three
/// variants. Returns `None` when the entry doesn't match any known
/// transport — daemon logs + skips so a malformed entry doesn't
/// brick session/new.
#[must_use]
pub fn project_to_acp(def: &MCPDefinition) -> Option<agent_client_protocol::schema::McpServer> {
    use agent_client_protocol::schema::{
        EnvVariable, HttpHeader, McpServer, McpServerHttp, McpServerSse, McpServerStdio,
    };
    let obj = def.raw.as_object()?;

    // Stdio: presence of `command` is the discriminator. Standard
    // `mcpServers` JSON shape.
    if let Some(command_v) = obj.get("command") {
        let command_str = command_v.as_str()?;
        let args: Vec<String> = obj
            .get("args")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        let env: Vec<EnvVariable> = obj
            .get("env")
            .and_then(|v| v.as_object())
            .map(|map| {
                map.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| EnvVariable::new(k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        let mut stdio = McpServerStdio::new(def.name.clone(), std::path::PathBuf::from(command_str));
        stdio.args = args;
        stdio.env = env;
        return Some(McpServer::Stdio(stdio));
    }

    // HTTP / SSE: `url` is the discriminator; `type` (when present)
    // distinguishes between them. `transport` is also accepted as an
    // alias used by some vendor configs.
    if let Some(url_v) = obj.get("url") {
        let url_str = url_v.as_str()?;
        let kind = obj
            .get("type")
            .or_else(|| obj.get("transport"))
            .and_then(|v| v.as_str())
            .unwrap_or("http");
        let headers: Vec<HttpHeader> = obj
            .get("headers")
            .and_then(|v| v.as_object())
            .map(|map| {
                map.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| HttpHeader::new(k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        if kind.eq_ignore_ascii_case("sse") {
            let mut sse = McpServerSse::new(def.name.clone(), url_str);
            sse.headers = headers;
            return Some(McpServer::Sse(sse));
        }
        let mut http = McpServerHttp::new(def.name.clone(), url_str);
        http.headers = headers;
        return Some(McpServer::Http(http));
    }

    None
}

impl MCPsRegistry {
    /// Construct from a pre-resolved set. Caller (`loader::load_files`)
    /// has already merged + warned on bad files.
    #[must_use]
    pub fn new(defs: Vec<MCPDefinition>) -> Self {
        let mut order = Vec::with_capacity(defs.len());
        let mut catalog = HashMap::with_capacity(defs.len());
        for d in defs {
            // Later-wins on collision: `loader::load_files` already
            // applies the file-iteration order, so by the time we get
            // here the resolved set is collision-free. The
            // contains_key guard is defensive — a bug in the loader
            // would drop the duplicate silently otherwise.
            order.retain(|n: &String| n.as_str() != d.name);
            order.push(d.name.clone());
            catalog.insert(d.name.clone(), d);
        }
        Self {
            catalog: RwLock::new(catalog),
            order: RwLock::new(order),
        }
    }

    #[must_use]
    pub fn list(&self) -> Vec<MCPDefinition> {
        let catalog = self.catalog.read().expect("mcps catalog lock poisoned");
        let order = self.order.read().expect("mcps order lock poisoned");
        order.iter().filter_map(|name| catalog.get(name).cloned()).collect()
    }

    #[allow(dead_code)]
    #[must_use]
    pub fn get(&self, name: &str) -> Option<MCPDefinition> {
        let catalog = self.catalog.read().expect("mcps catalog lock poisoned");
        catalog.get(name).cloned()
    }

    /// Project every entry onto its ACP `McpServer` typed shape, ready
    /// for `NewSessionRequest::mcp_servers` / `LoadSessionRequest::mcp_servers`.
    /// Skips entries that don't match a known transport (`command` for
    /// stdio, `url` for http/sse) — a `warn!` with the offending name
    /// records the drop. Order tracks `list()`.
    #[must_use]
    pub fn to_acp_servers(&self) -> Vec<agent_client_protocol::schema::McpServer> {
        self.list()
            .into_iter()
            .filter_map(|def| match project_to_acp(&def) {
                Some(server) => Some(server),
                None => {
                    tracing::warn!(
                        name = %def.name,
                        source = %def.source.display(),
                        "mcp::to_acp_servers: skipping entry — no `command` or `url` field"
                    );
                    None
                }
            })
            .collect()
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

    fn def(name: &str, source: &str, hyprpilot: HyprpilotExtension) -> MCPDefinition {
        MCPDefinition {
            name: name.to_string(),
            raw: serde_json::json!({ "command": "echo", "args": [name] }),
            hyprpilot,
            source: PathBuf::from(source),
        }
    }

    fn build_registry(defs: Vec<MCPDefinition>) -> Arc<MCPsRegistry> {
        Arc::new(MCPsRegistry::new(defs))
    }

    #[test]
    fn list_preserves_insertion_order() {
        let reg = build_registry(vec![
            def("alpha", "a.json", HyprpilotExtension::default()),
            def("beta", "a.json", HyprpilotExtension::default()),
            def("gamma", "b.json", HyprpilotExtension::default()),
        ]);
        let names: Vec<String> = reg.list().into_iter().map(|m| m.name).collect();
        assert_eq!(names, vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn empty_catalog_is_valid() {
        let reg = build_registry(Vec::new());
        assert_eq!(reg.count(), 0);
        assert!(reg.list().is_empty());
        assert!(reg.get("anything").is_none());
    }
}

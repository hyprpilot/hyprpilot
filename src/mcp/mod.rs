//! MCP server registry — JSON-file based.
//!
//! Captain's MCP config lives in JSON files listed under a profile's
//! `mcps` (or shared across profiles via a root `[[patches]]` entry).
//! Each file follows the standard
//! `{ "mcpServers": { "name": { command, args, env, ... } } }`
//! shape used by Claude Code / Codex / Cursor / every MCP client. Drop
//! `~/.claude.json` straight in and it Just Works.
//!
//! hyprpilot extends the spec via a per-server `hyprpilot` namespace
//! key carrying our own fields (tool include / exclude and
//! auto-accept / auto-reject tool globs today; future fields slot in
//! alongside without spec collision).
//! Everything else in the entry stays as opaque `serde_json::Value` —
//! the launcher never inspects `command` / `args` / `env` / `url` /
//! future spec additions; they ride through to the vendor verbatim
//! when the catalog is projected onto the native CLI.
//!
//! Resolution: a profile's `mcps` (when set) wholesale-replaces any
//! patch-provided catalog. Within a file set, later files override
//! same-name entries (`work.json` shipping a personal `github` token
//! after `base.json`). One malformed file warns + skips — doesn't
//! abort the launch.

pub mod auto_inject;
pub mod loader;
pub mod server;
pub mod transport;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::RwLock;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use transport::McpTransport;

/// hyprpilot-namespace fields under each `mcpServers` entry. CamelCase
/// to match the surrounding `mcpServers` JSON style.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct HyprpilotExtension {
    /// Optional glob allow-list for tools from this server. `None`
    /// means no visibility filter; `Some([])` is an explicit
    /// "no tools" filter for providers that can express it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_tools: Option<Vec<String>>,
    /// Glob deny-list for tools from this server. Exclude beats
    /// include on overlap.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub exclude_tools: Vec<String>,
    /// Glob patterns matching tool names; matches auto-resolve as
    /// "allow once" through the permission controller.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub auto_accept_tools: Vec<String>,
    /// Glob patterns matching tool names; matches auto-resolve as
    /// "deny once". Reject beats accept on overlap.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub auto_reject_tools: Vec<String>,
}

impl HyprpilotExtension {
    #[must_use]
    pub fn has_tool_policy(&self) -> bool {
        self.include_tools.is_some()
            || !self.exclude_tools.is_empty()
            || !self.auto_accept_tools.is_empty()
            || !self.auto_reject_tools.is_empty()
    }
}

/// One server entry. `name` is the `mcpServers` map key (used for
/// indexing + UI labels + tool→server attribution). `raw` carries the
/// untouched server entry minus the hyprpilot extension key — gets
/// projected onto `McpTransport` at vendor-native config build time
/// (`spawn::providers`). `hyprpilot` is the only typed slice;
/// everything else stays opaque so future MCP-spec additions ride
/// through without a hyprpilot release.
#[derive(Debug, Clone)]
pub struct MCPDefinition {
    pub name: String,
    pub raw: Value,
    pub hyprpilot: HyprpilotExtension,
    /// Source file the entry came from. Retained for diagnostics; not
    /// read on the launcher exec path.
    #[allow(dead_code)]
    pub source: PathBuf,
}

fn expand_value_with<F>(raw: &str, ctx: &str, lookup: &mut F) -> String
where
    F: FnMut(&str) -> Option<String>,
{
    let tilde = shellexpand::tilde(raw);
    match shellexpand::env_with_context(tilde.as_ref(), |name| {
        Ok::<Option<String>, std::convert::Infallible>(lookup_env(name, lookup))
    }) {
        Ok(expanded) => expanded.into_owned(),
        Err(err) => {
            tracing::warn!(value = raw, ctx, %err, "mcp::expand_value: expansion failed; using raw value");
            raw.to_string()
        }
    }
}

fn lookup_env<F>(name: &str, lookup: &mut F) -> Option<String>
where
    F: FnMut(&str) -> Option<String>,
{
    if let Some(value) = lookup(name) {
        return Some(value);
    }
    name.strip_prefix("env:").and_then(lookup)
}

fn expand_raw_strings_with<F>(def: &MCPDefinition, raw: &Value, lookup: &mut F) -> Value
where
    F: FnMut(&str) -> Option<String>,
{
    fn expand_object_strings<F>(obj: &mut serde_json::Map<String, Value>, ctx: &str, lookup: &mut F)
    where
        F: FnMut(&str) -> Option<String>,
    {
        for (key, value) in obj.iter_mut() {
            if let Value::String(raw) = value {
                *raw = expand_value_with(raw, &format!("{ctx}.{key}"), lookup);
            }
        }
    }

    let mut expanded = raw.clone();
    let Some(obj) = expanded.as_object_mut() else {
        return expanded;
    };

    if let Some(Value::String(command)) = obj.get_mut("command") {
        *command = expand_value_with(command, &format!("mcp.{}.command", def.name), lookup);
    }

    if let Some(Value::Array(args)) = obj.get_mut("args") {
        for (idx, arg) in args.iter_mut().enumerate() {
            if let Value::String(raw) = arg {
                *raw = expand_value_with(raw, &format!("mcp.{}.args[{idx}]", def.name), lookup);
            }
        }
    }

    if let Some(Value::Object(env)) = obj.get_mut("env") {
        expand_object_strings(env, &format!("mcp.{}.env", def.name), lookup);
    }

    if let Some(Value::Object(headers)) = obj.get_mut("headers") {
        expand_object_strings(headers, &format!("mcp.{}.headers", def.name), lookup);
    }

    expanded
}

/// Owned MCP catalog — the resolved set after merging every file.
/// Built per launch from the resolved profile's `mcps` files (the
/// resolver in `loader.rs`).
pub struct MCPsRegistry {
    /// Resolved name → definition map. Order tracked separately so
    /// `list()` is stable.
    catalog: RwLock<HashMap<String, MCPDefinition>>,
    order: RwLock<Vec<String>>,
}

/// Project an opaque `MCPDefinition.raw` JSON value onto its
/// `McpTransport` shape. The `mcpServers` JSON spec encodes transport
/// via field presence (`command` → stdio, `url` + optional
/// `type`/`transport` → http/sse). Returns `None` when the entry
/// doesn't match any known transport — callers log + skip so a
/// malformed entry doesn't brick a spawn.
#[must_use]
pub fn project_transport(def: &MCPDefinition) -> Option<McpTransport> {
    project_transport_with_lookup(def, &mut |name| std::env::var(name).ok())
}

/// Return the raw server entry after applying the same `~` and
/// environment-variable expansion used by transport projection.
#[must_use]
pub fn expanded_raw(def: &MCPDefinition) -> Value {
    expand_raw_strings_with(def, &def.raw, &mut |name| std::env::var(name).ok())
}

fn project_transport_with_lookup<F>(def: &MCPDefinition, lookup: &mut F) -> Option<McpTransport>
where
    F: FnMut(&str) -> Option<String>,
{
    let expanded = expand_raw_strings_with(def, &def.raw, lookup);
    let obj = expanded.as_object()?;

    // Stdio: presence of `command` is the discriminator. Standard
    // `mcpServers` JSON shape.
    if let Some(command_v) = obj.get("command") {
        let command_str = command_v.as_str()?;
        let args: Vec<String> = obj
            .get("args")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        let env: Vec<(String, String)> = obj
            .get("env")
            .and_then(|v| v.as_object())
            .map(|map| {
                map.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        return Some(McpTransport::Stdio {
            name: def.name.clone(),
            command: PathBuf::from(command_str),
            args,
            env,
        });
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
        let headers: Vec<(String, String)> = obj
            .get("headers")
            .and_then(|v| v.as_object())
            .map(|map| {
                map.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        if kind.eq_ignore_ascii_case("sse") {
            return Some(McpTransport::Sse {
                name: def.name.clone(),
                url: url_str.to_string(),
                headers,
            });
        }
        return Some(McpTransport::Http {
            name: def.name.clone(),
            url: url_str.to_string(),
            headers,
        });
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

    /// Per-name lookup. Stays for tests + future consumers.
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
    fn hyprpilot_extension_serializes_only_declared_tool_policy_fields() {
        assert_eq!(serde_json::json!(HyprpilotExtension::default()), serde_json::json!({}));

        let ext = HyprpilotExtension {
            include_tools: Some(Vec::new()),
            exclude_tools: Vec::new(),
            auto_accept_tools: vec!["read_*".into()],
            auto_reject_tools: Vec::new(),
        };

        assert_eq!(
            serde_json::json!(ext),
            serde_json::json!({
                "includeTools": [],
                "autoAcceptTools": ["read_*"],
            })
        );
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
    fn project_transport_expands_stdio_command_args_and_env() {
        let def = MCPDefinition {
            name: "memory".into(),
            raw: serde_json::json!({
                "command": "${HYPRPILOT_TEST_MCP_BIN}",
                "args": ["--path", "${HYPRPILOT_TEST_MCP_ENV}"],
                "env": { "MEMORY_FILE_PATH": "${HYPRPILOT_TEST_MCP_ENV}/memory.jsonl" }
            }),
            hyprpilot: HyprpilotExtension::default(),
            source: PathBuf::from("test.json"),
        };

        let projected = project_transport_with_lookup(&def, &mut |name| match name {
            "HYPRPILOT_TEST_MCP_BIN" => Some("/tmp/mcp-bin".into()),
            "HYPRPILOT_TEST_MCP_ENV" => Some("expanded-env".into()),
            _ => None,
        })
        .expect("stdio projects");
        let McpTransport::Stdio { command, args, env, .. } = projected else {
            panic!("expected stdio server");
        };
        assert_eq!(command.to_string_lossy(), "/tmp/mcp-bin");
        assert_eq!(args, vec!["--path", "expanded-env"]);
        assert_eq!(env[0].1, "expanded-env/memory.jsonl");
    }

    #[test]
    fn project_transport_expands_http_headers() {
        let def = MCPDefinition {
            name: "github".into(),
            raw: serde_json::json!({
                "url": "https://example.test/mcp",
                "headers": { "Authorization": "Bearer ${HYPRPILOT_TEST_MCP_TOKEN}" }
            }),
            hyprpilot: HyprpilotExtension::default(),
            source: PathBuf::from("test.json"),
        };

        let projected = project_transport_with_lookup(&def, &mut |name| match name {
            "HYPRPILOT_TEST_MCP_TOKEN" => Some("secret-token".into()),
            _ => None,
        })
        .expect("http projects");
        let McpTransport::Http { headers, .. } = projected else {
            panic!("expected http server");
        };
        assert_eq!(headers[0].1, "Bearer secret-token");
    }

    #[test]
    fn project_transport_expands_env_prefixed_placeholders() {
        let def = MCPDefinition {
            name: "github".into(),
            raw: serde_json::json!({
                "command": "${env:HYPRPILOT_TEST_MCP_BIN}",
                "args": ["--token", "${env:HYPRPILOT_TEST_MCP_TOKEN}"],
                "env": { "TOKEN": "${env:HYPRPILOT_TEST_MCP_TOKEN}" }
            }),
            hyprpilot: HyprpilotExtension::default(),
            source: PathBuf::from("test.json"),
        };

        let projected = project_transport_with_lookup(&def, &mut |name| match name {
            "HYPRPILOT_TEST_MCP_BIN" => Some("/tmp/mcp-bin".into()),
            "HYPRPILOT_TEST_MCP_TOKEN" => Some("secret-token".into()),
            _ => None,
        })
        .expect("stdio projects");
        let McpTransport::Stdio { command, args, env, .. } = projected else {
            panic!("expected stdio server");
        };
        assert_eq!(command.to_string_lossy(), "/tmp/mcp-bin");
        assert_eq!(args, vec!["--token", "secret-token"]);
        assert_eq!(env[0].1, "secret-token");
    }

    #[test]
    fn empty_catalog_is_valid() {
        let reg = build_registry(Vec::new());
        assert_eq!(reg.count(), 0);
        assert!(reg.list().is_empty());
        assert!(reg.get("anything").is_none());
    }
}

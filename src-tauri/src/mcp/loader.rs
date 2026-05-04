//! JSON file loader for MCP server config.
//!
//! Reads a list of paths, parses each as `{ "mcpServers": { ... } }`,
//! merges into a single resolved set with later-file-wins on
//! same-name collision. Pulls the `hyprpilot` extension out of each
//! entry; everything else stays as opaque `serde_json::Value` for
//! pass-through projection at ACP `session/new` time.
//!
//! Failure mode: malformed file warns and continues — one bad JSON
//! doesn't abort the whole catalog. Same warn-and-skip pattern the
//! skills loader uses.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;
use tracing::{debug, warn};

use super::{HyprpilotExtension, MCPDefinition};

/// Wrap `paths::resolve_user` so call sites read the same
/// (raw-path-in / fully-resolved-out) shape and the resolution
/// step stays consistent across mcp loader / agents / completion.
fn expand_path(path: &Path) -> PathBuf {
    crate::paths::resolve_user(&path.to_string_lossy())
}

/// Load + merge every file in `paths`. Returns the resolved
/// definition list ready to hand to `MCPsRegistry::new`. Errors are
/// per-file: a single bad file logs and is skipped; the others still
/// load. An empty list (no files supplied) returns an empty Vec.
pub fn load_files(paths: &[PathBuf]) -> Vec<MCPDefinition> {
    let mut resolved: Vec<MCPDefinition> = Vec::new();
    for raw_path in paths {
        let path = expand_path(raw_path);
        let entries = match load_one(&path) {
            Ok(entries) => entries,
            Err(err) => {
                warn!(path = %path.display(), %err, "mcp loader: skipping malformed file");
                continue;
            }
        };
        debug!(path = %path.display(), count = entries.len(), "mcp loader: file loaded");
        for entry in entries {
            // Later-wins: drop any prior definition with the same name
            // before pushing the new one.
            resolved.retain(|d: &MCPDefinition| d.name != entry.name);
            resolved.push(entry);
        }
    }
    resolved
}

fn load_one(path: &Path) -> Result<Vec<MCPDefinition>, anyhow::Error> {
    let body = fs::read_to_string(path).map_err(|e| anyhow::anyhow!("read {}: {e}", path.display()))?;
    let parsed: McpFile = serde_json::from_str(&body).map_err(|e| anyhow::anyhow!("parse {}: {e}", path.display()))?;

    let mut out = Vec::with_capacity(parsed.mcp_servers.len());
    for (name, mut raw) in parsed.mcp_servers {
        if name.is_empty() {
            warn!(path = %path.display(), "mcp loader: server entry with empty name — skipping");
            continue;
        }
        // Pull `hyprpilot` out of the raw entry (if present) so the
        // pass-through projection at session/new time doesn't ship
        // our extension key to the agent.
        let hyprpilot: HyprpilotExtension = match raw.as_object_mut() {
            Some(obj) => match obj.remove("hyprpilot") {
                Some(value) => serde_json::from_value(value).unwrap_or_else(|err| {
                    warn!(
                        path = %path.display(),
                        server = %name,
                        %err,
                        "mcp loader: server has malformed `hyprpilot` extension — defaulting"
                    );
                    HyprpilotExtension::default()
                }),
                None => HyprpilotExtension::default(),
            },
            None => {
                warn!(path = %path.display(), server = %name, "mcp loader: server entry is not an object — skipping");
                continue;
            }
        };
        out.push(MCPDefinition {
            name,
            raw,
            hyprpilot,
            source: path.to_path_buf(),
        });
    }
    Ok(out)
}

/// Wire-shape for the MCP config file. Strictly speaking the standard
/// allows other top-level keys; we ignore them. `mcpServers` is the
/// only key we read.
#[derive(Debug, Deserialize)]
struct McpFile {
    #[serde(default, rename = "mcpServers")]
    mcp_servers: serde_json::Map<String, Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(dir: &TempDir, name: &str, body: &str) -> PathBuf {
        let path = dir.path().join(name);
        fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn loads_single_file_with_extension() {
        let dir = TempDir::new().unwrap();
        let path = write(
            &dir,
            "base.json",
            r#"{
                "mcpServers": {
                    "filesystem": {
                        "command": "npx",
                        "args": ["-y", "fs"],
                        "env": { "ROOT": "/tmp" },
                        "hyprpilot": {
                            "autoAcceptTools": ["read_*"],
                            "autoRejectTools": ["delete_*"]
                        }
                    }
                }
            }"#,
        );
        let defs = load_files(std::slice::from_ref(&path));
        assert_eq!(defs.len(), 1);
        let d = &defs[0];
        assert_eq!(d.name, "filesystem");
        assert_eq!(d.source, path);
        assert_eq!(d.hyprpilot.auto_accept_tools, vec!["read_*"]);
        assert_eq!(d.hyprpilot.auto_reject_tools, vec!["delete_*"]);
        // hyprpilot key stripped from raw — agent never sees it.
        assert!(d.raw.get("hyprpilot").is_none());
        // Spec fields preserved.
        assert_eq!(d.raw.get("command").and_then(|v| v.as_str()), Some("npx"));
    }

    #[test]
    fn later_file_wins_on_same_name() {
        let dir = TempDir::new().unwrap();
        let base = write(
            &dir,
            "base.json",
            r#"{ "mcpServers": { "github": { "command": "uvx", "args": ["base"] } } }"#,
        );
        let personal = write(
            &dir,
            "personal.json",
            r#"{ "mcpServers": { "github": { "command": "uvx", "args": ["personal"] } } }"#,
        );
        let defs = load_files(&[base, personal.clone()]);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "github");
        assert_eq!(defs[0].source, personal);
        assert_eq!(
            defs[0]
                .raw
                .get("args")
                .and_then(|v| v.as_array())
                .and_then(|a| a.first()),
            Some(&Value::String("personal".to_string()))
        );
    }

    #[test]
    fn malformed_file_warns_and_skips() {
        let dir = TempDir::new().unwrap();
        let bad = write(&dir, "bad.json", "{ not valid json");
        let good = write(
            &dir,
            "good.json",
            r#"{ "mcpServers": { "alpha": { "command": "echo" } } }"#,
        );
        let defs = load_files(&[bad, good]);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "alpha");
    }

    #[test]
    fn missing_file_warns_and_skips() {
        let defs = load_files(&[PathBuf::from("/nonexistent/path/foo.json")]);
        assert!(defs.is_empty());
    }

    #[test]
    fn server_without_hyprpilot_extension_defaults_to_empty() {
        let dir = TempDir::new().unwrap();
        let path = write(
            &dir,
            "base.json",
            r#"{ "mcpServers": { "alpha": { "command": "echo" } } }"#,
        );
        let defs = load_files(&[path]);
        assert!(defs[0].hyprpilot.auto_accept_tools.is_empty());
        assert!(defs[0].hyprpilot.auto_reject_tools.is_empty());
    }

    #[test]
    fn empty_path_list_returns_empty() {
        let defs = load_files(&[]);
        assert!(defs.is_empty());
    }
}

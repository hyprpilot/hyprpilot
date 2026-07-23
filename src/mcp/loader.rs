//! JSON-shape loader for MCP server config.
//!
//! Sources are heterogeneous: each entry is either a path to a
//! standard `{ "mcpServers": { ... } }` JSON file OR an inline
//! pre-parsed map of the same payload (`mcp_servers = {...}` in the
//! hyprpilot config). Both flow through one server-extraction helper
//! so the `hyprpilot` extension pull + same-name later-wins merge are
//! identical across sources.
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
use crate::config::{ResolvedMcpFile, ResolvedMcpSource};

/// Display-source label for inline entries. Used purely in trace
/// logs + the `MCPDefinition.source` field — never read as a real
/// fs path, so the angle-bracket sentinel is safe.
const INLINE_SOURCE_LABEL: &str = "<inline>";

/// Load + merge every entry in `entries`. Returns the resolved,
/// collision-free `MCPDefinition` list the spawn path projects onto
/// the vendor CLI (via `resolve::build_mcp_registry_with`). Per-entry
/// `ignore` glob (when present) filters the loaded servers by name
/// before merging. Errors are per-entry: a single bad file logs and
/// is skipped; the others still load. Inline entries skip the fs
/// round-trip. Empty input returns an empty Vec.
pub fn load_files(entries: &[ResolvedMcpFile]) -> Vec<MCPDefinition> {
    let mut resolved: Vec<MCPDefinition> = Vec::new();
    for entry in entries {
        let (loaded, source_label) = match &entry.source {
            ResolvedMcpSource::File(path) => match load_one_file(path) {
                Ok(loaded) => (loaded, path.display().to_string()),
                Err(err) => {
                    warn!(path = %path.display(), %err, "mcp loader: skipping malformed file");
                    continue;
                }
            },
            ResolvedMcpSource::Inline(map) => {
                let label = PathBuf::from(INLINE_SOURCE_LABEL);
                (extract_servers(map.clone(), &label), INLINE_SOURCE_LABEL.to_string())
            }
        };
        let kept: Vec<MCPDefinition> = match &entry.ignore {
            Some(glob) => loaded
                .into_iter()
                .filter(|d| {
                    let drop = glob.is_match(&d.name);
                    if drop {
                        debug!(
                            source = %source_label,
                            server = %d.name,
                            "mcp loader: server name matches ignore glob — skipping"
                        );
                    }
                    !drop
                })
                .collect(),
            None => loaded,
        };
        debug!(source = %source_label, count = kept.len(), "mcp loader: entry loaded");
        for def in kept {
            // Later-wins: drop any prior definition with the same name
            // before pushing the new one.
            resolved.retain(|d: &MCPDefinition| d.name != def.name);
            resolved.push(def);
        }
    }
    resolved
}

fn load_one_file(path: &Path) -> Result<Vec<MCPDefinition>, anyhow::Error> {
    let body = fs::read_to_string(path).map_err(|e| anyhow::anyhow!("read {}: {e}", path.display()))?;
    let parsed: McpFile = serde_json::from_str(&body).map_err(|e| anyhow::anyhow!("parse {}: {e}", path.display()))?;
    Ok(extract_servers(parsed.mcp_servers, path))
}

/// Project a `mcpServers` map onto a Vec of resolved `MCPDefinition`
/// records. Strips the `hyprpilot` extension key off each entry so the
/// pass-through projection onto the vendor's MCP config doesn't ship
/// our extension to the agent. Shared between file + inline paths so
/// both honour the same hyprpilot extension semantics.
fn extract_servers(servers: serde_json::Map<String, Value>, source: &Path) -> Vec<MCPDefinition> {
    let mut out = Vec::with_capacity(servers.len());
    for (name, mut raw) in servers {
        if name.is_empty() {
            warn!(source = %source.display(), "mcp loader: server entry with empty name — skipping");
            continue;
        }
        let hyprpilot: HyprpilotExtension = match raw.as_object_mut() {
            Some(obj) => match obj.remove("hyprpilot") {
                Some(value) => serde_json::from_value(value).unwrap_or_else(|err| {
                    warn!(
                        source = %source.display(),
                        server = %name,
                        %err,
                        "mcp loader: server has malformed `hyprpilot` extension — defaulting"
                    );
                    HyprpilotExtension::default()
                }),
                None => HyprpilotExtension::default(),
            },
            None => {
                warn!(source = %source.display(), server = %name, "mcp loader: server entry is not an object — skipping");
                continue;
            }
        };
        out.push(MCPDefinition {
            name,
            raw,
            hyprpilot,
            source: source.to_path_buf(),
        });
    }
    out
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
    use std::path::PathBuf;

    use super::*;
    use tempfile::TempDir;

    fn write(dir: &TempDir, name: &str, body: &str) -> PathBuf {
        let path = dir.path().join(name);
        fs::write(&path, body).unwrap();
        path
    }

    fn entry(file: PathBuf) -> ResolvedMcpFile {
        ResolvedMcpFile {
            source: ResolvedMcpSource::File(file),
            ignore: None,
        }
    }

    fn entry_with_ignore(file: PathBuf, patterns: &[&str]) -> ResolvedMcpFile {
        ResolvedMcpFile {
            source: ResolvedMcpSource::File(file),
            ignore: Some(compile_globs(patterns)),
        }
    }

    fn inline_entry(body: &str, ignore: Option<&[&str]>) -> ResolvedMcpFile {
        // The inline shape mirrors the JSON file's `mcpServers` payload —
        // parse a wrapper to reuse the same fixture body shape tests use
        // for files, so file + inline cases stay symmetrical.
        let parsed: serde_json::Value = serde_json::from_str(body).expect("test fixture parses");
        let map = parsed
            .get("mcpServers")
            .and_then(Value::as_object)
            .cloned()
            .expect("test fixture has mcpServers object");
        ResolvedMcpFile {
            source: ResolvedMcpSource::Inline(map),
            ignore: ignore.map(compile_globs),
        }
    }

    fn compile_globs(patterns: &[&str]) -> globset::GlobSet {
        let mut builder = globset::GlobSetBuilder::new();
        for p in patterns {
            builder.add(globset::Glob::new(p).expect("test glob compiles"));
        }
        builder.build().expect("test glob set builds")
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
                            "includeTools": ["read_*", "list_*"],
                            "excludeTools": ["delete_*"],
                            "autoAcceptTools": ["read_*"],
                            "autoRejectTools": ["delete_*"]
                        }
                    }
                }
            }"#,
        );
        let defs = load_files(&[entry(path.clone())]);
        assert_eq!(defs.len(), 1);
        let d = &defs[0];
        assert_eq!(d.name, "filesystem");
        assert_eq!(d.source, path);
        assert_eq!(
            d.hyprpilot.include_tools.as_deref(),
            Some(&["read_*".to_string(), "list_*".to_string()][..])
        );
        assert_eq!(d.hyprpilot.exclude_tools, vec!["delete_*"]);
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
        let defs = load_files(&[entry(base), entry(personal.clone())]);
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
        let defs = load_files(&[entry(bad), entry(good)]);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "alpha");
    }

    #[test]
    fn missing_file_warns_and_skips() {
        let defs = load_files(&[entry(PathBuf::from("/nonexistent/path/foo.json"))]);
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
        let defs = load_files(&[entry(path)]);
        assert!(defs[0].hyprpilot.include_tools.is_none());
        assert!(defs[0].hyprpilot.exclude_tools.is_empty());
        assert!(defs[0].hyprpilot.auto_accept_tools.is_empty());
        assert!(defs[0].hyprpilot.auto_reject_tools.is_empty());
    }

    #[test]
    fn ignore_glob_drops_matching_servers() {
        let dir = TempDir::new().unwrap();
        let path = write(
            &dir,
            "team.json",
            r#"{
                "mcpServers": {
                    "github": { "command": "echo" },
                    "linear-laravel-work": { "command": "echo" },
                    "scratch-pad": { "command": "echo" }
                }
            }"#,
        );
        let defs = load_files(&[entry_with_ignore(path, &["*-work", "scratch-*"])]);
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec!["github"]);
    }

    #[test]
    fn empty_entry_list_returns_empty() {
        let defs = load_files(&[]);
        assert!(defs.is_empty());
    }

    /// Inline shape — `mcp_servers` map on the entry, no fs read.
    /// The hyprpilot extension extraction has to run identically to
    /// the file path; assertions mirror `loads_single_file_with_extension`.
    #[test]
    fn loads_inline_with_extension() {
        let entry = inline_entry(
            r#"{
                "mcpServers": {
                    "hyprpilot-nvim": {
                        "command": "uvx",
                        "args": ["hyprpilot-nvim-mcp"],
                        "env": { "NVIM_LISTEN_ADDRESS": "/tmp/nvim.sock" },
                        "hyprpilot": {
                            "autoAcceptTools": ["read_*"]
                        }
                    }
                }
            }"#,
            None,
        );
        let defs = load_files(&[entry]);
        assert_eq!(defs.len(), 1);
        let d = &defs[0];
        assert_eq!(d.name, "hyprpilot-nvim");
        assert_eq!(d.source, PathBuf::from("<inline>"));
        assert_eq!(d.hyprpilot.auto_accept_tools, vec!["read_*"]);
        assert!(d.raw.get("hyprpilot").is_none(), "hyprpilot key must be stripped");
        assert_eq!(d.raw.get("command").and_then(Value::as_str), Some("uvx"));
    }

    /// Later-wins merge spans across source kinds — a file entry
    /// followed by an inline entry naming the same server should
    /// land with the inline definition winning. Pins the captain's
    /// `--with-config` workflow: a base file under `[[mcps]]` +
    /// inline override at spawn time.
    #[test]
    fn inline_overrides_earlier_file_with_same_name() {
        let dir = TempDir::new().unwrap();
        let base = write(
            &dir,
            "base.json",
            r#"{ "mcpServers": { "shared": { "command": "echo", "args": ["from-file"] } } }"#,
        );
        let inline = inline_entry(
            r#"{ "mcpServers": { "shared": { "command": "echo", "args": ["from-inline"] } } }"#,
            None,
        );
        let defs = load_files(&[entry(base), inline]);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "shared");
        assert_eq!(defs[0].source, PathBuf::from("<inline>"));
        assert_eq!(
            defs[0]
                .raw
                .get("args")
                .and_then(Value::as_array)
                .and_then(|a| a.first()),
            Some(&Value::String("from-inline".to_string()))
        );
    }

    /// `ignore` glob applies to inline servers too — same matcher,
    /// anchored against the server name. The dropped server never
    /// reaches the registry.
    #[test]
    fn inline_ignore_glob_drops_matching_servers() {
        let entry = inline_entry(
            r#"{
                "mcpServers": {
                    "github": { "command": "echo" },
                    "scratch-pad": { "command": "echo" }
                }
            }"#,
            Some(&["scratch-*"]),
        );
        let defs = load_files(&[entry]);
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec!["github"]);
    }
}

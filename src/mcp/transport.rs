//! Local MCP server transport shape.
//!
//! Mirrors the three variants the `mcpServers` JSON spec expresses via
//! field presence (`command` -> stdio; `url` + optional `type` /
//! `transport` -> http/sse). Replaces the ACP crate's `McpServer` /
//! `HttpHeader` schema types (K-731) — the ACP wire runtime is gone
//! (K-727) and the only remaining consumer is
//! `spawn::providers`'s vendor-native config projection, which
//! never needed the full upstream schema.

use std::path::PathBuf;

/// One MCP server entry projected onto its wire transport. `name` is
/// the `mcpServers` map key; env / header pairs preserve declaration
/// order (mirrors the source JSON object's insertion order).
#[derive(Debug, Clone, PartialEq)]
pub enum McpTransport {
    Stdio {
        name: String,
        command: PathBuf,
        args: Vec<String>,
        env: Vec<(String, String)>,
    },
    Http {
        name: String,
        url: String,
        headers: Vec<(String, String)>,
    },
    Sse {
        name: String,
        url: String,
        headers: Vec<(String, String)>,
    },
}

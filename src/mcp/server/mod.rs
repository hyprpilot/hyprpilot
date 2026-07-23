//! `hyprpilot mcp …` — in-tree MCP server subcommands.
//!
//! Today only `serve` is wired; future subcommands (e.g. `inspect`,
//! `validate`) slot in alongside under the same `mcp` parent. The
//! single server (`HyprpilotServer`) ships skills today and grows new
//! features (workspace introspection, codebase tooling, …) alongside
//! without minting another subcommand.
//!
//! Spawned by the agent vendor as a stdio child via the MCP catalog
//! entry the launcher auto-injects. Lifetime is owned by the vendor —
//! the sidecar dies when the vendor session ends.

use clap::{Args, Subcommand};

pub mod serve;
pub mod skills;

/// Top-level args for `hyprpilot mcp <subcommand>`.
#[derive(Debug, Args)]
pub struct McpArgs {
    #[command(subcommand)]
    pub command: McpSubcommand,
}

/// Closed set of `hyprpilot mcp …` subcommands. Today: `serve` runs
/// the in-tree MCP server over stdio.
#[derive(Debug, Subcommand)]
pub enum McpSubcommand {
    /// Run the hyprpilot in-tree MCP server over stdio. The agent
    /// vendor spawns this when the launcher auto-injects the
    /// `hyprpilot` entry into its MCP catalog. Args (resolved skill
    /// roots, …) are passed by the launcher at spawn time.
    Serve(serve::ServeArgs),
}

impl McpArgs {
    /// Dispatch the requested subcommand. Runs the stdio MCP server in
    /// the foreground; exits when the vendor closes the pipe.
    pub async fn run(self) -> anyhow::Result<()> {
        match self.command {
            McpSubcommand::Serve(args) => serve::run(args).await,
        }
    }
}

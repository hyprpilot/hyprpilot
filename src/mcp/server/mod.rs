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

pub mod harness;
pub mod harness_server;
pub mod serve;
pub mod sessions;
pub mod skills;

/// Top-level args for `hyprpilot mcp <subcommand>`.
#[derive(Debug, Args)]
pub struct McpArgs {
    #[command(subcommand)]
    pub command: McpSubcommand,
}

/// Closed set of `hyprpilot mcp …` subcommands — one per MCP **server**
/// hyprpilot can run over stdio.
///
/// One subcommand per server rather than one server behind a flag, so
/// the two surfaces stay independent: each gets its own process (a panic
/// under the release profile's `panic = "abort"` takes down only its
/// own), its own MCP catalog entry, and therefore its own tool-approval
/// policy — auto-accepting a skill read is a very different decision
/// from auto-accepting `spawn`.
#[derive(Debug, Subcommand)]
pub enum McpSubcommand {
    /// Serve the skills catalog. Spawned by the agent vendor when the
    /// launcher auto-injects the skills entry; resolved skill roots are
    /// passed as `--skill-dir` args at spawn time.
    Skills(serve::ServeArgs),

    /// Serve the agent harness — `list_profiles` / `spawn` /
    /// `session_send` / `session_list` / `session_read` / `session_kill`.
    ///
    /// Needs no skill roots: the harness tools do not read the catalog.
    Harness(harness_server::HarnessArgs),
}

/// Where the harness tools should load their config from.
///
/// `--config` / `--config-profile` are `global = true`, so
/// `hyprpilot mcp serve --config /x` parses — but the dispatch used to
/// drop them, silently resolving the default XDG config instead. The
/// config is NOT loaded here: the `mcp` branch deliberately skips
/// `cfg.validate()` so an invalid `[[profiles]]` list cannot kill the
/// skills sidecar the vendor respawns over stdio. Harness tools load it
/// lazily and surface a failure as a tool error instead.
#[derive(Debug, Clone, Default)]
pub struct ConfigSource {
    pub path: Option<std::path::PathBuf>,
    pub profile: Option<String>,
}

impl ConfigSource {
    /// Load + validate on demand. Called per harness tool call, so a
    /// config the captain fixes mid-session is picked up without
    /// restarting the sidecar.
    pub fn load(&self) -> anyhow::Result<crate::config::Config> {
        let cfg = crate::config::load(self.path.as_deref(), self.profile.as_deref())?;
        cfg.validate()?;

        Ok(cfg)
    }
}

impl McpArgs {
    /// Dispatch the requested subcommand. Runs the stdio MCP server in
    /// the foreground; exits when the vendor closes the pipe.
    pub async fn run(self, config: ConfigSource) -> anyhow::Result<()> {
        match self.command {
            McpSubcommand::Skills(args) => serve::run_skills(args, config).await,
            McpSubcommand::Harness(args) => harness_server::run_harness(args, config).await,
        }
    }
}

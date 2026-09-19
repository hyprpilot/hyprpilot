//! How a `hyprpilot mcp <server>` process is served — the flags all
//! three subcommands share.
//!
//! Named for SERVING, not for `crate::mcp::transport`, which is the
//! shape of a vendor CATALOG entry. Two "transport" vocabularies in one
//! crate is how the wrong one gets read.
//!
//! Stdio is the default and the only transport the launcher ever starts:
//! auto-injection is unchanged, so a hyprpilot-launched vendor keeps
//! spawning its own sidecar over a pipe. HTTP is a server the captain
//! runs by hand, for clients that are not that vendor.

use std::path::PathBuf;

use clap::{Args, ValueEnum};

/// Where the token is read from when no `--token-file` is given.
///
/// An environment variable rather than a config key: the `mcp` branch
/// deliberately never loads config (see [`super::ConfigSource`]), and a
/// token under `[mcp]` would be profile-scoped by `[profiles.mcp]` —
/// meaningless for a server started by hand. No `--token <value>` flag
/// exists for the same class of reason: argv is world-readable through
/// `/proc`, and the local processes that can read it are exactly the
/// ones an unauthenticated port is exposed to.
/// Only the HTTP transport authenticates anything, so with that
/// feature off this — and [`ServeArgs::token`] — would be dead code the
/// minimal build rejects.
#[cfg(feature = "http")]
pub const TOKEN_ENV: &str = "HYPRPILOT_MCP_TOKEN";

/// How the server talks to its clients.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum Transport {
    /// One client, one pipe, lifetime owned by the vendor that spawned
    /// it. What auto-injection writes and what every launch uses.
    #[default]
    Stdio,
    /// A listening server the captain runs and owns.
    Http,
}

impl Transport {
    /// How long a client may cache a result from this transport.
    ///
    /// The 24h default is honest only because every mutable surface
    /// fires an invalidation the client actually receives. Over HTTP it
    /// does not: MCP `2026-07-28` is stateless per SEP-2567, so a
    /// request's peer dies with its response and the two out-of-band
    /// notifiers — the skills watcher and the harness exit hook — have
    /// no peer to broadcast through. A client holding a
    /// `subscriptions/listen` stream still gets everything; one that
    /// does not would be caching for a day against a promise nothing
    /// can keep.
    pub(super) fn result_ttl_ms(self) -> u64 {
        match self {
            Self::Stdio => super::rpc::RESULT_TTL_MS,
            Self::Http => 0,
        }
    }
}

/// Flattened into all three `mcp` subcommands, so the flags cannot
/// drift apart.
#[derive(Debug, Args, Clone, Default)]
pub struct ServeArgs {
    /// How to serve: over stdio for the vendor that spawned this
    /// process, or as an HTTP server clients connect to.
    #[arg(long, value_enum, default_value_t = Transport::Stdio)]
    pub transport: Transport,

    /// Address to listen on, e.g. `127.0.0.1:7777`. Required with
    /// `--transport http`.
    ///
    /// No default on purpose: the three servers each need their own
    /// port, and an ephemeral one addresses nothing.
    #[arg(long, value_name = "ADDR", required_if_eq("transport", "http"))]
    pub listen: Option<String>,

    /// File holding the bearer token clients must present. Overrides
    /// `HYPRPILOT_MCP_TOKEN`.
    ///
    /// A file rather than a value, because a value passed here would sit
    /// in `/proc/<pid>/cmdline` for every local process to read.
    #[arg(long, value_name = "PATH")]
    pub token_file: Option<PathBuf>,

    /// Accept requests whose `Host` is not a loopback name, for a bind
    /// that is reachable off this machine.
    ///
    /// Separate from `--listen` because the two are different
    /// decisions: the address decides who can open a socket, this
    /// decides whether the server answers them. Without it a
    /// non-loopback bind accepts the connection and then refuses every
    /// request, which looks like a bug rather than a policy.
    #[arg(long)]
    pub allow_remote: bool,
}

impl ServeArgs {
    /// The bearer token clients must present, if any.
    ///
    /// `--token-file` first, then [`TOKEN_ENV`]. `None` means the
    /// server is unauthenticated — a supported configuration, not an
    /// oversight, and the reason `--allow-remote` is its own flag.
    ///
    /// A file that cannot be read is fatal: it was named precisely to
    /// turn authentication on, so degrading to open would be the one
    /// failure mode the captain was guarding against.
    #[cfg(feature = "http")]
    pub fn token(&self) -> anyhow::Result<Option<String>> {
        if let Some(path) = &self.token_file {
            let raw = std::fs::read_to_string(path)
                .map_err(|err| anyhow::anyhow!("mcp: could not read --token-file {}: {err}", path.display()))?;
            let token = raw.trim().to_string();
            if token.is_empty() {
                anyhow::bail!("mcp: --token-file {} is empty", path.display());
            }

            return Ok(Some(token));
        }

        Ok(std::env::var(TOKEN_ENV)
            .ok()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stdio_is_the_default_and_keeps_the_long_ttl() {
        assert_eq!(Transport::default(), Transport::Stdio);
        assert_eq!(Transport::Stdio.result_ttl_ms(), super::super::rpc::RESULT_TTL_MS);
    }

    /// Not a tuning choice. Over HTTP the watcher and the exit hook have
    /// no peer to broadcast through, so a non-subscribing client would
    /// cache for a day against a notification that never comes.
    #[test]
    fn http_promises_no_freshness_it_cannot_signal() {
        assert_eq!(Transport::Http.result_ttl_ms(), 0);
    }

    #[cfg(feature = "http")]
    #[test]
    fn a_token_file_beats_the_environment() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token");
        std::fs::write(&path, "  from-file\n").unwrap();

        let args = ServeArgs {
            token_file: Some(path),
            ..ServeArgs::default()
        };
        assert_eq!(args.token().unwrap().as_deref(), Some("from-file"));
    }

    /// The file was named to turn auth ON. Falling back to the
    /// environment, or to no auth at all, would silently open the server
    /// the captain was closing.
    #[cfg(feature = "http")]
    #[test]
    fn an_unreadable_token_file_is_fatal_rather_than_open() {
        let args = ServeArgs {
            token_file: Some(PathBuf::from("/nonexistent/hyprpilot-token")),
            ..ServeArgs::default()
        };
        assert!(args.token().is_err());

        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().join("empty");
        std::fs::write(&empty, "\n").unwrap();
        let args = ServeArgs {
            token_file: Some(empty),
            ..ServeArgs::default()
        };
        assert!(args.token().is_err(), "an empty file is a mistake, not `no auth`");
    }
}

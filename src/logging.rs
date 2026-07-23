use anyhow::{Context, Result};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};

/// Tracing level, shared between the `--log-level` CLI flag (via
/// `clap::ValueEnum`) and the `[logging] level` config field (via
/// `serde`). Lowercase on the wire so TOML can write
/// `level = "info"`.
#[derive(ValueEnum, Copy, Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    #[default]
    Info,
    Warn,
    Error,
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            LogLevel::Trace => "trace",
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        };
        f.write_str(s)
    }
}

/// Installs the tracing subscriber ONCE with the fully-resolved filter.
/// Always writes to stderr — both debug and release builds — so a
/// captain launching the binary by hand sees output where they expect
/// it, and the `profiles --json` path keeps stdout pure (info →
/// stderr).
///
/// The filter folds in `[logging] level` up front, so the caller must
/// load the config BEFORE calling `init` (and emit the "config loaded"
/// line AFTER, so it honours the resolved level). This replaces the old
/// early-init/late-reload dance — there is no second subscriber install.
///
/// Filter precedence, highest first: `--log-level` (`cli_level`) →
/// `RUST_LOG` → `[logging] level` (`config_level`) → the `error`
/// default. The `error` default keeps a fresh run quiet — only errors
/// surface unless a level is explicitly requested.
///
/// `file:line` tagging rides only on `debug` / `trace` — the info
/// narrative stays terse.
pub fn init(cli_level: Option<LogLevel>, config_level: Option<LogLevel>) -> Result<()> {
    let (filter, verbose) = resolve_filter(cli_level, config_level)?;

    // ANSI always on. `journalctl` / a TTY render it; a pipe or
    // log-shipper strips it — so the same build serves both.
    let layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(true)
        .with_target(true)
        .with_file(verbose)
        .with_line_number(verbose)
        .with_thread_ids(false)
        .with_thread_names(false);

    tracing_subscriber::registry()
        .with(filter)
        .with(layer)
        .try_init()
        .context("failed to install tracing subscriber")?;

    Ok(())
}

/// Resolve the tracing `EnvFilter` and whether `file:line` provenance
/// rides (`verbose`), applying the precedence `--log-level` → `RUST_LOG`
/// → `[logging] level` → the `error` default.
fn resolve_filter(cli_level: Option<LogLevel>, config_level: Option<LogLevel>) -> Result<(EnvFilter, bool)> {
    if let Some(level) = cli_level {
        return Ok((filter_for(level)?, is_verbose(level)));
    }
    // RUST_LOG wins over `[logging] level`; a set-but-unparseable
    // RUST_LOG falls through to the config/default below.
    if std::env::var_os("RUST_LOG").is_some() {
        if let Ok(filter) = EnvFilter::try_from_default_env() {
            return Ok((filter, false));
        }
    }
    if let Some(level) = config_level {
        return Ok((filter_for(level)?, is_verbose(level)));
    }
    Ok((EnvFilter::new("error"), false))
}

fn filter_for(level: LogLevel) -> Result<EnvFilter> {
    EnvFilter::try_new(level.to_string()).context("failed to build log level filter")
}

fn is_verbose(level: LogLevel) -> bool {
    matches!(level, LogLevel::Debug | LogLevel::Trace)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_filter_precedence() {
        // Single test so RUST_LOG mutation stays serial even under
        // `cargo test`'s in-process threads.
        let saved = std::env::var_os("RUST_LOG");
        std::env::remove_var("RUST_LOG");

        let level_of = |cli, config| resolve_filter(cli, config).unwrap().0.to_string();

        // `--log-level` wins over RUST_LOG and config.
        std::env::set_var("RUST_LOG", "trace");
        assert_eq!(level_of(Some(LogLevel::Warn), Some(LogLevel::Debug)), "warn");

        // RUST_LOG wins over `[logging] level` when `--log-level` unset.
        assert_eq!(level_of(None, Some(LogLevel::Debug)), "trace");
        std::env::remove_var("RUST_LOG");

        // `[logging] level` applies when `--log-level` + RUST_LOG unset.
        assert_eq!(level_of(None, Some(LogLevel::Error)), "error");

        // Bare run: no source set → the quiet `error` default.
        assert_eq!(level_of(None, None), "error");

        // `--log-level` debug/trace turns on `file:line` provenance.
        assert!(resolve_filter(Some(LogLevel::Debug), None).unwrap().1);
        assert!(!resolve_filter(Some(LogLevel::Info), None).unwrap().1);

        match saved {
            Some(v) => std::env::set_var("RUST_LOG", v),
            None => std::env::remove_var("RUST_LOG"),
        }
    }
}

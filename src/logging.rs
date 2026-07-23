use anyhow::{Context, Result};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, reload, EnvFilter, Registry};

/// Handle to the installed `EnvFilter`, returned by [`init`] so the
/// filter can be swapped once — after `config::load` — to honour the
/// `[logging] level` config field. The filter lives behind a
/// [`reload::Layer`] precisely because the subscriber is installed
/// before the config is read (early enough that `config::load`'s own
/// lines are captured).
pub type LogReloadHandle = reload::Handle<EnvFilter, Registry>;

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

/// Installs the tracing subscriber and returns a [`LogReloadHandle`]
/// so the filter can be reloaded once the config is known. Always
/// writes to stderr — both debug and release builds — so a captain
/// launching the binary by hand sees output where they expect it, and
/// the `profiles --json` path keeps stdout pure (info → stderr).
///
/// Filter precedence, highest first: `--log-level` (`level` here) →
/// `RUST_LOG` → `[logging] level` (applied later via
/// [`apply_config_level`]) → the `warn,hyprpilot=info` default. That
/// default keeps third-party crates (tokio / rmcp / nucleo) at `warn`
/// while surfacing the launcher's own `info` lifecycle narrative.
///
/// `file:line` tagging rides only on `debug` / `trace` — the info
/// narrative stays terse. Captain's call; flip `verbose` to always-on
/// if the extra provenance is wanted.
pub fn init(level: Option<LogLevel>) -> Result<LogReloadHandle> {
    let filter = match level {
        Some(l) => EnvFilter::try_new(l.to_string()).context("failed to build log level filter")?,
        None => EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn,hyprpilot=info")),
    };
    let (filter, reload_handle) = reload::Layer::new(filter);

    let verbose = matches!(level, Some(LogLevel::Debug | LogLevel::Trace));

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

    Ok(reload_handle)
}

/// Reload the filter to `[logging] level` when — and only when — no
/// higher-precedence source spoke: `--log-level` unset (`cli_level`
/// is `None`) AND `RUST_LOG` unset. A no-op otherwise, so the
/// precedence `--log-level` > `RUST_LOG` > `[logging] level` holds.
pub fn apply_config_level(
    handle: &LogReloadHandle,
    cli_level: Option<LogLevel>,
    config_level: Option<LogLevel>,
) -> Result<()> {
    if cli_level.is_some() || std::env::var_os("RUST_LOG").is_some() {
        return Ok(());
    }
    let Some(level) = config_level else {
        return Ok(());
    };
    let filter = EnvFilter::try_new(level.to_string()).context("failed to build [logging].level filter")?;
    handle.reload(filter).context("failed to apply [logging].level")?;
    tracing::debug!(%level, "logging: applied [logging].level");
    Ok(())
}

#[cfg(test)]
mod tests {
    use tracing_subscriber::{reload, EnvFilter, Registry};

    use super::*;

    /// The reload `Layer` must stay alive for the handle's `Weak` to
    /// upgrade — the returned guard keeps it in scope.
    fn handle_at(initial: &str) -> (reload::Layer<EnvFilter, Registry>, LogReloadHandle) {
        reload::Layer::<EnvFilter, Registry>::new(EnvFilter::new(initial))
    }

    fn current(handle: &LogReloadHandle) -> String {
        handle.clone_current().map(|f| f.to_string()).unwrap_or_default()
    }

    #[test]
    fn apply_config_level_precedence() {
        // Single test so RUST_LOG mutation stays serial even under
        // `cargo test`'s in-process threads.
        let saved = std::env::var_os("RUST_LOG");
        std::env::remove_var("RUST_LOG");

        // Branch 4: --log-level unset, RUST_LOG unset, config set →
        // the config level is applied.
        let (_keep, handle) = handle_at("info");
        apply_config_level(&handle, None, Some(LogLevel::Debug)).unwrap();
        assert!(current(&handle).contains("debug"), "config level must apply");

        // Branch 3: config None → no-op, filter untouched.
        let (_keep, handle) = handle_at("info");
        apply_config_level(&handle, None, None).unwrap();
        assert!(current(&handle).contains("info") && !current(&handle).contains("debug"));

        // Branch 1: --log-level set wins → config ignored.
        let (_keep, handle) = handle_at("info");
        apply_config_level(&handle, Some(LogLevel::Warn), Some(LogLevel::Debug)).unwrap();
        assert!(current(&handle).contains("info") && !current(&handle).contains("debug"));

        // Branch 2: RUST_LOG set wins → config ignored.
        std::env::set_var("RUST_LOG", "trace");
        let (_keep, handle) = handle_at("info");
        apply_config_level(&handle, None, Some(LogLevel::Debug)).unwrap();
        assert!(current(&handle).contains("info") && !current(&handle).contains("debug"));

        match saved {
            Some(v) => std::env::set_var("RUST_LOG", v),
            None => std::env::remove_var("RUST_LOG"),
        }
    }
}

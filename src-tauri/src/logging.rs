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

/// Installs the tracing subscriber. Always writes to stderr — both
/// debug and release builds. systemd / journald already capture
/// stdout / stderr from the unit, and a captain running the daemon
/// in a terminal sees output where they expect it. Daily-rolled
/// files under `$XDG_STATE_HOME/hyprpilot/logs/` were duplicate
/// plumbing over what every standard service supervisor already
/// provides; removed.
///
/// The previous file-only release path also meant any captain who
/// launched the release binary by hand under `RUST_LOG=...` saw an
/// empty terminal even though traces were firing — their output was
/// silently shoved into a log file they had to know about.
pub fn init(level: Option<LogLevel>) -> Result<()> {
    // `log::Record` events (emitted by `tauri-plugin-log` on behalf of
    // the webview's `log.*` wrapper) route into this tracing subscriber
    // via `LogTracer`, auto-installed by `tracing-subscriber`'s
    // `tracing-log` feature when `try_init()` runs below — so UI and
    // backend share one sink. Do NOT call `LogTracer::init()` here;
    // it collides with that auto-install and the second setter panics
    // with "attempted to set a logger after the logging system was
    // already initialized". `tauri-plugin-log` is registered with
    // `.skip_logger()` so it doesn't fight for the same slot.
    let filter = match level {
        Some(l) => EnvFilter::try_new(l.to_string()).context("failed to build log level filter")?,
        None => EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
    };

    // ANSI on debug builds (developer terminal); off in release so
    // journald / file capture doesn't get peppered with escape codes
    // when the unit's stderr isn't a TTY. The other axes (target /
    // file / line) stay on across both builds — a captain reading
    // their journal wants the same callsite breadcrumbs the dev
    // terminal shows.
    let layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(cfg!(debug_assertions))
        .with_target(true)
        .with_file(true)
        .with_line_number(true)
        .with_thread_ids(false)
        .with_thread_names(false);

    tracing_subscriber::registry()
        .with(filter)
        .with(layer)
        .try_init()
        .context("failed to install tracing subscriber")?;

    Ok(())
}

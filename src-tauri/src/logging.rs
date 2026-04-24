use std::fs;

use anyhow::{Context, Result};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};

use crate::paths;

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

impl LogLevel {
    fn as_str(self) -> &'static str {
        match self {
            LogLevel::Trace => "trace",
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        }
    }
}

/// Installs the tracing subscriber. Stderr in debug builds, daily-rolled files
/// under `$XDG_STATE_HOME/hyprpilot/logs/` in release. The returned guard must
/// live for the duration of the program so the file writer flushes on drop.
pub fn init(level: Option<LogLevel>) -> Result<Option<WorkerGuard>> {
    // Route `log::Record` events (emitted by `tauri-plugin-log` on behalf
    // of the webview's `log.*` wrapper) into the tracing subscriber below
    // so UI and backend share one sink. Idempotent across re-inits within
    // the same process (second call returns SetLoggerError); swallow it.
    if let Err(err) = tracing_log::LogTracer::init() {
        eprintln!("LogTracer::init returned {err}; already initialized, continuing");
    }

    let filter = match level {
        Some(l) => EnvFilter::try_new(l.as_str()).context("failed to build log level filter")?,
        None => EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
    };

    if cfg!(debug_assertions) {
        tracing_subscriber::registry()
            .with(filter)
            .with(dev_fmt_layer(std::io::stderr))
            .try_init()
            .context("failed to install tracing subscriber")?;

        return Ok(None);
    }

    let log_dir = paths::log_dir();
    fs::create_dir_all(&log_dir).with_context(|| format!("failed to create log directory {}", log_dir.display()))?;

    let file_appender = tracing_appender::rolling::daily(&log_dir, "hyprpilot.log");
    let (writer, guard) = tracing_appender::non_blocking(file_appender);

    tracing_subscriber::registry()
        .with(filter)
        .with(file_fmt_layer(writer))
        .try_init()
        .context("failed to install tracing subscriber")?;

    Ok(Some(guard))
}

/// Human-readable dev formatter with colored levels, module target, and the
/// `file:line` callsite of each event for fast jump-to-source.
fn dev_fmt_layer<S, W>(writer: W) -> fmt::Layer<S, fmt::format::DefaultFields, fmt::format::Format, W>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
    W: for<'w> fmt::MakeWriter<'w> + 'static,
{
    fmt::layer()
        .with_writer(writer)
        .with_ansi(true)
        .with_target(true)
        .with_file(true)
        .with_line_number(true)
        .with_thread_ids(false)
        .with_thread_names(false)
}

/// Release file-log formatter. Same callsite info as the dev layer, ANSI
/// colors stripped (log files don't need escape codes).
fn file_fmt_layer<S, W>(writer: W) -> fmt::Layer<S, fmt::format::DefaultFields, fmt::format::Format, W>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
    W: for<'w> fmt::MakeWriter<'w> + 'static,
{
    fmt::layer()
        .with_writer(writer)
        .with_ansi(false)
        .with_target(true)
        .with_file(true)
        .with_line_number(true)
        .with_thread_ids(false)
        .with_thread_names(false)
}

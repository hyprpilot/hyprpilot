use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use directories::BaseDirs;

const APP_NAME: &str = "hyprpilot";

/// Process-lifetime XDG / known-dir base. `BaseDirs::new()` walks env
/// vars + libc for every call; cache once so the seven helpers below
/// don't re-pay the cost on every ctl invocation.
pub fn base() -> &'static BaseDirs {
    static CACHE: OnceLock<BaseDirs> = OnceLock::new();
    CACHE.get_or_init(|| BaseDirs::new().expect("unable to resolve user base directories"))
}

/// Resolved home directory. `BaseDirs::home_dir` returns a borrowed
/// path; consumers that need an owned `PathBuf` clone at the call
/// site.
pub fn home_dir() -> &'static Path {
    base().home_dir()
}

pub fn runtime_dir() -> PathBuf {
    if let Some(dir) = base().runtime_dir() {
        return dir.to_path_buf();
    }

    // XDG_RUNTIME_DIR is unset — happens in minimal containers, cron sessions,
    // some `sudo -i` contexts. Namespace the `/tmp` fallback by uid so
    // sockets/state from different users on the same box can't collide.
    // SAFETY: `getuid` is always safe to call on any Unix.
    let uid = unsafe { libc::getuid() };

    std::env::temp_dir().join(format!("{APP_NAME}-{uid}"))
}

pub fn config_dir() -> PathBuf {
    base().config_dir().join(APP_NAME)
}

pub fn state_dir() -> PathBuf {
    base()
        .state_dir()
        .map(PathBuf::from)
        .unwrap_or_else(|| base().data_local_dir().to_path_buf())
        .join(APP_NAME)
}

pub fn config_file() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn profile_config_file(name: &str) -> PathBuf {
    config_dir().join("profiles").join(format!("{name}.toml"))
}

pub fn socket_path() -> PathBuf {
    runtime_dir().join(format!("{APP_NAME}.sock"))
}

pub fn log_dir() -> PathBuf {
    state_dir().join("logs")
}

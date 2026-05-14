use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{anyhow, Result};
use directories::BaseDirs;
use path_absolutize::Absolutize;

const APP_NAME: &str = "hyprpilot";

/// Process-lifetime XDG / known-dir base. `BaseDirs::new()` walks env
/// vars + libc for every call; cache once so the seven helpers below
/// don't re-pay the cost on every ctl invocation.
pub fn base() -> &'static BaseDirs {
    static CACHE: OnceLock<BaseDirs> = OnceLock::new();
    CACHE.get_or_init(|| BaseDirs::new().expect("unable to resolve user base directories"))
}

/// Resolved home directory. Borrowed so ctl-heavy paths don't pay an
/// allocation per call — same reasoning as `base()` itself.
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

/// Extensions the daemon recognises for config-file lookups. Priority
/// order is **declaration order** — `find_config_file` returns the
/// first existing match when callers don't supply an explicit path,
/// so `.toml` is preferred when multiple files coexist after a
/// migration. Mirrors the `--with-config` extension set so a captain
/// authoring overlays in YAML can also keep their root config in
/// YAML without juggling formats.
pub const CONFIG_EXTENSIONS: &[&str] = &["toml", "json", "yaml", "yml"];

/// Search `config_dir()` for `config.{toml,json,yaml,yml}` in
/// priority order. Returns:
///
/// - `Ok(Some(path))` when exactly one exists — the canonical
///   format-agnostic resolver.
/// - `Ok(None)` when none exist — captain hasn't authored a root
///   config; the daemon falls through to defaults + profile only.
/// - `Err(anyhow)` when multiple coexist — captain has both
///   `config.toml` AND `config.yaml` (or any other ambiguous mix),
///   the daemon refuses to pick a winner silently.
pub fn find_config_file() -> Result<Option<PathBuf>> {
    let dir = config_dir();
    let matches: Vec<PathBuf> = CONFIG_EXTENSIONS
        .iter()
        .map(|ext| dir.join(format!("config.{ext}")))
        .filter(|p| p.exists())
        .collect();
    match matches.len() {
        0 => Ok(None),
        1 => Ok(Some(matches.into_iter().next().expect("len == 1"))),
        _ => Err(anyhow!(
            "multiple config files exist in {}; keep exactly one of: {}",
            dir.display(),
            matches
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// Conventional path for a named profile when authored as TOML.
/// Used in error messages + by the missing-profile path of
/// `--config-profile`; format-agnostic lookups go through
/// `find_profile_config_file`.
pub fn profile_config_file(name: &str) -> PathBuf {
    config_dir().join("profiles").join(format!("{name}.toml"))
}

/// Search `config_dir()/profiles/` for `<name>.{toml,json,yaml,yml}`
/// in priority order. Same shape as `find_config_file`:
///
/// - `Ok(Some(path))` — exactly one match.
/// - `Ok(None)` — no profile by that name.
/// - `Err(anyhow)` — multiple coexist; captain must pick.
pub fn find_profile_config_file(name: &str) -> Result<Option<PathBuf>> {
    let dir = config_dir().join("profiles");
    let matches: Vec<PathBuf> = CONFIG_EXTENSIONS
        .iter()
        .map(|ext| dir.join(format!("{name}.{ext}")))
        .filter(|p| p.exists())
        .collect();
    match matches.len() {
        0 => Ok(None),
        1 => Ok(Some(matches.into_iter().next().expect("len == 1"))),
        _ => Err(anyhow!(
            "multiple profile files exist for '{}' in {}; keep exactly one of: {}",
            name,
            dir.display(),
            matches
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

pub fn socket_path() -> PathBuf {
    runtime_dir().join(format!("{APP_NAME}.sock"))
}

/// Captain-supplied path → fully resolved `PathBuf`. Two passes:
/// (1) `shellexpand::full` for `~` + `$VAR` / `${VAR}` substitution,
/// (2) `path-absolutize` for `./` / `../` collapse + cwd-relative
/// resolution. Order matters — absolutize doesn't know `~` is special,
/// so shellexpand has to substitute first.
///
/// Both passes are best-effort: a failed `shellexpand` (undefined var,
/// broken `~`) keeps the raw string, and a failed absolutize keeps
/// whatever shellexpand produced. Captains see the bad path land
/// downstream as a "no such file" rather than spawn-time refusal.
pub fn resolve_user(s: &str) -> PathBuf {
    let expanded = shellexpand::full(s)
        .map(|s| s.into_owned())
        .unwrap_or_else(|_| s.to_owned());
    Path::new(&expanded)
        .absolutize()
        .map(|c| c.into_owned())
        .unwrap_or_else(|_| PathBuf::from(expanded))
}

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{anyhow, Result};
use directories::BaseDirs;
use path_absolutize::Absolutize;

const APP_NAME: &str = "hyprpilot";

/// Process-lifetime XDG / known-dir base. `BaseDirs::new()` walks env
/// vars + libc for every call; cache once so the seven helpers below
/// don't re-pay the cost on every launch.
pub fn base() -> &'static BaseDirs {
    static CACHE: OnceLock<BaseDirs> = OnceLock::new();
    CACHE.get_or_init(|| BaseDirs::new().expect("unable to resolve user base directories"))
}

pub fn config_dir() -> PathBuf {
    base().config_dir().join(APP_NAME)
}

/// Extensions recognised for config-file lookups. Priority
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
///   config; the loader falls through to defaults + profile only.
/// - `Err(anyhow)` when multiple coexist — captain has both
///   `config.toml` AND `config.yaml` (or any other ambiguous mix),
///   so the loader refuses to pick a winner silently.
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

/// Expand `~` and `${VAR}` / `$VAR` references in `raw`, resolving env
/// names through `lookup` (with an `env:`-prefixed fallback: a
/// `${env:FOO}` reference retries the bare `FOO`). On expansion failure
/// the value is returned verbatim after a `warn!` stamped with `ctx`.
///
/// Single source for the tilde + `env_with_context` + warn-fallback +
/// `env:`-prefix shape the launcher (`spawn::providers`, process env)
/// and the MCP catalogue (`mcp`, injected lookups) both need — the two
/// only differ in how a name resolves to a value, so `lookup` is the
/// one parameter.
pub fn expand_env_value<F>(raw: &str, ctx: &str, mut lookup: F) -> String
where
    F: FnMut(&str) -> Option<String>,
{
    let tilde = shellexpand::tilde(raw);
    match shellexpand::env_with_context(tilde.as_ref(), |name| {
        Ok::<Option<String>, std::convert::Infallible>(lookup_with_env_prefix(name, &mut lookup))
    }) {
        Ok(expanded) => expanded.into_owned(),
        Err(err) => {
            tracing::warn!(value = raw, ctx, %err, "env expansion failed; using raw value");
            raw.to_string()
        }
    }
}

fn lookup_with_env_prefix<F>(name: &str, lookup: &mut F) -> Option<String>
where
    F: FnMut(&str) -> Option<String>,
{
    if let Some(value) = lookup(name) {
        return Some(value);
    }
    name.strip_prefix("env:").and_then(lookup)
}

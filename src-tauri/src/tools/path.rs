//! Path resolution helpers shared across frontends. Pure functions
//! — every caller passes home + cwd explicitly so the same shapes
//! reach the wire (Tauri / JSON-RPC) for any UI consumer (Vue
//! overlay today, Neovim plugin tomorrow). Display-side niceties
//! (home → `~` substitution, CSS-driven truncation) stay in the
//! frontend; everything that needs OS knowledge (`$HOME`,
//! `${VAR}` interpolation, working-directory join) lives here.
//!
//! `expand_value` from `adapters::acp::agents::mod` already wraps
//! `shellexpand::full` for env values; we reuse the same crate
//! here so a captain who writes `~/proj` or `$XDG_DATA_HOME/foo`
//! into the cwd palette gets the same resolution rules across
//! the daemon.

/// `~` / `~/foo` → `<home>/foo`. Pass-through for paths that don't
/// start with the tilde sigil.
pub fn expand_tilde(path: &str, home: &str) -> String {
    if path == "~" {
        return home.to_string();
    }

    if let Some(rest) = path.strip_prefix("~/") {
        return format!("{home}/{rest}");
    }
    path.to_string()
}

/// `$VAR` / `${VAR}` → process-env value. Failure (undefined var,
/// malformed expansion) returns the input unchanged — captains
/// see the raw `$FOO` land downstream rather than a silent
/// resolution failure.
pub fn expand_env(raw: &str) -> String {
    shellexpand::env(raw)
        .map(std::borrow::Cow::into_owned)
        .unwrap_or_else(|_| raw.to_string())
}

/// Captain-typed path → absolute, resolving `~` against `home`,
/// `$VAR` / `${VAR}` against process env, and relative paths
/// against `cwd_base`. Returns `None` when the input is empty or
/// relative-with-no-base.
///
/// Order: env-expand first (so `${HOME}/proj` works), then
/// tilde-expand (so `~/proj` works regardless of whether `$HOME`
/// is also set), then resolve relative against `cwd_base`.
pub fn resolve_absolute(raw: &str, home: &str, cwd_base: Option<&str>) -> Option<String> {
    let trimmed = raw.trim();

    if trimmed.is_empty() {
        return None;
    }
    let env_expanded = expand_env(trimmed);
    let tilde_expanded = expand_tilde(&env_expanded, home);

    if tilde_expanded.starts_with('/') {
        return Some(tilde_expanded);
    }
    let base = cwd_base?.trim_end_matches('/');

    if tilde_expanded == "." {
        return Some(base.to_string());
    }

    if let Some(rest) = tilde_expanded.strip_prefix("./") {
        return Some(format!("{base}/{rest}"));
    }
    Some(format!("{base}/{tilde_expanded}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_tilde_replaces_with_home() {
        assert_eq!(expand_tilde("~/dev/x", "/home/captain"), "/home/captain/dev/x");
    }

    #[test]
    fn expand_tilde_replaces_bare_tilde() {
        assert_eq!(expand_tilde("~", "/home/captain"), "/home/captain");
    }

    #[test]
    fn expand_tilde_passes_through_non_tilde() {
        assert_eq!(expand_tilde("/etc/foo", "/home/captain"), "/etc/foo");
        assert_eq!(expand_tilde("dev/x", "/home/captain"), "dev/x");
    }

    #[test]
    fn resolve_absolute_passes_through_absolute() {
        let r = resolve_absolute("/etc/foo", "/home/captain", Some("/srv"));
        assert_eq!(r, Some("/etc/foo".to_string()));
    }

    #[test]
    fn resolve_absolute_expands_tilde() {
        let r = resolve_absolute("~/dev", "/home/captain", None);
        assert_eq!(r, Some("/home/captain/dev".to_string()));
    }

    #[test]
    fn resolve_absolute_uses_cwd_base_for_bare_relative() {
        let r = resolve_absolute("src/foo", "/home/captain", Some("/srv/proj"));
        assert_eq!(r, Some("/srv/proj/src/foo".to_string()));
    }

    #[test]
    fn resolve_absolute_uses_cwd_base_for_dot_relative() {
        let r = resolve_absolute("./foo", "/home/captain", Some("/srv/proj"));
        assert_eq!(r, Some("/srv/proj/foo".to_string()));
    }

    #[test]
    fn resolve_absolute_handles_dot() {
        let r = resolve_absolute(".", "/home/captain", Some("/srv/proj"));
        assert_eq!(r, Some("/srv/proj".to_string()));
    }

    #[test]
    fn resolve_absolute_returns_none_when_relative_without_base() {
        assert_eq!(resolve_absolute("src", "/home/captain", None), None);
    }

    #[test]
    fn resolve_absolute_returns_none_for_empty() {
        assert_eq!(resolve_absolute("   ", "/home/captain", Some("/srv")), None);
    }

    #[test]
    fn resolve_absolute_strips_trailing_slash_from_base() {
        let r = resolve_absolute("src", "/home/captain", Some("/srv/proj/"));
        assert_eq!(r, Some("/srv/proj/src".to_string()));
    }

    #[test]
    fn resolve_absolute_expands_env_vars() {
        // shellexpand reads the process env; pin a known var.
        std::env::set_var("HYPRPILOT_TEST_PATH", "/captured");
        let r = resolve_absolute("$HYPRPILOT_TEST_PATH/foo", "/home/captain", None);
        assert_eq!(r, Some("/captured/foo".to_string()));
        std::env::remove_var("HYPRPILOT_TEST_PATH");
    }

    #[test]
    fn resolve_absolute_passes_through_unresolved_env_var() {
        // Undefined var — keep raw rather than refusing to spawn
        // the agent over a typo.
        let r = resolve_absolute("$DEFINITELY_NOT_SET/foo", "/home/captain", Some("/srv"));
        // `shellexpand::env` returns Err on undefined vars; we keep the
        // raw input. Then it's not absolute, no `~`, has no `./` prefix,
        // so it resolves against the cwd base.
        assert_eq!(r, Some("/srv/$DEFINITELY_NOT_SET/foo".to_string()));
    }
}

//! Path resolution helpers shared across frontends. Pure resolution
//! functions take home + cwd explicitly so the same shapes reach
//! every wire (Tauri / JSON-RPC) for any UI consumer (Vue overlay
//! today, Neovim plugin tomorrow). Convenience wrappers
//! (`normalize_cwd` / `display_cwd`) read the process `$HOME` once
//! through `crate::paths::home_dir()` so callers at wire boundaries
//! don't have to thread home through every frame.
//!
//! Display-side niceties (home → `~` substitution) live HERE, not
//! in the frontend. Every wire emit site formats once so dumb
//! consumers (Vue overlay, future Neovim plugin, `ctl` JSON dumps)
//! render the path verbatim and never need to know the captain's
//! home dir.
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

/// Normalize a captain-supplied cwd to an absolute string by
/// expanding `${VAR}` then `~` against the process `$HOME`. Falls
/// back to the input unchanged when neither expansion applies (bare
/// relative paths, paths the daemon should later reject — never
/// silently joined against an arbitrary base here).
///
/// Use at every boundary where a captain-typed cwd lands in daemon
/// state: the actor's read of `cfg.cwd`, every wire-supplied
/// `agent.cwd` overlay (`instances/spawn`, `instances/restart`,
/// `instances/focus { ensure: true }`). Internal storage is then
/// canonical absolute, so the agent-persisted `session.cwd`
/// (always absolute) lines up byte-for-byte with what the daemon
/// thinks the cwd is. **No `canonicalize`** — symlink rewrites
/// would diverge the daemon from the captain's mental model in
/// log lines and from the agent's own bookkeeping.
pub fn normalize_cwd(raw: &str) -> String {
    let home = crate::paths::home_dir().to_string_lossy();
    let env_expanded = expand_env(raw);
    expand_tilde(&env_expanded, &home)
}

/// Format an absolute path for human-facing display: collapse the
/// process `$HOME` prefix back to `~/`. Pass-through for paths that
/// don't sit under `$HOME` and for empty input.
///
/// Use at every wire-emit site that ships a path to a frontend:
/// `InstanceMeta { cwd }`, sessions-list cwd, daemon cwd snapshot,
/// MCP source path. Frontends render the result verbatim — no UI
/// needs to know `$HOME` to do its own collapse, so multi-client
/// designs (Vue overlay + future Neovim plugin + `ctl --json`)
/// stay in lockstep.
pub fn display_cwd(absolute: &str) -> String {
    let home = crate::paths::home_dir().to_string_lossy();

    if absolute == home.as_ref() {
        return "~".to_string();
    }
    let home_with_sep = format!("{home}/");

    if let Some(rest) = absolute.strip_prefix(&home_with_sep) {
        return format!("~/{rest}");
    }
    absolute.to_string()
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

    /// `normalize_cwd` and `display_cwd` round-trip via the process
    /// `$HOME`, so the assertions are anchored in `paths::home_dir()`
    /// rather than a literal — this keeps the suite stable across
    /// CI environments where `$HOME` differs.

    #[test]
    fn normalize_cwd_expands_tilde() {
        let home = crate::paths::home_dir().to_string_lossy().to_string();
        assert_eq!(normalize_cwd("~/proj/x"), format!("{home}/proj/x"));
        assert_eq!(normalize_cwd("~"), home);
    }

    #[test]
    fn normalize_cwd_passes_through_absolute() {
        assert_eq!(normalize_cwd("/etc/foo"), "/etc/foo".to_string());
    }

    #[test]
    fn normalize_cwd_passes_through_relative_unchanged() {
        // Bare relative paths land at later boundaries (the actor
        // joins them against the daemon's cwd, or resolve-time code
        // rejects). normalize_cwd never silently joins.
        assert_eq!(normalize_cwd("src/foo"), "src/foo".to_string());
    }

    #[test]
    fn display_cwd_collapses_home_prefix() {
        let home = crate::paths::home_dir().to_string_lossy().to_string();
        assert_eq!(display_cwd(&format!("{home}/proj/x")), "~/proj/x");
    }

    #[test]
    fn display_cwd_collapses_bare_home() {
        let home = crate::paths::home_dir().to_string_lossy().to_string();
        assert_eq!(display_cwd(&home), "~");
    }

    #[test]
    fn display_cwd_passes_through_non_home_paths() {
        assert_eq!(display_cwd("/etc/foo"), "/etc/foo");
        assert_eq!(display_cwd(""), "");
    }

    #[test]
    fn display_cwd_does_not_match_partial_prefix() {
        // /home/cenk_other should NOT collapse against /home/cenk
        // — strict descendant via the trailing `/`.
        let home = crate::paths::home_dir().to_string_lossy().to_string();
        let sibling = format!("{home}_other/foo");
        assert_eq!(display_cwd(&sibling), sibling);
    }

    #[test]
    fn normalize_then_display_round_trips_tilde() {
        // The bug fix invariant: a captain who writes `~/proj/x`
        // into the config gets `cwd_str = "~/proj/x"` back out at
        // the wire (after normalize → display).
        let r = display_cwd(&normalize_cwd("~/proj/x"));
        assert_eq!(r, "~/proj/x");
    }
}

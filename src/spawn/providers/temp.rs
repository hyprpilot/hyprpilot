//! Launch-scoped temp-config lifecycle + reaper.
//!
//! Claude's `--mcp-config` takes a PATH, and the JSON that path points
//! at carries expanded MCP header secrets (bearer tokens). Writing it
//! to an owner-only (0600) temp file keeps those secrets out of the
//! world-readable `/proc/<pid>/cmdline` argv an inline `--mcp-config
//! <json>` would expose. The launcher `exec()`s, so it never unlinks
//! the file — the vendor CLI reads it afterwards — hence the reaper
//! that sweeps stale orphans before each write.

use std::path::PathBuf;

use anyhow::{Context, Result};

/// Write a launch-scoped config to an owner-only (0600) temp file and
/// return its path. Keeps expanded MCP header secrets (bearer tokens)
/// out of the world-readable `/proc/<pid>/cmdline` argv that an inline
/// `--mcp-config <json>` would expose. The file is created 0600 from
/// the start via `OpenOptionsExt::mode` (not a chmod-after-write race),
/// so it is never briefly world-readable. It is deliberately NOT
/// deleted before the launcher `exec()`s: `exec()` replaces this
/// process, so the vendor CLI must still be able to read the path
/// afterwards — the file is a launch-scoped temp the OS reclaims on
/// tmp cleanup, and its 0600 mode bounds the exposure.
pub(super) fn write_launch_temp_config(label: &str, config: &str) -> Result<PathBuf> {
    use std::io::Write;

    // The launcher `exec()`s, so it never unlinks the file it just
    // wrote — the vendor CLI reads the path afterwards. On long-lived
    // systems those per-launch configs accumulate, so reap stale ones
    // before writing the new file (K-750 item 7).
    reap_stale_temp_configs();

    let path = launch_temp_path("mcp", "json");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .with_context(|| format!("{label}: create owner-only temp config at {}", path.display()))?;
    file.write_all(config.as_bytes())
        .with_context(|| format!("{label}: write temp config at {}", path.display()))?;

    Ok(path)
}

/// Age past which an orphaned launch-scoped MCP temp config is fair
/// game to reap. Comfortably longer than any real agent session, so a
/// live vendor never has its `--mcp-config` file yanked out from under
/// it, while genuinely abandoned files (a launch that outlived a
/// reboot's tmp survival) still get cleaned up.
const STALE_TEMP_CONFIG_TTL: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// Best-effort reap of orphaned `hyprpilot-mcp-*.json` temp configs
/// older than [`STALE_TEMP_CONFIG_TTL`]. Every failure — unreadable
/// dir, un-stat-able entry, unlink error — logs at `debug` and is
/// swallowed: reaping must NEVER abort a launch. Called before each
/// write so the temp dir doesn't grow unbounded across the launcher's
/// `exec()`-and-never-clean-up lifecycle.
fn reap_stale_temp_configs() {
    let dir = std::env::temp_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(err) => {
            tracing::debug!(%err, dir = %dir.display(), "cli: temp reaper: read_dir failed; skipping");
            return;
        }
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !is_reapable_temp_config(name) {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|mtime| now.duration_since(mtime).ok())
            .is_some_and(|age| age >= STALE_TEMP_CONFIG_TTL);
        if !stale {
            continue;
        }
        let path = entry.path();
        match std::fs::remove_file(&path) {
            Ok(()) => tracing::debug!(path = %path.display(), "cli: temp reaper: removed stale MCP config"),
            Err(err) => tracing::debug!(%err, path = %path.display(), "cli: temp reaper: remove failed; skipping"),
        }
    }
}

/// A file name matches the launch-scoped MCP config shape
/// `hyprpilot-mcp-<pid>-<nanos>.json` written by
/// [`write_launch_temp_config`] via [`launch_temp_path`].
fn is_reapable_temp_config(name: &str) -> bool {
    name.starts_with("hyprpilot-mcp-") && name.ends_with(".json")
}

/// A per-launch unique temp path under the OS temp dir. The `(pid,
/// nanos)` pair makes collisions between concurrent launches
/// vanishingly unlikely, and `create_new` on the open turns any
/// residual collision into a hard error rather than a clobber.
fn launch_temp_path(kind: &str, ext: &str) -> PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    std::env::temp_dir().join(format!("hyprpilot-{kind}-{}-{nanos}.{ext}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reapable_predicate_matches_only_launch_mcp_configs() {
        assert!(is_reapable_temp_config("hyprpilot-mcp-123-456.json"));
        assert!(!is_reapable_temp_config("hyprpilot-mcp-123-456.txt"));
        assert!(!is_reapable_temp_config("hyprpilot-other-123.json"));
        assert!(!is_reapable_temp_config("unrelated.json"));
    }

    #[test]
    fn reaper_removes_stale_configs_but_keeps_fresh() {
        use std::time::{Duration, SystemTime, UNIX_EPOCH};

        let dir = std::env::temp_dir();
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let stale = dir.join(format!(
            "hyprpilot-mcp-reaptest-stale-{}-{nanos}.json",
            std::process::id()
        ));
        let fresh = dir.join(format!(
            "hyprpilot-mcp-reaptest-fresh-{}-{nanos}.json",
            std::process::id()
        ));

        let file = std::fs::File::create(&stale).expect("create stale temp");
        file.set_modified(SystemTime::now() - Duration::from_secs(25 * 60 * 60))
            .expect("backdate stale temp");
        drop(file);
        std::fs::File::create(&fresh).expect("create fresh temp");

        reap_stale_temp_configs();

        assert!(!stale.exists(), "a >24h stale MCP config must be reaped");
        assert!(fresh.exists(), "a fresh MCP config must survive the reaper");

        let _ = std::fs::remove_file(&stale);
        let _ = std::fs::remove_file(&fresh);
    }
}

//! tmux/zellij window-title integration.
//!
//! `[multiplexer] set_title = true` (seeded by defaults.toml) drives a
//! best-effort rename of the current tmux window / zellij tab to
//! `hyprpilot@<cwd-basename>` right before the launcher `exec()`s into
//! the vendor CLI (see `spawn::launch_profile`). Mechanism is
//! shelling out to the vendor multiplexer CLI, NOT OSC escapes — OSC 2
//! is gated by tmux `allow-rename`/`set-titles` and varies by zellij
//! version, whereas `tmux rename-window` / `zellij action rename-tab`
//! work regardless of those settings.
//!
//! Every failure (multiplexer binary missing, non-zero exit, `TMUX_PANE`
//! unset) logs at `debug` and never aborts the launch — this is a
//! cosmetic affordance, not a correctness requirement.

use std::path::Path;
use std::process::{Command, Stdio};

/// `rename_argv` embeds this literal token for the tmux pane target;
/// `set_title` substitutes the real `$TMUX_PANE` value (there is no
/// shell in the `Command::new` spawn to expand it for us) right before
/// spawning. Keeping the substitution out of `rename_argv` keeps that
/// function pure and deterministic for tests.
const TMUX_PANE_PLACEHOLDER: &str = "$TMUX_PANE";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Multiplexer {
    Tmux,
    Zellij,
}

impl Multiplexer {
    /// Detects the live multiplexer from the real process environment.
    /// Precedence: `TMUX` then `ZELLIJ` — when both are set (nested
    /// sessions), `$TMUX_PANE` gives a precise rename target so `Tmux`
    /// wins.
    pub(crate) fn detect() -> Option<Self> {
        Self::detect_with(|name| std::env::var(name).ok())
    }

    /// Same precedence as `detect`, routed through a caller-supplied
    /// lookup so tests never touch real process env.
    pub(crate) fn detect_with(lookup: impl Fn(&str) -> Option<String>) -> Option<Self> {
        if lookup("TMUX").is_some() {
            Some(Self::Tmux)
        } else if lookup("ZELLIJ").is_some() {
            Some(Self::Zellij)
        } else {
            None
        }
    }

    /// Pure argv builder — no env reads, no spawning. `--` always
    /// precedes the title so a folder named `-foo` isn't parsed as a
    /// flag.
    pub(crate) fn rename_argv(self, title: &str) -> Vec<String> {
        match self {
            Self::Tmux => vec![
                "tmux".to_string(),
                "rename-window".to_string(),
                "-t".to_string(),
                TMUX_PANE_PLACEHOLDER.to_string(),
                "--".to_string(),
                title.to_string(),
            ],
            Self::Zellij => vec![
                "zellij".to_string(),
                "action".to_string(),
                "rename-tab".to_string(),
                "--".to_string(),
                title.to_string(),
            ],
        }
    }

    /// Best-effort rename: substitutes the real tmux pane target, then
    /// spawns + waits. Every failure mode (missing binary, non-zero
    /// exit, unset `TMUX_PANE`) logs at `debug` and returns — never
    /// propagates an error, never aborts the launch.
    pub(crate) fn set_title(self, title: &str) {
        let mut argv = self.rename_argv(title);

        if self == Self::Tmux {
            let Ok(pane) = std::env::var("TMUX_PANE") else {
                tracing::debug!("cli spawn: multiplexer rename skipped; TMUX_PANE unset");
                return;
            };
            for arg in &mut argv {
                if arg == TMUX_PANE_PLACEHOLDER {
                    *arg = pane.clone();
                }
            }
        }

        let Some((program, args)) = argv.split_first() else {
            return;
        };

        match Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
        {
            Ok(status) if !status.success() => {
                tracing::debug!(multiplexer = ?self, title, ?status, "cli spawn: multiplexer rename exited non-zero");
            }
            Err(err) => {
                tracing::debug!(multiplexer = ?self, title, %err, "cli spawn: multiplexer rename failed to spawn");
            }
            Ok(_) => {}
        }
    }
}

/// `hyprpilot@<basename>` — `cwd`'s final path component, or the full
/// display string when `file_name()` is `None` (e.g. cwd is `/`).
pub(crate) fn title_for(cwd: &Path) -> String {
    let base = cwd
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| cwd.display().to_string());

    format!("hyprpilot@{base}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_prefers_tmux_when_both_set() {
        let lookup = |name: &str| match name {
            "TMUX" => Some("/tmp/tmux-1000/default,1234,0".to_string()),
            "ZELLIJ" => Some("0".to_string()),
            _ => None,
        };

        assert_eq!(Multiplexer::detect_with(lookup), Some(Multiplexer::Tmux));
    }

    #[test]
    fn detect_falls_back_to_zellij() {
        let lookup = |name: &str| (name == "ZELLIJ").then(|| "0".to_string());

        assert_eq!(Multiplexer::detect_with(lookup), Some(Multiplexer::Zellij));
    }

    #[test]
    fn detect_returns_none_outside_a_multiplexer() {
        let lookup = |_: &str| None;

        assert_eq!(Multiplexer::detect_with(lookup), None);
    }

    #[test]
    fn detect_ignores_unrelated_env_vars() {
        let lookup = |name: &str| (name == "SHELL").then(|| "/bin/zsh".to_string());

        assert_eq!(Multiplexer::detect_with(lookup), None);
    }

    #[test]
    fn rename_argv_tmux_shape() {
        let argv = Multiplexer::Tmux.rename_argv("hyprpilot@hyprpilot");

        assert_eq!(
            argv,
            vec!["tmux", "rename-window", "-t", "$TMUX_PANE", "--", "hyprpilot@hyprpilot"]
        );
    }

    #[test]
    fn rename_argv_zellij_shape() {
        let argv = Multiplexer::Zellij.rename_argv("hyprpilot@hyprpilot");

        assert_eq!(
            argv,
            vec!["zellij", "action", "rename-tab", "--", "hyprpilot@hyprpilot"]
        );
    }

    #[test]
    fn rename_argv_passes_dash_dash_before_a_dash_prefixed_title() {
        let argv = Multiplexer::Tmux.rename_argv("-foo");

        let dash_dash_idx = argv.iter().position(|arg| arg == "--").expect("-- present");
        assert_eq!(argv[dash_dash_idx + 1], "-foo");
    }

    #[test]
    fn title_for_uses_resolved_cwd_basename() {
        assert_eq!(
            title_for(Path::new("/home/cenk/development/hyprpilot")),
            "hyprpilot@hyprpilot"
        );
    }

    #[test]
    fn title_for_falls_back_to_full_display_at_root() {
        assert_eq!(title_for(Path::new("/")), "hyprpilot@/");
    }
}

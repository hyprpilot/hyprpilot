//! Shell completion — clap_complete's dynamic engine.
//!
//! The shell calls the binary back on every `<TAB>` (`COMPLETE=<shell>
//! hyprpilot -- <words>`), so candidates come from the captain's
//! RESOLVED config rather than a script generated at build time. A
//! value completer only sees the word under the cursor, so the layer
//! selection (`--config`, every `--config-profile`) and the positional
//! profile are read back off the in-progress command line the shell
//! hands the engine.
//!
//! Completion must stay silent: a config that fails to load or validate
//! yields no candidates, never an error on the captain's prompt.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use clap::parser::ValueSource;
use clap::{Args, CommandFactory};
use clap_complete::env::{Bash, Elvish, EnvCompleter, Fish, Powershell, Shells};
use clap_complete::CompletionCandidate;

use crate::config::Config;

/// The env var the registration script sets when it calls back in.
pub const VAR: &str = "COMPLETE";

/// clap_complete's engines, with zsh swapped for [`Zsh`].
pub const SHELLS: Shells<'static> = Shells(&[&Bash, &Elvish, &Fish, &Powershell, &Zsh]);

/// clap_complete's zsh engine, registered as `_hyprpilot` rather than
/// `_clap_dynamic_completer_hyprpilot`.
///
/// `_<command>` is the name zsh plugin managers expect to find, e.g. a
/// lazy `znap function _hyprpilot '…eval "$(hyprpilot completion zsh)"'`
/// followed by `compdef _hyprpilot hyprpilot`. The script's own
/// `compdef` is skipped when `compinit` has not run yet, so evaluating
/// it early is not an error.
struct Zsh;

impl EnvCompleter for Zsh {
    fn name(&self) -> &'static str {
        clap_complete::env::Zsh.name()
    }

    fn is(&self, name: &str) -> bool {
        clap_complete::env::Zsh.is(name)
    }

    fn write_registration(
        &self,
        var: &str,
        name: &str,
        bin: &str,
        completer: &str,
        buf: &mut dyn std::io::Write,
    ) -> std::io::Result<()> {
        let mut script = Vec::new();
        clap_complete::env::Zsh.write_registration(var, name, bin, completer, &mut script)?;
        let function = format!("_{}", name.replace('-', "_"));
        let script = String::from_utf8_lossy(&script)
            .replace(&format!("_clap_dynamic_completer{function}"), &function)
            .replace(
                &format!("compdef {function} {bin}"),
                &format!("(( $+functions[compdef] )) && compdef {function} {bin}"),
            );

        buf.write_all(script.as_bytes())
    }

    fn write_complete(
        &self,
        cmd: &mut clap::Command,
        args: Vec<OsString>,
        current_dir: Option<&Path>,
        buf: &mut dyn std::io::Write,
    ) -> std::io::Result<()> {
        clap_complete::env::Zsh.write_complete(cmd, args, current_dir, buf)
    }
}

#[derive(Args, Debug)]
pub struct CompletionArgs {
    /// Shell to print the registration script for.
    #[arg(value_enum)]
    shell: clap_complete::Shell,
}

/// Print the same registration script `COMPLETE=<shell> hyprpilot`
/// does, for `eval "$(hyprpilot completion zsh)"`.
pub fn run(args: &CompletionArgs) -> Result<()> {
    let name = args.shell.to_string();
    let shell = SHELLS
        .completer(&name)
        .ok_or_else(|| anyhow!("no completion engine for `{name}`"))?;
    // The binary the shell calls back, spelled the way
    // `CompleteEnv` spells it: a bare name stays a PATH lookup.
    let mut completer = PathBuf::from(std::env::args_os().next().unwrap_or_else(|| "hyprpilot".into()));
    if completer.components().count() > 1 {
        completer = std::env::current_dir()?.join(completer);
    }
    let cmd = crate::Cli::command();
    shell.write_registration(
        VAR,
        cmd.get_name(),
        cmd.get_name(),
        &completer.to_string_lossy(),
        &mut std::io::stdout(),
    )?;

    Ok(())
}

/// The parts of the in-progress command line the completers need.
#[derive(Debug, Default)]
struct CommandLine {
    /// `--config` / `HYPRPILOT_CONFIG`.
    config: Option<PathBuf>,
    /// The overlays a launch would fold: every `--config-profile` on
    /// the line, else `HYPRPILOT_CONFIG_PROFILE`.
    overlays: Vec<String>,
    /// Whether `overlays` came off the line, which makes them the ones
    /// not worth offering again.
    overlays_on_line: bool,
    /// The positional `[PROFILE]`.
    profile: Option<String>,
}

impl CommandLine {
    /// The words before the cursor, from the process args `CompleteEnv`
    /// passes: `<bin> -- <words…>`, the cursor at `_CLAP_COMPLETE_INDEX`
    /// (bash, zsh, elvish) or on the last word (fish, powershell).
    fn current() -> Self {
        let index = std::env::var("_CLAP_COMPLETE_INDEX")
            .ok()
            .and_then(|raw| raw.parse().ok());
        Self::parse(words_before_cursor(std::env::args_os().collect(), index))
    }

    /// Parse leniently with the real CLI, so a flag means here what it
    /// means at launch — env fallbacks and value delimiters included.
    fn parse(words: Vec<OsString>) -> Self {
        let Ok(matches) = crate::Cli::command().ignore_errors(true).try_get_matches_from(words) else {
            return Self::default();
        };
        // The shell hands a word over as typed: quotes intact, `~` and
        // `$VAR` unexpanded.
        Self {
            config: matches
                .try_get_one::<PathBuf>("config")
                .ok()
                .flatten()
                .map(|path| crate::paths::resolve_user(unquote(&path.to_string_lossy()))),
            overlays: matches
                .try_get_many::<String>("config_profiles")
                .ok()
                .flatten()
                .map(|values| values.map(|name| unquote(name).to_string()).collect())
                .unwrap_or_default(),
            overlays_on_line: matches.value_source("config_profiles") == Some(ValueSource::CommandLine),
            profile: matches
                .try_get_one::<String>("profile_id")
                .ok()
                .flatten()
                .map(|id| unquote(id).to_string()),
        }
    }

    fn load(&self) -> Option<Config> {
        let cfg = crate::config::load(self.config.as_deref(), &self.overlays).ok()?;
        cfg.validate().ok()?;

        Some(cfg)
    }
}

fn unquote(word: &str) -> &str {
    ['\'', '"']
        .into_iter()
        .find_map(|quote| word.strip_prefix(quote)?.strip_suffix(quote))
        .unwrap_or(word)
}

fn words_before_cursor(mut args: Vec<OsString>, index: Option<usize>) -> Vec<OsString> {
    let start = args.iter().position(|arg| arg == "--").map_or(args.len(), |i| i + 1);
    args.drain(..start);
    let cursor = index.unwrap_or(args.len().saturating_sub(1)).min(args.len());
    args.truncate(cursor);

    args
}

/// Session profile ids from the resolved config, `<agent> <model>` as
/// help and the default marked.
pub fn profile_ids() -> Vec<CompletionCandidate> {
    let Some(cfg) = CommandLine::current().load() else {
        return Vec::new();
    };
    crate::spawn::list_profiles(&cfg, None, &[])
        .into_iter()
        .map(|summary| {
            let mut help = summary.agent;
            if let Some(model) = summary.model {
                help = format!("{help} {model}");
            }
            if summary.is_default {
                help.push_str(" (default)");
            }
            CompletionCandidate::new(summary.id).help(Some(help.into()))
        })
        .collect()
}

/// Overlay names under `profiles/` in the config dir, minus the ones
/// already on the line.
pub fn config_profiles() -> Vec<CompletionCandidate> {
    let line = CommandLine::current();
    let given = if line.overlays_on_line {
        line.overlays
    } else {
        Vec::new()
    };
    overlay_names(&crate::paths::config_dir().join("profiles"), &given)
        .into_iter()
        .map(CompletionCandidate::new)
        .collect()
}

fn overlay_names(dir: &Path, exclude: &[String]) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let ext = path.extension()?.to_str()?;
            if !path.is_file() || !crate::paths::CONFIG_EXTENSIONS.contains(&ext) {
                return None;
            }
            path.file_stem()?.to_str().map(str::to_string)
        })
        .filter(|name| !exclude.contains(name))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// The modes the launched profile's provider accepts — the positional
/// one, else `[profile] default`.
pub fn modes() -> Vec<CompletionCandidate> {
    let line = CommandLine::current();
    let Some(cfg) = line.load() else {
        return Vec::new();
    };
    let Ok((resolved, _)) = crate::resolve::resolve_into_instance_and_profile(&cfg, line.profile.as_deref(), &[])
    else {
        return Vec::new();
    };
    crate::spawn::providers::known_modes(resolved.agent.provider)
        .into_iter()
        .map(CompletionCandidate::new)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(argv: &[&str]) -> Vec<OsString> {
        argv.iter().map(OsString::from).collect()
    }

    #[test]
    fn the_words_before_the_cursor_are_what_the_shell_passed_after_the_escape() {
        let args = words(&[
            "/usr/bin/hyprpilot",
            "--",
            "hyprpilot",
            "--config-profile",
            "work",
            "cl",
        ]);

        assert_eq!(
            words_before_cursor(args.clone(), Some(3)),
            words(&["hyprpilot", "--config-profile", "work"])
        );
        assert_eq!(
            words_before_cursor(args.clone(), None),
            words(&["hyprpilot", "--config-profile", "work"]),
            "without an index the cursor is on the last word"
        );
        assert_eq!(
            words_before_cursor(args, Some(99)).len(),
            4,
            "an index past the end keeps every word"
        );
    }

    #[test]
    fn the_command_line_yields_the_selection_and_the_positional_profile() {
        let line = CommandLine::parse(words(&[
            "hyprpilot",
            "--config",
            "/etc/hyprpilot/config.yaml",
            "--config-profile",
            "personal",
            "--config-profile=work",
            "claude/opus",
            "--mode",
        ]));

        assert_eq!(line.config, Some(PathBuf::from("/etc/hyprpilot/config.yaml")));
        assert_eq!(line.overlays, ["personal", "work"]);
        assert!(line.overlays_on_line);
        assert_eq!(line.profile.as_deref(), Some("claude/opus"));
    }

    /// bash hands a word over as typed, so `'~/cfg.yaml'` must load
    /// the same file a launch would.
    #[test]
    fn quoted_and_unexpanded_words_resolve_like_a_launch() {
        let line = CommandLine::parse(words(&[
            "hyprpilot",
            "--config",
            "'~/cfg.yaml'",
            "--config-profile",
            "\"work\"",
            "'claude/opus'",
        ]));

        assert_eq!(line.config, Some(crate::paths::resolve_user("~/cfg.yaml")));
        assert_eq!(line.overlays, ["work"]);
        assert_eq!(line.profile.as_deref(), Some("claude/opus"));
    }

    /// The flag the cursor is completing has no value yet. Parsing must
    /// still succeed rather than drop the rest of the line.
    #[test]
    fn a_trailing_flag_without_its_value_does_not_lose_the_line() {
        let line = CommandLine::parse(words(&["hyprpilot", "--config-profile", "work", "--config-profile"]));

        assert_eq!(line.overlays, ["work"]);
    }

    /// The contract zsh plugin managers load against: a `_hyprpilot`
    /// completer, and a script that can be evaluated before `compinit`.
    #[test]
    fn the_zsh_script_defines_the_conventional_completer() {
        let mut buf = Vec::new();
        Zsh.write_registration(VAR, "hyprpilot", "hyprpilot", "hyprpilot", &mut buf)
            .unwrap();
        let script = String::from_utf8(buf).unwrap();

        assert!(script.contains("function _hyprpilot() {"), "{script}");
        assert!(
            script.contains("(( $+functions[compdef] )) && compdef _hyprpilot hyprpilot"),
            "{script}"
        );
        assert!(!script.contains("_clap_dynamic_completer"), "{script}");
    }

    #[test]
    fn overlay_names_cover_every_extension_once_and_skip_the_given_ones() {
        let dir = tempfile::tempdir().unwrap();
        for file in ["personal.yaml", "work.toml", "work.json", "notes.txt"] {
            std::fs::write(dir.path().join(file), "").unwrap();
        }
        std::fs::create_dir(dir.path().join("archive.yaml")).unwrap();

        assert_eq!(overlay_names(dir.path(), &[]), ["personal", "work"]);
        assert_eq!(overlay_names(dir.path(), &["work".into()]), ["personal"]);
        assert!(overlay_names(&dir.path().join("missing"), &[]).is_empty());
    }
}

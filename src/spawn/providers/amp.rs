//! Amp (`amp`) native-flag projection.
//!
//! amp selects its model, system prompt and tool set through one knob —
//! the agent mode — so a profile's `mode` is the only selection that
//! projects. Which account and which settings file a launch uses are
//! the vendor's own env (`AMP_API_KEY`, `AMP_SETTINGS_FILE`), set on
//! the agent or profile `env` like `CLAUDE_CONFIG_DIR` / `CODEX_HOME`.

use anyhow::Result;

use crate::mcp::{expanded_raw, MCPDefinition};
use crate::resolve::profile::ResolvedProfile;

use super::argv::{combined_args, has_flag};
use super::{base_command, mcp_transports, HarnessProjection, Resume, SpawnCommand};

pub(super) fn build_amp(
    resolved: &ResolvedProfile,
    system_prompt: Option<&str>,
    mcp_defs: &[MCPDefinition],
    provider_args: Vec<String>,
    prompt: Option<&str>,
    resume: Option<&Resume>,
    harness: Option<&HarnessProjection>,
) -> Result<SpawnCommand> {
    let mut command = base_command(resolved);
    let detect_args = combined_args(&command.args, &provider_args);

    // `-x` takes an OPTIONAL message, so a positional prompt after it
    // would be ambiguous with whatever flag follows; with no message it
    // reads the prompt from stdin.
    if prompt.is_some() && !has_flag(&detect_args, "--execute", Some("-x")) {
        command.args.push("--execute".into());
    }

    if let Some(harness) = harness {
        if harness.structured_output && !has_flag(&detect_args, "--stream-json", None) {
            command.args.push("--stream-json".into());
        }
        // An executed thread is archived when the turn ends, and an
        // archived thread refuses `threads continue` — the harness's
        // next turn would fail. Verified against the installed CLI.
        if !has_flag(&detect_args, "--no-archive-after-execute", None) {
            command.args.push("--no-archive-after-execute".into());
        }
    }

    // Resuming is the `threads continue` subcommand; the global flags
    // generated below are accepted after it.
    if let Some(resume) = resume {
        if !has_flag(&detect_args, "threads", None) && !has_flag(&detect_args, "last", None) {
            let target = match resume {
                Resume::Picker => None,
                Resume::Last => Some("--last".to_string()),
                Resume::Session(id) => Some(id.clone()),
            };
            command.args.splice(
                0..0,
                ["threads".to_string(), "continue".to_string()]
                    .into_iter()
                    .chain(target),
            );
        }
    }

    if let Some(mode) = resolved.mode.as_deref() {
        if !has_flag(&detect_args, "--mode", Some("-m")) {
            command.args.push("--mode".into());
            command.args.push(mode.into());
        }
    }
    if let Some(model) = resolved.model.as_deref() {
        tracing::warn!(
            model,
            "cli spawn: amp picks its model through the mode; ignoring model (set `mode` instead)"
        );
    }
    if let Some(effort) = resolved.effort.as_deref() {
        tracing::warn!(effort, "cli spawn: amp has no effort setting; ignoring effort");
    }
    if system_prompt.is_some_and(|prompt| !prompt.is_empty()) {
        tracing::warn!(
            "cli spawn: amp takes no system prompt on its command line; ignoring system_prompt (amp reads AGENTS.md instead)"
        );
    }

    if !has_flag(&detect_args, "--mcp-config", None) {
        if let Some(config) = amp_mcp_config(mcp_defs)? {
            // By PATH, like claude: the JSON carries expanded header
            // secrets and argv is world-readable.
            let temp_config = super::temp::write_launch_temp_config("amp --mcp-config", &config)?;
            command.args.push("--mcp-config".into());
            command.args.push(temp_config.path().to_string_lossy().into_owned());
            command.temp_config = Some(temp_config);
        }
    }
    for def in mcp_defs.iter().filter(|def| restricts_tools(def)) {
        tracing::warn!(
            name = %def.name,
            "cli spawn: amp has no command-line tool policy; ignoring includeTools / excludeTools / autoRejectTools (use `amp.tools.disable` in the settings file)"
        );
    }

    command.args.extend(provider_args);
    command.stdin_prompt = prompt.map(str::to_string);

    Ok(command)
}

/// The `--mcp-config` body: a bare `{ name: server }` map. amp rejects
/// the `{ "mcpServers": … }` wrapper claude takes.
fn amp_mcp_config(defs: &[MCPDefinition]) -> Result<Option<String>> {
    let servers: serde_json::Map<String, serde_json::Value> = mcp_transports(defs, "amp")
        .map(|(def, _)| (def.name.clone(), expanded_raw(def)))
        .collect();
    if servers.is_empty() {
        return Ok(None);
    }

    serde_json::to_string(&servers)
        .map(Some)
        .map_err(|e| anyhow::anyhow!("serialize amp MCP config: {e}"))
}

fn restricts_tools(def: &MCPDefinition) -> bool {
    def.hyprpilot.include_tools.is_some()
        || !def.hyprpilot.exclude_tools.is_empty()
        || !def.hyprpilot.auto_reject_tools.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AgentProvider;
    use crate::spawn::providers::build_command;
    use crate::spawn::providers::fixtures::*;

    fn amp_command(
        resolved: &ResolvedProfile,
        mcp_defs: &[MCPDefinition],
        provider_args: Vec<String>,
        prompt: Option<&str>,
        resume: Option<&Resume>,
        harness: Option<&HarnessProjection>,
    ) -> SpawnCommand {
        build_command(resolved, None, mcp_defs, provider_args, prompt, resume, harness).unwrap()
    }

    #[test]
    fn amp_projects_mode_and_nothing_else_it_cannot_honour() {
        let command = amp_command(&resolved(AgentProvider::Amp), &[], vec![], None, None, None);

        assert_eq!(command.args, ["--mode", "plan"]);
        assert!(command.stdin_prompt.is_none());
    }

    #[test]
    fn amp_mode_yields_to_a_hand_passed_short_flag() {
        let command = amp_command(
            &resolved(AgentProvider::Amp),
            &[],
            vec!["-m".into(), "high".into()],
            None,
            None,
            None,
        );

        assert_eq!(command.args, ["-m", "high"]);
    }

    /// The prompt must ride stdin: `-x` swallows a following operand as
    /// its message, so a positional would be read as the prompt only by
    /// accident of argv order.
    #[test]
    fn amp_headless_executes_with_the_prompt_on_stdin() {
        let command = amp_command(
            &resolved_with_mode(AgentProvider::Amp, None),
            &[],
            vec![],
            Some("fix the test"),
            None,
            None,
        );

        assert_eq!(command.args, ["--execute"]);
        assert_eq!(command.stdin_prompt.as_deref(), Some("fix the test"));
    }

    #[test]
    fn amp_harness_streams_json_and_keeps_the_thread_resumable() {
        let command = amp_command(
            &resolved_with_mode(AgentProvider::Amp, None),
            &[],
            vec![],
            Some("go"),
            Some(&Resume::Session("T-123".into())),
            Some(&HarnessProjection {
                structured_output: true,
            }),
        );

        assert_eq!(
            command.args,
            [
                "threads",
                "continue",
                "T-123",
                "--execute",
                "--stream-json",
                "--no-archive-after-execute"
            ]
        );
    }

    #[test]
    fn amp_projects_every_resume_intent() {
        let cases = [
            (Resume::Picker, vec!["threads", "continue"]),
            (Resume::Last, vec!["threads", "continue", "--last"]),
            (Resume::Session("T-abc".into()), vec!["threads", "continue", "T-abc"]),
        ];

        for (resume, expected) in cases {
            let command = amp_command(
                &resolved_with_mode(AgentProvider::Amp, None),
                &[],
                vec![],
                None,
                Some(&resume),
                None,
            );

            assert_eq!(command.args, expected, "{resume:?}");
        }
    }

    #[test]
    fn amp_resume_yields_to_a_hand_passed_subcommand() {
        let command = amp_command(
            &resolved_with_mode(AgentProvider::Amp, None),
            &[],
            vec!["last".into()],
            None,
            Some(&Resume::Picker),
            None,
        );

        assert_eq!(command.args, ["last"]);
    }

    #[test]
    fn amp_mcp_config_is_a_bare_server_map_behind_an_owner_only_path() {
        let _guard = env_test_guard();
        std::env::set_var("NVIM_GITHUB", "secret-token");
        let command = amp_command(
            &resolved_with_mode(AgentProvider::Amp, None),
            &[mcp_def(), remote_mcp_def_with_headers(), transportless_mcp_def()],
            vec![],
            None,
            None,
            None,
        );
        std::env::remove_var("NVIM_GITHUB");

        let at = command.args.iter().position(|arg| arg == "--mcp-config").unwrap();
        let path = &command.args[at + 1];
        assert!(!command.args.iter().any(|arg| arg.contains("secret-token")));
        assert_eq!(
            command.temp_config.as_ref().unwrap().path().to_str(),
            Some(path.as_str())
        );

        let parsed: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let names: Vec<&str> = parsed.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(names, ["github", "hyprpilot-nvim"]);
        assert_eq!(parsed["hyprpilot-nvim"]["command"], "uvx");
        assert_eq!(parsed["github"]["headers"]["Authorization"], "Bearer secret-token");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn amp_hand_passed_mcp_config_suppresses_the_generated_one() {
        let command = amp_command(
            &resolved_with_mode(AgentProvider::Amp, None),
            &[mcp_def()],
            vec!["--mcp-config".into(), "/mine.json".into()],
            None,
            None,
            None,
        );

        assert_eq!(command.args, ["--mcp-config", "/mine.json"]);
        assert!(command.temp_config.is_none());
    }
}

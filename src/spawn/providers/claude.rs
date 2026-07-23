//! Claude Code (`claude`) native-flag projection.

use anyhow::Result;

use crate::mcp::{expanded_raw, project_transport, MCPDefinition};
use crate::profile::ResolvedProfile;

use super::argv::{combined_args, has_flag};
use super::{base_command, ensure_inline_size, SpawnCommand};

pub(super) fn build_claude(
    resolved: &ResolvedProfile,
    system_prompt: Option<&str>,
    mcp_defs: &[MCPDefinition],
    provider_args: Vec<String>,
    prompt: Option<&str>,
) -> Result<SpawnCommand> {
    let mut command = base_command(resolved);
    // Headless: `claude --print` reading the prompt from STDIN. `--print`
    // is prepended so the whole model/effort/mode/system-prompt/MCP
    // projection below is shared with the interactive path. The prompt is
    // delivered on the child's stdin (see `stdin_prompt` below), NOT as a
    // trailing positional: claude's `--allowedTools`/`--disallowedTools`
    // are variadic (`<tools...>`) and would greedily swallow a trailing
    // operand as a tool entry, so a positional prompt never reaches the
    // model. stdin has no such ambiguity.
    if prompt.is_some() && !has_flag(&command.args, "--print", Some("-p")) {
        command.args.insert(0, "--print".into());
    }
    let detect_args = combined_args(&command.args, &provider_args);

    if let Some(model) = resolved.model.as_deref() {
        if !has_flag(&detect_args, "--model", None) {
            command.args.push("--model".into());
            command.args.push(model.into());
        }
    }
    if let Some(effort) = resolved.effort.as_deref() {
        if !has_flag(&detect_args, "--effort", None) {
            command.args.push("--effort".into());
            command.args.push(effort.into());
        }
    }
    if let Some(mode) = resolved.mode.as_deref() {
        if !has_flag(&detect_args, "--permission-mode", None) {
            command.args.push("--permission-mode".into());
            command.args.push(mode.into());
        }
    }
    if let Some(prompt) = system_prompt {
        if !prompt.is_empty() && !has_flag(&detect_args, "--append-system-prompt", None) {
            ensure_inline_size("claude --append-system-prompt", prompt)?;
            command.args.push("--append-system-prompt".into());
            command.args.push(prompt.into());
        }
    }
    if !mcp_defs.is_empty() && !has_flag(&detect_args, "--mcp-config", None) {
        let config = claude_mcp_config(mcp_defs)?;
        // Pass the resolved MCP config by PATH, never inline: the JSON
        // carries expanded header secrets (bearer tokens) and argv is
        // world-readable via `/proc/<pid>/cmdline`. `claude --help`
        // documents `--mcp-config` as accepting "JSON files or
        // strings"; the temp file is created 0600 and is launch-scoped.
        let path = super::temp::write_launch_temp_config("claude --mcp-config", &config)?;
        command.args.push("--mcp-config".into());
        command.args.push(path.to_string_lossy().into_owned());
    }
    let permission_tools = claude_mcp_permission_tools(mcp_defs);
    if !permission_tools.allow.is_empty() && !has_claude_allowed_tools_flag(&detect_args) {
        command.args.push("--allowedTools".into());
        command.args.push(permission_tools.allow.join(","));
    }
    if !permission_tools.deny.is_empty() && !has_claude_disallowed_tools_flag(&detect_args) {
        command.args.push("--disallowedTools".into());
        command.args.push(permission_tools.deny.join(","));
    }

    command.args.extend(provider_args);
    // Deliver the headless prompt on stdin (see the `--print` comment
    // above) — never as a positional the variadic tool flags would eat.
    command.stdin_prompt = prompt.map(str::to_string);

    Ok(command)
}

fn has_claude_allowed_tools_flag(args: &[String]) -> bool {
    has_flag(args, "--allowedTools", None) || has_flag(args, "--allowed-tools", None)
}

fn has_claude_disallowed_tools_flag(args: &[String]) -> bool {
    has_flag(args, "--disallowedTools", None) || has_flag(args, "--disallowed-tools", None)
}

fn claude_mcp_config(defs: &[MCPDefinition]) -> Result<String> {
    let mut servers = serde_json::Map::new();
    for def in defs {
        // Parity with codex/opencode, which warn+skip transportless
        // defs: claude used to serialize any raw entry verbatim, so a
        // def missing both `command` and `url` reached the vendor as a
        // malformed server. Skip it here instead.
        if project_transport(def).is_none() {
            tracing::warn!(name = %def.name, "cli spawn: skipping MCP entry without command or url for claude");
            continue;
        }
        servers.insert(def.name.clone(), claude_mcp_server_config(def));
    }
    serde_json::to_string(&serde_json::json!({ "mcpServers": servers }))
        .map_err(|e| anyhow::anyhow!("serialize claude MCP config: {e}"))
}

fn claude_mcp_server_config(def: &MCPDefinition) -> serde_json::Value {
    let mut raw = expanded_raw(def);
    let Some(obj) = raw.as_object_mut() else {
        return raw;
    };
    if obj.contains_key("url") && !obj.contains_key("type") {
        let transport = obj
            .get("transport")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("http");
        obj.insert("type".into(), serde_json::Value::String(transport.into()));
    }

    raw
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ClaudePermissionTools {
    allow: Vec<String>,
    deny: Vec<String>,
}

fn claude_mcp_permission_tools(defs: &[MCPDefinition]) -> ClaudePermissionTools {
    let mut tools = ClaudePermissionTools::default();
    for def in defs {
        if let Some(include) = def.hyprpilot.include_tools.as_ref() {
            tools.allow.extend(
                include
                    .iter()
                    .map(|pattern| claude_mcp_tool_pattern(&def.name, pattern)),
            );
        }
        tools.deny.extend(
            def.hyprpilot
                .exclude_tools
                .iter()
                .map(|pattern| claude_mcp_tool_pattern(&def.name, pattern)),
        );
        tools.allow.extend(
            def.hyprpilot
                .auto_accept_tools
                .iter()
                .map(|pattern| claude_mcp_tool_pattern(&def.name, pattern)),
        );
        tools.deny.extend(
            def.hyprpilot
                .auto_reject_tools
                .iter()
                .map(|pattern| claude_mcp_tool_pattern(&def.name, pattern)),
        );
    }

    tools.allow.sort();
    tools.allow.dedup();
    tools.deny.sort();
    tools.deny.dedup();

    tools
}

fn claude_mcp_tool_pattern(server: &str, pattern: &str) -> String {
    if pattern.starts_with("mcp__") {
        pattern.to_string()
    } else {
        format!("mcp__{server}__{pattern}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AgentProvider;
    use crate::spawn::providers::build_command;
    use crate::spawn::providers::fixtures::*;

    #[test]
    fn claude_provider_args_suppress_generated_model() {
        let command = build_command(
            &resolved(AgentProvider::ClaudeCode),
            None,
            &[],
            vec!["--model".into(), "user-model".into()],
            None,
        )
        .unwrap();

        assert_eq!(command.args.iter().filter(|arg| arg.as_str() == "--model").count(), 1);
    }

    /// Resolved argv order: profile-replaced `agent.args →
    /// generated…`. `resolved.agent.args` here stands in for the
    /// already-replaced list — a `--fallback-model` override arg must
    /// precede the generated `--model` flag `build_command` appends
    /// from `resolved.model`.
    #[test]
    fn override_agent_args_precede_generated_flags() {
        let resolved = resolved_with_agent_args(
            AgentProvider::ClaudeCode,
            vec!["--fallback-model".into(), "claude-sonnet-4-5".into()],
        );
        let command = build_command(&resolved, None, &[], vec![], None).unwrap();

        let fallback_idx = command
            .args
            .iter()
            .position(|a| a == "--fallback-model")
            .expect("--fallback-model present");
        let model_idx = command
            .args
            .iter()
            .position(|a| a == "--model")
            .expect("generated --model present");

        assert!(
            fallback_idx < model_idx,
            "override args must precede generated vendor flags"
        );
        assert_eq!(
            command.args[model_idx + 1],
            "model-a",
            "generated --model flag still carries resolved.model's value"
        );
    }

    /// An override arg spelling a generated flag (`--model`)
    /// suppresses the generated duplicate — the existing `has_flag`
    /// dedup in `build_claude` covers `resolved.agent.args` the same
    /// way it already covers `provider_args` (trailing `-- <args>`).
    #[test]
    fn override_authored_model_flag_suppresses_generated_duplicate() {
        let resolved = resolved_with_agent_args(
            AgentProvider::ClaudeCode,
            vec!["--model".into(), "override-model".into()],
        );
        let command = build_command(&resolved, None, &[], vec![], None).unwrap();

        assert_eq!(
            command.args.iter().filter(|arg| arg.as_str() == "--model").count(),
            1,
            "generated --model must not duplicate the override-authored one"
        );
        assert_eq!(
            command.args,
            vec!["--model".to_string(), "override-model".to_string()],
            "override's --model value wins; no generated --model appended"
        );
    }

    #[test]
    fn claude_mcp_config_uses_expanded_mcp_entries() {
        let _guard = env_test_guard();
        let home = std::env::var("HOME").expect("HOME should be present in test env");
        let command = build_command(
            &resolved(AgentProvider::ClaudeCode),
            None,
            &[mcp_def_with_home_env()],
            Vec::new(),
            None,
        )
        .unwrap();
        let path = command
            .args
            .windows(2)
            .find_map(|w| (w[0] == "--mcp-config").then_some(&w[1]))
            .expect("claude mcp config path injected");
        let body = std::fs::read_to_string(path).expect("temp mcp config readable");
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();

        assert_eq!(parsed["mcpServers"]["shell-env"]["command"], format!("{home}/bin/mcp"));
        assert_eq!(parsed["mcpServers"]["shell-env"]["args"][1], format!("{home}/state"));
        assert_eq!(
            parsed["mcpServers"]["shell-env"]["env"]["TOKEN_FILE"],
            format!("{home}/token")
        );

        let _ = std::fs::remove_file(path);
    }

    /// K-748: the resolved claude MCP config — carrying expanded header
    /// secrets — must reach the vendor as a 0600 temp-file PATH, never
    /// as inline argv JSON (argv is world-readable via
    /// `/proc/<pid>/cmdline`).
    #[test]
    fn claude_mcp_config_written_to_owner_only_temp_file_not_argv() {
        let command = build_command(
            &resolved(AgentProvider::ClaudeCode),
            None,
            &[remote_mcp_def_with_headers()],
            Vec::new(),
            None,
        )
        .unwrap();
        let path = command
            .args
            .windows(2)
            .find_map(|w| (w[0] == "--mcp-config").then_some(w[1].as_str()))
            .expect("claude mcp config path injected");

        // The argv token is a filesystem path — never the inline JSON
        // or the header secret it references.
        assert!(
            !path.contains("mcpServers"),
            "argv must not carry inline MCP JSON: {path}"
        );
        assert!(
            !path.contains("Authorization") && !path.contains("Bearer"),
            "argv must not carry the MCP header secret: {path}"
        );

        let body = std::fs::read_to_string(path).expect("temp mcp config readable");
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["mcpServers"]["github"]["url"], "https://example.test/mcp");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "temp mcp config must be owner-only (0600)");
        }

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn claude_mcp_config_adds_http_type_for_url_servers() {
        let config = claude_mcp_config(&[remote_mcp_def_with_headers()]).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&config).unwrap();

        assert_eq!(parsed["mcpServers"]["github"]["type"], "http");
        assert_eq!(parsed["mcpServers"]["github"]["url"], "https://example.test/mcp");
    }

    #[test]
    fn claude_mcp_permission_globs_map_to_allowed_and_disallowed_tools() {
        let command = build_command(
            &resolved(AgentProvider::ClaudeCode),
            None,
            &[mcp_def_with_permissions()],
            Vec::new(),
            None,
        )
        .unwrap();
        let allowed = command
            .args
            .windows(2)
            .find_map(|w| (w[0] == "--allowedTools").then_some(w[1].as_str()))
            .expect("allowed tools injected");
        let disallowed = command
            .args
            .windows(2)
            .find_map(|w| (w[0] == "--disallowedTools").then_some(w[1].as_str()))
            .expect("disallowed tools injected");

        assert_eq!(allowed, "mcp__filesystem__list_*,mcp__filesystem__read_*");
        assert_eq!(disallowed, "mcp__filesystem__delete_*,mcp__filesystem__write_*");
    }

    #[test]
    fn claude_mcp_visibility_globs_map_to_allowed_and_disallowed_tools() {
        let command = build_command(
            &resolved(AgentProvider::ClaudeCode),
            None,
            &[mcp_def_with_visibility()],
            Vec::new(),
            None,
        )
        .unwrap();
        let allowed = command
            .args
            .windows(2)
            .find_map(|w| (w[0] == "--allowedTools").then_some(w[1].as_str()))
            .expect("allowed tools injected");
        let disallowed = command
            .args
            .windows(2)
            .find_map(|w| (w[0] == "--disallowedTools").then_some(w[1].as_str()))
            .expect("disallowed tools injected");

        assert_eq!(allowed, "mcp__filesystem__list_*,mcp__filesystem__read_file");
        assert_eq!(disallowed, "mcp__filesystem__delete_file,mcp__filesystem__write_*");
    }

    #[test]
    fn claude_provider_args_suppress_generated_mcp_permissions() {
        let command = build_command(
            &resolved(AgentProvider::ClaudeCode),
            None,
            &[mcp_def_with_permissions()],
            vec![
                "--allowed-tools".into(),
                "Read".into(),
                "--disallowedTools".into(),
                "Bash".into(),
            ],
            None,
        )
        .unwrap();

        assert_eq!(
            command
                .args
                .iter()
                .filter(|arg| matches!(arg.as_str(), "--allowedTools" | "--allowed-tools"))
                .count(),
            1
        );
        assert_eq!(
            command
                .args
                .iter()
                .filter(|arg| matches!(arg.as_str(), "--disallowedTools" | "--disallowed-tools"))
                .count(),
            1
        );
    }

    #[test]
    fn claude_emits_model_effort_mode_and_system_prompt_flags() {
        let command = build_command(
            &resolved(AgentProvider::ClaudeCode),
            Some("be terse"),
            &[],
            vec![],
            None,
        )
        .unwrap();

        assert!(command.args.windows(2).any(|w| w == ["--model", "model-a"]));
        assert!(command.args.windows(2).any(|w| w == ["--effort", "high"]));
        assert!(command.args.windows(2).any(|w| w == ["--permission-mode", "plan"]));
        assert!(command
            .args
            .windows(2)
            .any(|w| w == ["--append-system-prompt", "be terse"]));
    }

    #[test]
    fn claude_omits_append_system_prompt_when_empty() {
        let command = build_command(&resolved(AgentProvider::ClaudeCode), Some(""), &[], vec![], None).unwrap();

        assert!(!command.args.iter().any(|arg| arg == "--append-system-prompt"));
    }

    #[test]
    fn claude_effort_and_mode_deduped_when_provider_args_set_them() {
        let command = build_command(
            &resolved(AgentProvider::ClaudeCode),
            None,
            &[],
            vec![
                "--effort".into(),
                "low".into(),
                "--permission-mode".into(),
                "acceptEdits".into(),
            ],
            None,
        )
        .unwrap();

        assert_eq!(command.args.iter().filter(|arg| arg.as_str() == "--effort").count(), 1);
        assert_eq!(
            command
                .args
                .iter()
                .filter(|arg| arg.as_str() == "--permission-mode")
                .count(),
            1
        );
    }

    #[test]
    fn claude_skips_transportless_mcp_def() {
        let config = claude_mcp_config(&[transportless_mcp_def(), mcp_def()]).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&config).unwrap();

        assert!(
            parsed["mcpServers"].get("broken").is_none(),
            "transportless def must be skipped for parity with codex/opencode"
        );
        assert!(
            parsed["mcpServers"].get("hyprpilot-nvim").is_some(),
            "a valid def alongside a transportless one still serialises"
        );
    }

    #[test]
    fn claude_preserves_sse_transport_type() {
        let config = claude_mcp_config(&[sse_mcp_def()]).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&config).unwrap();

        assert_eq!(parsed["mcpServers"]["events"]["type"], "sse");
        assert_eq!(parsed["mcpServers"]["events"]["url"], "https://example.test/sse");
    }

    #[test]
    fn claude_headless_projects_print_and_prompt_on_stdin() {
        // `claude --print` reading the prompt from STDIN — `--print`
        // present, model/mode projection still applies, the prompt rides
        // `stdin_prompt` (spawn path), and it is NEVER a trailing
        // positional the variadic tool flags could swallow.
        let command = build_command(
            &resolved(AgentProvider::ClaudeCode),
            None,
            &[],
            vec![],
            Some("fix the bug"),
        )
        .unwrap();

        assert!(command.args.iter().any(|a| a == "--print"), "{:?}", command.args);
        assert!(command.args.iter().any(|a| a == "--model"), "model still projected");
        assert_eq!(
            command.stdin_prompt.as_deref(),
            Some("fix the bug"),
            "prompt is delivered on stdin"
        );
        assert!(
            !command.args.iter().any(|a| a == "fix the bug"),
            "prompt must NOT be a positional arg: {:?}",
            command.args
        );
    }

    #[test]
    fn claude_headless_prompt_survives_variadic_tool_flags() {
        // With tool policy present, `--allowedTools`/`--disallowedTools`
        // are the last flags in argv — the prompt must still reach the
        // model via stdin, not be appended where those variadic flags
        // would eat it.
        let command = build_command(
            &resolved(AgentProvider::ClaudeCode),
            None,
            &[mcp_def_with_permissions()],
            vec![],
            Some("do the thing"),
        )
        .unwrap();

        assert!(command.args.iter().any(|a| a == "--allowedTools"), "{:?}", command.args);
        assert_eq!(command.stdin_prompt.as_deref(), Some("do the thing"));
        assert!(
            !command.args.iter().any(|a| a == "do the thing"),
            "prompt must not trail the variadic tool flags: {:?}",
            command.args
        );
    }

    #[test]
    fn claude_interactive_has_no_print_or_stdin_prompt() {
        // No prompt → interactive path is byte-for-byte unchanged (no
        // `--print`, no stdin prompt, plain `exec()`).
        let command = build_command(&resolved(AgentProvider::ClaudeCode), None, &[], vec![], None).unwrap();

        assert!(!command.args.iter().any(|a| a == "--print"), "{:?}", command.args);
        assert_eq!(command.stdin_prompt, None);
        assert_eq!(command.args.last().map(String::as_str), Some("plan")); // --permission-mode plan
    }
}

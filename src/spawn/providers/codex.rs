//! Codex (`codex`) native config-override projection.

use std::collections::BTreeSet;

use anyhow::{bail, Result};

use crate::mcp::{project_transport, MCPDefinition, McpTransport};
use crate::resolve::profile::ResolvedProfile;

use super::argv::{combined_args, has_config_override, has_flag};
use super::{base_command, ensure_inline_size, is_exact_tool_name, mcp_leaf_pattern, HarnessProjection, SpawnCommand};

const CODEX_APPROVAL_POLICIES: &[&str] = &["untrusted", "on-request", "never", "on-failure"];
const CODEX_SANDBOX_MODES: &[&str] = &["read-only", "workspace-write", "danger-full-access"];

pub(super) fn build_codex(
    resolved: &ResolvedProfile,
    system_prompt: Option<&str>,
    mcp_defs: &[MCPDefinition],
    provider_args: Vec<String>,
    prompt: Option<&str>,
    harness: Option<&HarnessProjection>,
) -> Result<SpawnCommand> {
    let mut command = base_command(resolved);
    // Headless: `codex exec [OPTIONS]` reading the prompt from STDIN —
    // the non-interactive subcommand. `exec` is prepended before every
    // generated `-c` / `--model` / `--cd` / `-s` option (all valid
    // `codex exec` flags). The prompt is delivered on the child's stdin
    // (see `stdin_prompt` below), NOT as a positional: hyprpilot spawns
    // codex and writes the prompt to a fresh pipe, then closes it (EOF),
    // so `codex exec` reads it without the non-TTY hang that an inherited
    // already-at-EOF fd0 or an open-but-idle pipe would cause.
    let headless = prompt.is_some();
    if headless {
        command.args.insert(0, "exec".into());
    }
    // `resume <SESSION_ID>` is a SUBCOMMAND of `exec`, not a flag, so it
    // has to sit immediately after `exec` and before every generated
    // option — unlike claude's `--resume`, which is order-free. Inserted
    // in reverse so `exec resume <id>` lands in that order.
    if let Some(id) = harness.and_then(|h| h.resume.as_deref()) {
        if headless && !has_flag(&command.args, "resume", None) {
            command.args.insert(1, "resume".into());
            command.args.insert(2, id.into());
        }
    }
    let detect_args = combined_args(&command.args, &provider_args);

    if let Some(harness) = harness {
        // Codex names it `thread_id` on the `thread.started` event, not
        // `session_id` — the harness parses that key back out. Verified
        // against the installed CLI.
        if harness.structured_output && !has_flag(&detect_args, "--json", None) {
            command.args.push("--json".into());
        }
    }

    if let Some(cwd) = command.cwd.as_ref() {
        if !has_flag(&detect_args, "--cd", Some("-C")) {
            command.args.push("--cd".into());
            command.args.push(cwd.display().to_string());
        }
    }
    if let Some(model) = resolved.model.as_deref() {
        if !has_flag(&detect_args, "--model", Some("-m")) {
            command.args.push("--model".into());
            command.args.push(model.into());
        }
    }
    if let Some(effort) = resolved.effort.as_deref() {
        push_codex_config_if_absent(
            &mut command.args,
            &detect_args,
            "model_reasoning_effort",
            toml_string(effort),
        );
    }
    if let Some(mode) = resolved.mode.as_deref() {
        push_codex_mode_if_absent(&mut command.args, &detect_args, mode, headless)?;
    }
    if let Some(prompt) = system_prompt {
        if !prompt.is_empty() {
            ensure_inline_size("codex -c instructions", prompt)?;
            push_codex_config_if_absent(&mut command.args, &detect_args, "instructions", toml_string(prompt));
        }
    }
    for (key, value) in codex_mcp_config_entries(mcp_defs) {
        push_codex_config_if_absent(&mut command.args, &detect_args, &key, value);
    }
    for (key, value) in codex_mcp_tool_policy_entries(mcp_defs) {
        push_codex_config_if_absent(&mut command.args, &detect_args, &key, value);
    }

    command.args.extend(provider_args);
    // Deliver the headless prompt on stdin (see the `exec` comment
    // above) — never as a positional.
    command.stdin_prompt = prompt.map(str::to_string);

    Ok(command)
}

fn push_codex_config_if_absent(args: &mut Vec<String>, detect_args: &[String], key: &str, value: String) {
    if has_config_override(detect_args, key) {
        return;
    }

    args.push("-c".into());
    args.push(format!("{key}={value}"));
}

fn push_codex_mode_if_absent(args: &mut Vec<String>, detect_args: &[String], mode: &str, headless: bool) -> Result<()> {
    let mode = mode.trim();
    if CODEX_APPROVAL_POLICIES.contains(&mode) {
        // `codex exec` is non-interactive — approval is always "never"
        // and the subcommand rejects `--ask-for-approval` outright.
        // Drop the approval-policy mode (warn) instead of shipping a
        // flag codex will error on; sandbox modes below still project
        // via `-s`, which IS a valid `codex exec` flag.
        if headless {
            tracing::warn!(
                mode,
                "cli spawn: codex exec is non-interactive (approval always 'never'); ignoring approval-policy mode in headless launch"
            );

            return Ok(());
        }
        if !has_flag(detect_args, "--ask-for-approval", Some("-a")) {
            args.push("--ask-for-approval".into());
            args.push(mode.into());
        }

        return Ok(());
    }

    if CODEX_SANDBOX_MODES.contains(&mode) {
        if !has_flag(detect_args, "--sandbox", Some("-s")) {
            args.push("--sandbox".into());
            args.push(mode.into());
        }

        return Ok(());
    }

    if has_codex_mode_override(detect_args) {
        tracing::warn!(
            mode,
            "cli spawn: ignoring unsupported codex profile mode because provider args override codex approval/sandbox policy"
        );

        return Ok(());
    }

    bail!(
        "codex spawn mode '{mode}' is not supported by Codex CLI; use an approval policy ({}) or sandbox mode ({})",
        CODEX_APPROVAL_POLICIES.join(", "),
        CODEX_SANDBOX_MODES.join(", ")
    );
}

fn has_codex_mode_override(args: &[String]) -> bool {
    has_flag(args, "--ask-for-approval", Some("-a"))
        || has_flag(args, "--sandbox", Some("-s"))
        || has_flag(args, "--dangerously-bypass-approvals-and-sandbox", None)
}

#[derive(Debug, Default)]
struct CodexToolPolicy {
    default_approval: bool,
    approval_tools: BTreeSet<String>,
    enabled_tools: Option<BTreeSet<String>>,
    disabled_tools: BTreeSet<String>,
}

fn codex_mcp_tool_policy_entries(defs: &[MCPDefinition]) -> Vec<(String, String)> {
    let mut entries = Vec::new();
    for def in defs {
        let policy = codex_mcp_tool_policy(def);
        let prefix = toml_key_path(&["mcp_servers", &def.name]);

        if policy.default_approval {
            entries.push((format!("{prefix}.default_tools_approval_mode"), toml_string("approve")));
        }
        for tool in policy.approval_tools {
            entries.push((
                toml_key_path(&["mcp_servers", &def.name, "tools", &tool, "approval_mode"]),
                toml_string("approve"),
            ));
        }
        if let Some(enabled_tools) = policy.enabled_tools {
            entries.push((
                format!("{prefix}.enabled_tools"),
                toml_array(&enabled_tools.into_iter().collect::<Vec<_>>()),
            ));
        }
        if !policy.disabled_tools.is_empty() {
            entries.push((
                format!("{prefix}.disabled_tools"),
                toml_array(&policy.disabled_tools.into_iter().collect::<Vec<_>>()),
            ));
        }
    }

    entries
}

fn codex_mcp_tool_policy(def: &MCPDefinition) -> CodexToolPolicy {
    let mut policy = CodexToolPolicy::default();

    if let Some(include) = def.hyprpilot.include_tools.as_ref() {
        let mut enabled = BTreeSet::new();
        let mut all_tools = false;
        for pattern in include {
            match mcp_leaf_pattern(&def.name, pattern) {
                Some("*") => {
                    all_tools = true;
                }
                Some(leaf) if is_exact_tool_name(leaf) => {
                    enabled.insert(leaf.to_string());
                }
                Some(leaf) => {
                    tracing::warn!(
                        server = %def.name,
                        pattern = %leaf,
                        "cli spawn: skipping codex includeTools glob; Codex enabled_tools supports exact tool names only"
                    );
                }
                None => {}
            }
        }
        if !all_tools {
            policy.enabled_tools = Some(enabled);
        }
    }

    for pattern in def
        .hyprpilot
        .exclude_tools
        .iter()
        .chain(def.hyprpilot.auto_reject_tools.iter())
    {
        match mcp_leaf_pattern(&def.name, pattern) {
            Some("*") => {
                policy.enabled_tools = Some(BTreeSet::new());
                policy.disabled_tools.clear();
            }
            Some(leaf)
                if is_exact_tool_name(leaf)
                    && policy
                        .enabled_tools
                        .as_ref()
                        .map_or(true, |enabled| !enabled.is_empty()) =>
            {
                policy.disabled_tools.insert(leaf.to_string());
            }
            Some(leaf) => {
                tracing::warn!(
                    server = %def.name,
                    pattern = %leaf,
                    "cli spawn: skipping codex reject/exclude glob; Codex disabled_tools supports exact tool names only"
                );
            }
            None => {}
        }
    }

    if policy.enabled_tools.as_ref().is_some_and(BTreeSet::is_empty) {
        return policy;
    }

    for pattern in &def.hyprpilot.auto_accept_tools {
        match mcp_leaf_pattern(&def.name, pattern) {
            Some("*") => {
                policy.default_approval = true;
            }
            Some(leaf) if is_exact_tool_name(leaf) => {
                policy.approval_tools.insert(leaf.to_string());
            }
            Some(leaf) => {
                tracing::warn!(
                    server = %def.name,
                    pattern = %leaf,
                    "cli spawn: skipping codex autoAcceptTools glob; Codex tool approval overrides support exact tool names only"
                );
            }
            None => {}
        }
    }

    policy
}

fn codex_mcp_config_entries(defs: &[MCPDefinition]) -> Vec<(String, String)> {
    let mut entries = Vec::new();
    for def in defs {
        let Some(server) = project_transport(def) else {
            tracing::warn!(name = %def.name, "cli spawn: skipping MCP entry without command or url for codex");
            continue;
        };
        match server {
            McpTransport::Stdio {
                name,
                command,
                args,
                env,
            } => {
                let prefix = toml_key_path(&["mcp_servers", &name]);
                entries.push((format!("{prefix}.command"), toml_string(&command.to_string_lossy())));
                if !args.is_empty() {
                    entries.push((format!("{prefix}.args"), toml_array(&args)));
                }
                for (key, value) in env {
                    entries.push((toml_key_path(&["mcp_servers", &name, "env", &key]), toml_string(&value)));
                }
            }
            McpTransport::Http { name, url, headers } => {
                let prefix = toml_key_path(&["mcp_servers", &name]);
                entries.push((format!("{prefix}.url"), toml_string(&url)));
                for (key, value) in headers {
                    push_codex_http_header(&mut entries, def, &name, &key, &value);
                }
            }
            McpTransport::Sse { name, url, headers } => {
                // Codex's config exposes only a bare `.url` remote-MCP
                // shape — no distinct SSE discriminator. Project as
                // HTTP but warn loudly so the downgrade is not silent.
                tracing::warn!(
                    server = %name,
                    "cli spawn: codex has no distinct SSE MCP transport; projecting `type=sse` server as HTTP (url + headers)"
                );
                let prefix = toml_key_path(&["mcp_servers", &name]);
                entries.push((format!("{prefix}.url"), toml_string(&url)));
                for (key, value) in headers {
                    push_codex_http_header(&mut entries, def, &name, &key, &value);
                }
            }
        }
    }

    entries
}

fn push_codex_http_header(
    entries: &mut Vec<(String, String)>,
    def: &MCPDefinition,
    server: &str,
    header: &str,
    expanded_value: &str,
) {
    if let Some(var) = raw_header_value(def, header).and_then(bearer_env_reference) {
        if header.eq_ignore_ascii_case("authorization") {
            entries.push((
                toml_key_path(&["mcp_servers", server, "bearer_token_env_var"]),
                toml_string(&var),
            ));
            return;
        }
    }

    if let Some(var) = raw_header_value(def, header).and_then(env_reference) {
        entries.push((
            toml_key_path(&["mcp_servers", server, "env_http_headers", header]),
            toml_string(&var),
        ));
        return;
    }

    // A literal (non-`${VAR}`) header value lands verbatim in
    // `http_headers`, which codex projects onto argv (`-c
    // mcp_servers.<server>.http_headers.<header>="value"`) — and argv is
    // world-readable via `/proc/<pid>/cmdline`. A `${VAR}` env reference
    // instead rides the `env_http_headers` / `bearer_token_env_var` path
    // above and keeps the value out of argv. Warn so a captain moves an
    // inline secret behind an env reference (we do NOT synthesize one —
    // that would change the projection shape).
    tracing::warn!(
        server = %server,
        header = %header,
        "cli spawn: codex MCP header value is a literal projected onto argv; put MCP secrets in a `${{VAR}}` env reference, not inline — a literal value is projected onto argv"
    );
    entries.push((
        toml_key_path(&["mcp_servers", server, "http_headers", header]),
        toml_string(expanded_value),
    ));
}

fn raw_header_value<'a>(def: &'a MCPDefinition, header: &str) -> Option<&'a str> {
    let headers = def.raw.get("headers")?.as_object()?;
    headers.get(header).and_then(serde_json::Value::as_str).or_else(|| {
        headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(header))
            .and_then(|(_, value)| value.as_str())
    })
}

fn bearer_env_reference(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let (scheme, value) = trimmed.split_once(char::is_whitespace)?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }

    env_reference(value.trim())
}

fn env_reference(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let candidate = if let Some(inner) = trimmed.strip_prefix("${").and_then(|value| value.strip_suffix('}')) {
        inner.strip_prefix("env:").unwrap_or(inner)
    } else {
        trimmed.strip_prefix('$')?
    };

    is_env_name(candidate).then(|| candidate.to_string())
}

fn is_env_name(candidate: &str) -> bool {
    !candidate.is_empty() && candidate.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn toml_key_path(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|part| {
            if is_bare_toml_key(part) {
                (*part).to_string()
            } else {
                serde_json::to_string(part).expect("str always serializes")
            }
        })
        .collect::<Vec<_>>()
        .join(".")
}

fn is_bare_toml_key(part: &str) -> bool {
    !part.is_empty()
        && part
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
}

fn toml_string(value: &str) -> String {
    serde_json::to_string(value).expect("str always serializes")
}

fn toml_array(values: &[String]) -> String {
    let body = values
        .iter()
        .map(|value| toml_string(value))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{body}]")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AgentProvider;
    use crate::spawn::providers::build_command_cli as build_command;
    use crate::spawn::providers::fixtures::*;

    #[test]
    fn codex_projects_mcp_into_config_overrides() {
        let command = build_command(
            &resolved_with_mode(AgentProvider::Codex, Some("on-request")),
            Some("be terse"),
            &[mcp_def()],
            Vec::new(),
            None,
        )
        .unwrap();
        let joined = command.args.join("\n");

        assert!(joined.contains("instructions=\"be terse\""));
        assert!(joined.contains("mcp_servers.hyprpilot-nvim.command=\"uvx\""));
        assert!(joined.contains("mcp_servers.hyprpilot-nvim.args=[\"hyprpilot-nvim-mcp\"]"));
        assert!(joined.contains("mcp_servers.hyprpilot-nvim.env.NVIM=\"/tmp/nvim.sock\""));
    }

    #[test]
    fn codex_projects_remote_mcp_headers_into_supported_config() {
        let entries = codex_mcp_config_entries(&[remote_mcp_def_with_headers()]);
        let rendered = entries
            .into_iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("mcp_servers.github.url=\"https://example.test/mcp\""));
        assert!(rendered.contains("mcp_servers.github.bearer_token_env_var=\"NVIM_GITHUB\""));
        assert!(rendered.contains("mcp_servers.github.http_headers.X-MCP-Insiders=\"true\""));
        assert!(rendered.contains("mcp_servers.github.env_http_headers.x-api-key=\"EXA_API_KEY\""));
        assert!(!rendered.contains("mcp_servers.github.headers."));
        assert!(!rendered.contains("Bearer "));
    }

    #[test]
    fn codex_projects_exact_mcp_tool_policy_into_supported_config() {
        let command = build_command(
            &resolved_with_mode(AgentProvider::Codex, Some("on-request")),
            None,
            &[mcp_def_with_visibility()],
            Vec::new(),
            None,
        )
        .unwrap();
        let joined = command.args.join("\n");

        assert!(joined.contains("mcp_servers.filesystem.enabled_tools=[\"read_file\"]"));
        assert!(joined.contains("mcp_servers.filesystem.disabled_tools=[\"delete_file\"]"));
        assert!(joined.contains("mcp_servers.filesystem.tools.read_file.approval_mode=\"approve\""));
        assert!(!joined.contains("list_*"));
        assert!(!joined.contains("write_*"));
    }

    #[test]
    fn codex_approval_mode_maps_to_ask_for_approval() {
        let command = build_command(
            &resolved_with_mode(AgentProvider::Codex, Some("on-request")),
            None,
            &[],
            Vec::new(),
            None,
        )
        .unwrap();

        assert!(command
            .args
            .windows(2)
            .any(|w| w == ["--ask-for-approval", "on-request"]));
    }

    #[test]
    fn codex_passes_resolved_cwd_to_native_cd_flag() {
        let command = build_command(
            &resolved_with_cwd(AgentProvider::Codex, "/tmp/hyprpilot-work"),
            None,
            &[],
            Vec::new(),
            None,
        )
        .unwrap();

        assert!(command.args.windows(2).any(|w| w == ["--cd", "/tmp/hyprpilot-work"]));
        assert_eq!(command.cwd(), Some(std::path::Path::new("/tmp/hyprpilot-work")));
    }

    #[test]
    fn codex_provider_args_suppress_generated_cd_flag() {
        let command = build_command(
            &resolved_with_cwd(AgentProvider::Codex, "/tmp/hyprpilot-work"),
            None,
            &[],
            vec!["-C".into(), "/tmp/other".into()],
            None,
        )
        .unwrap();

        assert_eq!(command.args.iter().filter(|arg| arg.as_str() == "--cd").count(), 0);
        assert_eq!(command.args.iter().filter(|arg| arg.as_str() == "-C").count(), 1);
    }

    #[test]
    fn codex_sandbox_mode_maps_to_sandbox() {
        let command = build_command(
            &resolved_with_mode(AgentProvider::Codex, Some("workspace-write")),
            None,
            &[],
            Vec::new(),
            None,
        )
        .unwrap();

        assert!(command.args.windows(2).any(|w| w == ["--sandbox", "workspace-write"]));
    }

    #[test]
    fn codex_rejects_unknown_mode_without_provider_override() {
        let err = build_command(
            &resolved_with_mode(AgentProvider::Codex, Some("plan")),
            None,
            &[],
            Vec::new(),
            None,
        )
        .expect_err("unknown mode should fail before spawning codex");

        assert!(err.to_string().contains("codex spawn mode 'plan'"), "{err}");
    }

    #[test]
    fn codex_provider_args_override_unknown_mode() {
        let command = build_command(
            &resolved_with_mode(AgentProvider::Codex, Some("plan")),
            None,
            &[],
            vec!["--ask-for-approval".into(), "never".into()],
            None,
        )
        .unwrap();

        assert_eq!(
            command
                .args
                .iter()
                .filter(|arg| arg.as_str() == "--ask-for-approval")
                .count(),
            1
        );
    }

    #[test]
    fn codex_emits_model_reasoning_effort() {
        let command = build_command(
            &resolved_with_mode(AgentProvider::Codex, Some("on-request")),
            None,
            &[],
            vec![],
            None,
        )
        .unwrap();

        assert!(command.args.join("\n").contains("model_reasoning_effort=\"high\""));
    }

    #[test]
    fn codex_instructions_deduped_when_provider_arg_sets_it() {
        let command = build_command(
            &resolved_with_mode(AgentProvider::Codex, Some("on-request")),
            Some("generated prompt"),
            &[],
            vec!["-c".into(), "instructions=\"user\"".into()],
            None,
        )
        .unwrap();
        let joined = command.args.join("\n");

        assert!(
            !joined.contains("instructions=\"generated prompt\""),
            "generated instructions must be suppressed when a provider arg sets the key"
        );
        assert!(joined.contains("instructions=\"user\""));
        assert_eq!(
            command
                .args
                .iter()
                .filter(|arg| arg.starts_with("instructions="))
                .count(),
            1
        );
    }

    #[test]
    fn codex_model_deduped_when_provider_arg_sets_it() {
        let command = build_command(
            &resolved_with_mode(AgentProvider::Codex, Some("on-request")),
            None,
            &[],
            vec!["--model".into(), "user-model".into()],
            None,
        )
        .unwrap();

        assert_eq!(command.args.iter().filter(|arg| arg.as_str() == "--model").count(), 1);
    }

    #[test]
    fn codex_projects_sse_as_http_url() {
        let entries = codex_mcp_config_entries(&[sse_mcp_def()]);
        let rendered = entries
            .into_iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("mcp_servers.events.url=\"https://example.test/sse\""));
    }

    #[test]
    fn codex_headless_projects_exec_subcommand_and_prompt_on_stdin() {
        let command = build_command(
            &resolved_with_mode(AgentProvider::Codex, None),
            None,
            &[],
            vec![],
            Some("summarize the diff"),
        )
        .unwrap();

        assert_eq!(
            command.args.first().map(String::as_str),
            Some("exec"),
            "exec is the subcommand"
        );
        assert!(command.args.iter().any(|a| a == "--model"), "model still projected");
        assert_eq!(
            command.stdin_prompt.as_deref(),
            Some("summarize the diff"),
            "prompt is delivered on stdin"
        );
        assert!(
            !command.args.iter().any(|a| a == "summarize the diff"),
            "prompt must NOT be a positional arg: {:?}",
            command.args
        );
    }

    #[test]
    fn codex_headless_skips_approval_policy_mode() {
        // `codex exec` has no `--ask-for-approval` (approval is always
        // "never") — an approval-policy mode must be dropped, not shipped.
        let command = build_command(
            &resolved_with_mode(AgentProvider::Codex, Some("on-request")),
            None,
            &[],
            vec![],
            Some("do it"),
        )
        .unwrap();

        assert!(
            !command.args.iter().any(|a| a == "--ask-for-approval"),
            "approval-policy mode must be dropped in headless: {:?}",
            command.args
        );
        assert_eq!(command.args.first().map(String::as_str), Some("exec"));
    }

    #[test]
    fn codex_headless_keeps_sandbox_mode() {
        // Sandbox modes ARE valid `codex exec` flags — they still project.
        let command = build_command(
            &resolved_with_mode(AgentProvider::Codex, Some("read-only")),
            None,
            &[],
            vec![],
            Some("do it"),
        )
        .unwrap();

        let sandbox = command
            .args
            .windows(2)
            .find_map(|w| (w[0] == "--sandbox").then_some(w[1].as_str()));
        assert_eq!(sandbox, Some("read-only"), "{:?}", command.args);
    }

    #[test]
    fn codex_interactive_still_projects_approval_policy_mode() {
        // Interactive path is unchanged — approval-policy modes map to
        // `--ask-for-approval`.
        let command = build_command(
            &resolved_with_mode(AgentProvider::Codex, Some("on-request")),
            None,
            &[],
            vec![],
            None,
        )
        .unwrap();

        let approval = command
            .args
            .windows(2)
            .find_map(|w| (w[0] == "--ask-for-approval").then_some(w[1].as_str()));
        assert_eq!(approval, Some("on-request"));
        assert!(
            !command.args.iter().any(|a| a == "exec"),
            "no exec subcommand interactively"
        );
    }
}

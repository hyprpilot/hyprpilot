//! opencode (`opencode`) projection: `--standalone`, plus every profile
//! knob as native V2 config in `OPENCODE_CONFIG_CONTENT`.

use anyhow::{bail, Context, Result};
use serde_json::{json, Map, Value};

use crate::mcp::{MCPDefinition, McpTransport};
use crate::resolve::profile::ResolvedProfile;

use super::argv::{combined_args, flag_value, has_flag};
use super::{
    base_command, ensure_inline_size, mcp_leaf_pattern, mcp_transports, HarnessProjection, Resume, SpawnCommand,
};

/// Agent the generated prompt and model hang off when the profile names
/// no `mode`.
const DEFAULT_AGENT: &str = "hyprpilot";

pub(super) fn build_opencode(
    resolved: &ResolvedProfile,
    system_prompt: Option<&str>,
    mcp_defs: &[MCPDefinition],
    provider_args: Vec<String>,
    prompt: Option<&str>,
    resume: Option<&Resume>,
    harness: Option<&HarnessProjection>,
) -> Result<SpawnCommand> {
    let mut command = base_command(resolved);
    // Headless: `opencode run [OPTIONS] <prompt>` — the one-shot
    // subcommand. opencode has no stdin prompt support, so the buffered
    // prompt is the positional `message`.
    if prompt.is_some() {
        command.args.insert(0, "run".into());
    }
    let detect_args = combined_args(&command.args, &provider_args);

    // The shared background service keeps the env of whichever client
    // started it, not this launch's. A `--server` is the captain's choice.
    if !has_flag(&detect_args, "--standalone", None) && !has_flag(&detect_args, "--server", None) {
        command.args.push("--standalone".into());
    }

    if let Some(harness) = harness {
        // opencode spells it `sessionID` (camelCase) on every event —
        // a third distinct key across the three vendors. Verified
        // against the installed CLI.
        if harness.structured_output && !has_flag(&detect_args, "--format", None) {
            command.args.push("--format".into());
            command.args.push("json".into());
        }
    }

    // Both spellings guard each other: `--session <id>` and `--continue`
    // are the same choice made two ways.
    if let Some(resume) = resume {
        if !has_flag(&detect_args, "--session", Some("-s")) && !has_flag(&detect_args, "--continue", Some("-c")) {
            match resume {
                Resume::Last => command.args.push("--continue".into()),
                Resume::Session(id) => {
                    command.args.push("--session".into());
                    command.args.push(id.clone());
                }
                // opencode registers no picker on either the TUI or
                // `run` — only `--continue` and `--session <id>` — so
                // there is nothing to project and a silent fall back to
                // the most recent session would resume the wrong one.
                Resume::Picker => bail!(
                    "opencode has no session picker to open: resume the most recent with `--resume-last`, \
                     or name one with `--resume=<session-id>`"
                ),
            }
        }
    }

    // `run` resolves its directory from `$PWD` before the process cwd.
    // The resolved cwd wins over a preset `PWD`: it is the directory
    // hyprpilot reports.
    if let Some(cwd) = command.cwd.as_ref() {
        command.env.insert("PWD".into(), cwd.display().to_string());
    }

    // The TUI takes no `--model` / `--agent` (it rejects both), so the
    // model and agent reach every launch as config. `run` takes both
    // flags as well, and without `--agent` it follows up its answer with
    // a second step under the default agent.
    let agent_name = flag_value(&detect_args, "--agent", None).or_else(|| resolved.mode.clone());
    if !command.env.contains_key("OPENCODE_CONFIG_CONTENT") {
        if let Some(config) = opencode_config_content(
            agent_name.as_deref(),
            resolved.model.as_deref(),
            resolved.effort.as_deref(),
            system_prompt,
            mcp_defs,
        )? {
            ensure_inline_size("OPENCODE_CONFIG_CONTENT", &config)?;
            command.env.insert("OPENCODE_CONFIG_CONTENT".into(), config);

            if prompt.is_some() {
                let model = opencode_model_ref(resolved.model.as_deref(), resolved.effort.as_deref())?;
                if !has_flag(&detect_args, "--agent", None) {
                    command.args.push("--agent".into());
                    command.args.push(agent_name.unwrap_or_else(|| DEFAULT_AGENT.into()));
                }
                if let Some(model) = model.filter(|_| !has_flag(&detect_args, "--model", Some("-m"))) {
                    command.args.push("--model".into());
                    command.args.push(model);
                }
            }
        }
    } else if system_prompt.is_some()
        || resolved.model.is_some()
        || resolved.effort.is_some()
        || agent_name.is_some()
        || !mcp_defs.is_empty()
    {
        tracing::warn!(
            "cli spawn: OPENCODE_CONFIG_CONTENT already set by agent env; the profile's model, agent, system prompt, MCP servers and tool permissions are NOT projected"
        );
    }

    command.args.extend(provider_args);
    if let Some(prompt) = prompt {
        command.args.push(prompt.into());
    }

    Ok(command)
}

fn opencode_config_content(
    agent_name: Option<&str>,
    model: Option<&str>,
    effort: Option<&str>,
    system_prompt: Option<&str>,
    mcp_defs: &[MCPDefinition],
) -> Result<Option<String>> {
    let mut root = Map::new();
    let mut agent = Map::new();
    if let Some(prompt) = system_prompt.filter(|prompt| !prompt.is_empty()) {
        agent.insert("system".into(), Value::String(prompt.into()));
    }
    if let Some(model) = opencode_model_ref(model, effort)? {
        agent.insert("model".into(), Value::String(model.clone()));
        // The top-level default too, so switching agents in the TUI
        // keeps the profile's model instead of falling back to opencode's.
        root.insert("model".into(), Value::String(model));
    }
    if !agent.is_empty() || agent_name.is_some() {
        let name = agent_name.unwrap_or(DEFAULT_AGENT);
        if !agent.is_empty() {
            root.insert("agents".into(), json!({ name: agent }));
        }
        root.insert("default_agent".into(), Value::String(name.into()));
    }

    let servers = opencode_mcp_servers(mcp_defs);
    if !servers.is_empty() {
        root.insert("mcp".into(), json!({ "servers": servers }));
    }
    let permissions = opencode_permissions(mcp_defs);
    if !permissions.is_empty() {
        root.insert("permissions".into(), Value::Array(permissions));
    }
    if root.is_empty() {
        return Ok(None);
    }

    serde_json::to_string(&Value::Object(root))
        .map(Some)
        .context("serialize opencode config")
}

/// The `provider/model#variant` string form of a V2 model selection, the
/// variant being the effort. A ref opencode cannot parse makes it drop the
/// whole agent, prompt included, without a word — so it fails here.
fn opencode_model_ref(model: Option<&str>, effort: Option<&str>) -> Result<Option<String>> {
    let reference = match (model, effort) {
        (Some(model), Some(effort)) if !model.contains('#') => Some(format!("{model}#{effort}")),
        (Some(model), Some(effort)) => {
            tracing::warn!(
                model,
                effort,
                "cli spawn: opencode model already names a variant; ignoring effort"
            );
            Some(model.into())
        }
        (Some(model), None) => Some(model.into()),
        (None, Some(effort)) => {
            tracing::warn!(
                effort,
                "cli spawn: opencode selects a variant only on a model reference; ignoring effort without a model"
            );
            None
        }
        (None, None) => None,
    };
    if let Some(reference) = reference.as_deref() {
        let valid = reference.split_once('/').is_some_and(|(provider, rest)| {
            let mut parts = rest.split('#');
            !provider.is_empty()
                && !provider.contains('#')
                && parts.next().is_some_and(|model| !model.is_empty())
                && parts.next().map_or(true, |variant| !variant.is_empty())
                && parts.next().is_none()
        });
        if !valid {
            bail!("opencode model `{reference}` is not a `provider/model[#variant]` reference");
        }
    }

    Ok(reference)
}

fn opencode_mcp_servers(defs: &[MCPDefinition]) -> Map<String, Value> {
    let mut out = Map::new();
    for (_, server) in mcp_transports(defs, "opencode") {
        match server {
            McpTransport::Stdio {
                name,
                command,
                args,
                env,
            } => {
                let mut cmd = Vec::with_capacity(1 + args.len());
                cmd.push(command.to_string_lossy().to_string());
                cmd.extend(args);
                let environment: Map<String, Value> = env
                    .into_iter()
                    .map(|(key, value)| (key, Value::String(value)))
                    .collect();
                let mut entry = json!({
                    "type": "local",
                    "command": cmd,
                });
                if !environment.is_empty() {
                    entry["environment"] = Value::Object(environment);
                }
                out.insert(name, entry);
            }
            McpTransport::Http { name, url, headers } => {
                out.insert(name, opencode_remote_mcp(url, headers));
            }
            McpTransport::Sse { name, url, headers } => {
                // opencode's `type: "remote"` MCP entry carries no SSE
                // discriminator — the SSE-ness is dropped. Warn so the
                // downgrade to HTTP-style remote is not silent.
                tracing::warn!(
                    server = %name,
                    "cli spawn: opencode has no distinct SSE MCP transport; projecting `type=sse` server as a `remote` (HTTP) server"
                );
                out.insert(name, opencode_remote_mcp(url, headers));
            }
        }
    }

    out
}

/// The MCP tool policy as opencode `permissions` rules. The ruleset is
/// LAST match wins, so the order below is the precedence: an include
/// list denies the whole server before allowing its entries back, and
/// reject / exclude come after every allow so they beat it.
fn opencode_permissions(defs: &[MCPDefinition]) -> Vec<Value> {
    let mut rules = Vec::new();
    for def in defs {
        let tool_rules = |patterns: &[String], effect: &str| -> Vec<Value> {
            patterns
                .iter()
                .filter_map(|pattern| opencode_mcp_tool_pattern(&def.name, pattern))
                .map(|action| opencode_rule(action, effect))
                .collect()
        };

        rules.extend(tool_rules(&def.hyprpilot.auto_accept_tools, "allow"));
        if let Some(include) = def.hyprpilot.include_tools.as_ref() {
            rules.push(opencode_rule(
                format!("{}_*", opencode_sanitize_tool_name(&def.name)),
                "deny",
            ));
            rules.extend(tool_rules(include, "allow"));
        }
        rules.extend(tool_rules(&def.hyprpilot.auto_reject_tools, "deny"));
        rules.extend(tool_rules(&def.hyprpilot.exclude_tools, "deny"));
    }

    rules
}

/// opencode asserts an MCP call as action `<server>_<tool>` over resource
/// `*`, so every rule targets the whole resource space.
fn opencode_rule(action: String, effect: &str) -> Value {
    json!({ "action": action, "resource": "*", "effect": effect })
}

fn opencode_mcp_tool_pattern(server: &str, pattern: &str) -> Option<String> {
    let leaf = mcp_leaf_pattern(server, pattern)?;
    let sanitized_leaf = opencode_sanitize_tool_pattern(leaf);
    if sanitized_leaf != leaf {
        tracing::warn!(
            server = %server,
            pattern = %leaf,
            sanitized = %sanitized_leaf,
            "cli spawn: opencode permission pattern altered by sanitization; character-class globs (e.g. `[abc]`) collapse to `_` and no longer match the intended tools"
        );
    }
    Some(format!("{}_{}", opencode_sanitize_tool_name(server), sanitized_leaf))
}

fn opencode_sanitize_tool_name(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn opencode_sanitize_tool_pattern(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '*' | '?') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn opencode_remote_mcp(url: String, headers: Vec<(String, String)>) -> Value {
    let mut entry = json!({
        "type": "remote",
        "url": url,
    });
    if !headers.is_empty() {
        let headers: Map<String, Value> = headers
            .into_iter()
            .map(|(key, value)| (key, Value::String(value)))
            .collect();
        entry["headers"] = Value::Object(headers);
    }

    entry
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AgentProvider;
    use crate::spawn::providers::build_command_cli as build_command;
    use crate::spawn::providers::fixtures::*;

    fn config_of(command: &SpawnCommand) -> Value {
        serde_json::from_str(command.env.get("OPENCODE_CONFIG_CONTENT").expect("generated config")).unwrap()
    }

    /// The effect opencode would reach for `action`: the LAST rule whose
    /// action glob matches, `ask` when none does.
    fn effect_for(config: &Value, action: &str) -> String {
        config["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .find(|rule| {
                globset::Glob::new(rule["action"].as_str().unwrap())
                    .unwrap()
                    .compile_matcher()
                    .is_match(action)
            })
            .map_or("ask".into(), |rule| rule["effect"].as_str().unwrap().into())
    }

    #[test]
    fn opencode_projects_the_two_resume_shapes_it_has() {
        let cases = [
            (Resume::Last, vec!["--continue".to_string()]),
            (
                Resume::Session("abc123".into()),
                vec!["--session".to_string(), "abc123".to_string()],
            ),
        ];

        for (resume, expected) in cases {
            let command = crate::spawn::providers::build_command(
                &resolved(AgentProvider::OpenCode),
                None,
                &[],
                vec![],
                None,
                Some(&resume),
                None,
            )
            .unwrap();

            let at = command
                .args
                .iter()
                .position(|arg| arg == &expected[0])
                .unwrap_or_else(|| panic!("{resume:?} projects {expected:?}, got {:?}", command.args));
            assert_eq!(command.args[at..at + expected.len()], expected[..], "{resume:?}");
        }
    }

    /// opencode registers no picker at all, so the refusal has to be
    /// loud — falling back to `--continue` would resume a session the
    /// captain never chose.
    #[test]
    fn opencode_refuses_a_picker_it_cannot_open() {
        let err = crate::spawn::providers::build_command(
            &resolved(AgentProvider::OpenCode),
            None,
            &[],
            vec![],
            None,
            Some(&Resume::Picker),
            None,
        )
        .expect_err("opencode has no picker");

        assert!(err.to_string().contains("--resume-last"), "{err}");
    }

    /// The shared background service keeps the env of whichever client
    /// started it, so without `--standalone` the profile's config, db
    /// and keys are ignored whenever one is already running — on the
    /// TUI and on `run` alike.
    #[test]
    fn opencode_always_runs_standalone() {
        for prompt in [None, Some("do the thing")] {
            let command = build_command(&resolved(AgentProvider::OpenCode), None, &[], vec![], prompt).unwrap();

            assert!(
                command.args.iter().any(|arg| arg == "--standalone"),
                "{prompt:?}: {:?}",
                command.args
            );
        }
    }

    #[test]
    fn opencode_an_explicit_server_suppresses_standalone() {
        for flag in ["--standalone", "--server=http://127.0.0.1:4096"] {
            let command =
                build_command(&resolved(AgentProvider::OpenCode), None, &[], vec![flag.into()], None).unwrap();

            assert_eq!(
                command.args.iter().filter(|arg| arg.starts_with("--s")).count(),
                1,
                "{flag}: {:?}",
                command.args
            );
        }
    }

    /// The TUI rejects `--model` and `--agent` (and `--dir`) outright, so
    /// emitting any of them leaves no session at all.
    #[test]
    fn opencode_interactive_carries_no_flag_the_tui_rejects() {
        let command = build_command(
            &resolved_with_cwd(AgentProvider::OpenCode, "/tmp/hyprpilot-work"),
            None,
            &[],
            Vec::new(),
            None,
        )
        .unwrap();

        assert_eq!(command.args, vec!["--standalone"]);
        assert_eq!(command.cwd(), Some(std::path::Path::new("/tmp/hyprpilot-work")));
    }

    /// `run` resolves its directory from `$PWD` before the process cwd,
    /// and `$PWD` is inherited from the caller's shell — so the agent ran
    /// in the wrong tree while every hyprpilot surface reported the
    /// requested path. `run` has no `--dir` to say it instead.
    #[test]
    fn opencode_headless_runs_in_the_resolved_cwd() {
        let command = build_command(
            &resolved_with_cwd(AgentProvider::OpenCode, "/tmp/hyprpilot-work"),
            None,
            &[],
            Vec::new(),
            Some("do the thing"),
        )
        .unwrap();

        assert_eq!(command.env.get("PWD").map(String::as_str), Some("/tmp/hyprpilot-work"));
        assert_eq!(command.cwd(), Some(std::path::Path::new("/tmp/hyprpilot-work")));
        assert!(!command.args.iter().any(|arg| arg == "--dir"), "{:?}", command.args);
    }

    #[test]
    fn opencode_without_a_resolved_cwd_leaves_pwd_alone() {
        let command = build_command(&resolved(AgentProvider::OpenCode), None, &[], Vec::new(), Some("go")).unwrap();

        assert!(!command.env.contains_key("PWD"), "{:?}", command.env);
    }

    #[test]
    fn opencode_resume_rides_alongside_standalone_on_both_paths() {
        for prompt in [None, Some("go on")] {
            for resume in [Resume::Last, Resume::Session("ses_abc".into())] {
                let command = crate::spawn::providers::build_command(
                    &resolved(AgentProvider::OpenCode),
                    None,
                    &[],
                    vec![],
                    prompt,
                    Some(&resume),
                    None,
                )
                .unwrap();
                let args = &command.args;

                assert!(
                    args.iter().any(|arg| arg == "--standalone"),
                    "{prompt:?} {resume:?}: {args:?}"
                );
                match &resume {
                    Resume::Last => assert!(args.iter().any(|arg| arg == "--continue"), "{args:?}"),
                    Resume::Session(id) => {
                        assert!(args.windows(2).any(|w| w == ["--session", id.as_str()]), "{args:?}")
                    }
                    Resume::Picker => unreachable!(),
                }
            }
        }
    }

    #[test]
    fn opencode_harness_asks_run_for_json_events() {
        let command = crate::spawn::providers::build_command(
            &resolved(AgentProvider::OpenCode),
            None,
            &[],
            vec![],
            Some("go"),
            None,
            Some(&HarnessProjection {
                structured_output: true,
            }),
        )
        .unwrap();

        assert_eq!(command.args.first().map(String::as_str), Some("run"));
        assert!(
            command.args.windows(2).any(|w| w == ["--format", "json"]),
            "{:?}",
            command.args
        );
    }

    /// opencode silently drops an agent whose model ref it cannot parse —
    /// system prompt included — so a bare model id must fail the launch.
    #[test]
    fn opencode_refuses_a_model_without_a_provider() {
        for model in [
            "model-a",
            "/model-a",
            "provider/",
            "provider/model#",
            "provider/model#a#b",
        ] {
            let mut resolved = resolved(AgentProvider::OpenCode);
            resolved.model = Some(model.into());
            resolved.effort = None;
            let err = build_command(&resolved, Some("be terse"), &[], vec![], None).expect_err(model);

            assert!(err.to_string().contains("provider/model"), "{model}: {err}");
        }
    }

    #[test]
    fn opencode_puts_prompt_model_and_variant_on_the_default_agent() {
        let command = build_command(
            &resolved(AgentProvider::OpenCode),
            Some("be terse"),
            &[],
            Vec::new(),
            None,
        )
        .unwrap();
        let config = config_of(&command);

        assert_eq!(config["default_agent"], "plan");
        assert_eq!(config["agents"]["plan"]["system"], "be terse");
        assert_eq!(config["agents"]["plan"]["model"], "provider/model-a#high");
        assert_eq!(config["model"], "provider/model-a#high");
    }

    #[test]
    fn opencode_effort_without_a_model_is_dropped() {
        let mut resolved = resolved_with_mode(AgentProvider::OpenCode, None);
        resolved.model = None;
        let command = build_command(&resolved, None, &[], Vec::new(), None).unwrap();

        assert!(
            !command.env.contains_key("OPENCODE_CONFIG_CONTENT"),
            "{:?}",
            command.env
        );
    }

    #[test]
    fn opencode_a_model_naming_its_variant_is_kept_verbatim() {
        let mut resolved = resolved(AgentProvider::OpenCode);
        resolved.model = Some("provider/model#low".into());
        let config = config_of(&build_command(&resolved, None, &[], Vec::new(), None).unwrap());

        assert_eq!(config["model"], "provider/model#low");
    }

    #[test]
    fn opencode_agent_name_falls_back_to_hyprpilot() {
        let command = build_command(
            &resolved_with_mode(AgentProvider::OpenCode, None),
            Some("be terse"),
            &[],
            vec![],
            None,
        )
        .unwrap();
        let config = config_of(&command);

        assert_eq!(config["default_agent"], "hyprpilot");
        assert_eq!(config["agents"]["hyprpilot"]["system"], "be terse");
    }

    /// A mode alone names an existing opencode agent; there is nothing
    /// to define, only the default to pick.
    #[test]
    fn opencode_mode_alone_only_selects_the_agent() {
        let mut resolved = resolved(AgentProvider::OpenCode);
        resolved.model = None;
        resolved.effort = None;
        let config = config_of(&build_command(&resolved, None, &[], vec![], None).unwrap());

        assert_eq!(config, json!({ "default_agent": "plan" }));
    }

    #[test]
    fn opencode_an_agent_flag_names_the_generated_agent() {
        let command = build_command(
            &resolved(AgentProvider::OpenCode),
            Some("be terse"),
            &[],
            vec!["--agent".into(), "review".into()],
            Some("look"),
        )
        .unwrap();
        let config = config_of(&command);

        assert_eq!(config["default_agent"], "review");
        assert_eq!(config["agents"]["review"]["system"], "be terse");
    }

    #[test]
    fn opencode_projects_mcp_servers_in_the_v2_shape() {
        let command = build_command(
            &resolved(AgentProvider::OpenCode),
            None,
            &[mcp_def(), remote_mcp_def_with_headers()],
            Vec::new(),
            None,
        )
        .unwrap();
        let servers = &config_of(&command)["mcp"]["servers"];

        assert_eq!(
            servers["hyprpilot-nvim"],
            json!({
                "type": "local",
                "command": ["uvx", "hyprpilot-nvim-mcp"],
                "environment": { "NVIM": "/tmp/nvim.sock" },
            })
        );
        assert_eq!(servers["github"]["type"], "remote");
        assert_eq!(servers["github"]["url"], "https://example.test/mcp");
        assert_eq!(servers["github"]["headers"]["X-MCP-Insiders"], "true");
        assert!(servers["github"].get("enabled").is_none());
    }

    #[test]
    fn opencode_projects_mcp_tool_policy_into_permission_rules() {
        let command = build_command(
            &resolved(AgentProvider::OpenCode),
            None,
            &[mcp_def_with_visibility()],
            Vec::new(),
            None,
        )
        .unwrap();
        let config = config_of(&command);

        assert!(config["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|rule| rule["resource"] == "*"));
        assert_eq!(effect_for(&config, "filesystem_read_file"), "allow");
        assert_eq!(effect_for(&config, "filesystem_list_directory"), "allow");
        assert_eq!(effect_for(&config, "filesystem_delete_file"), "deny");
        assert_eq!(effect_for(&config, "filesystem_write_file"), "deny");
        assert_eq!(effect_for(&config, "filesystem_move_file"), "deny", "not included");
    }

    /// The ruleset is last-match-wins, so hyprpilot's precedence —
    /// exclude beats include, reject beats accept — is the rule ORDER.
    #[test]
    fn opencode_permission_order_preserves_hyprpilot_policy_precedence() {
        let command = build_command(
            &resolved(AgentProvider::OpenCode),
            None,
            &[mcp_def_with_visibility_conflicts()],
            Vec::new(),
            None,
        )
        .unwrap();
        let config = config_of(&command);

        assert_eq!(
            effect_for(&config, "filesystem_read_file"),
            "deny",
            "reject beats accept"
        );
        assert_eq!(
            effect_for(&config, "filesystem_delete_file"),
            "deny",
            "exclude beats accept"
        );
        assert_eq!(effect_for(&config, "filesystem_write_file"), "deny", "not included");
    }

    #[test]
    fn opencode_auto_accept_allows_without_an_include_list() {
        let config = config_of(
            &build_command(
                &resolved(AgentProvider::OpenCode),
                None,
                &[mcp_def_with_permissions()],
                vec![],
                None,
            )
            .unwrap(),
        );

        assert_eq!(effect_for(&config, "filesystem_read_file"), "allow");
        assert_eq!(effect_for(&config, "filesystem_delete_file"), "deny");
        assert_eq!(effect_for(&config, "filesystem_write_file"), "deny", "prefixed pattern");
        assert_eq!(effect_for(&config, "filesystem_move_file"), "ask");
    }

    #[test]
    fn opencode_preset_config_content_is_not_overwritten() {
        let mut resolved = resolved(AgentProvider::OpenCode);
        resolved
            .agent
            .env
            .insert("OPENCODE_CONFIG_CONTENT".into(), "{\"preset\":true}".into());
        let command = build_command(&resolved, Some("be terse"), &[mcp_def()], vec![], None).unwrap();

        assert_eq!(
            command.env.get("OPENCODE_CONFIG_CONTENT").map(String::as_str),
            Some("{\"preset\":true}"),
            "an agent-provided OPENCODE_CONFIG_CONTENT must not be clobbered by generated config"
        );
    }

    #[test]
    fn opencode_projects_sse_as_remote() {
        let map = Value::Object(opencode_mcp_servers(&[sse_mcp_def()]));

        assert_eq!(map["events"]["type"], "remote");
        assert_eq!(map["events"]["url"], "https://example.test/sse");
    }

    #[test]
    fn opencode_sanitizes_charclass_glob_in_permission_action() {
        let command = build_command(
            &resolved_with_mode(AgentProvider::OpenCode, None),
            None,
            &[mcp_def_with_charclass_glob()],
            vec![],
            None,
        )
        .unwrap();

        // `[` and `]` collapse to `_`: read_[abc] → read__abc_ (the
        // pattern no longer matches the intended tools — hence the warn).
        assert_eq!(
            config_of(&command)["permissions"][0],
            json!({ "action": "filesystem_read__abc_", "resource": "*", "effect": "allow" })
        );
    }

    #[test]
    fn opencode_headless_projects_run_subcommand_and_prompt_positional() {
        let command = build_command(
            &resolved_with_mode(AgentProvider::OpenCode, None),
            None,
            &[],
            vec![],
            Some("write the readme"),
        )
        .unwrap();

        assert_eq!(
            command.args.first().map(String::as_str),
            Some("run"),
            "run is the subcommand"
        );
        assert!(command.args.iter().any(|a| a == "--standalone"));
        assert_eq!(command.args.last().map(String::as_str), Some("write the readme"));
    }

    #[test]
    fn opencode_headless_passes_the_agent_and_model_before_the_prompt() {
        let command = build_command(
            &resolved(AgentProvider::OpenCode),
            None,
            &[],
            vec![],
            Some("write the readme"),
        )
        .unwrap();
        let args = &command.args;

        let agent = args.iter().position(|a| a == "--agent").expect("--agent");
        let model = args.iter().position(|a| a == "--model").expect("--model");
        assert_eq!(args[agent + 1], "plan");
        assert_eq!(args[model + 1], "provider/model-a#high");
        assert!(agent < args.len() - 1 && model < args.len() - 1, "{args:?}");
        assert_eq!(args.last().map(String::as_str), Some("write the readme"));
    }

    #[test]
    fn opencode_headless_forwarded_agent_and_model_suppress_the_generated_ones() {
        for forwarded in [
            vec![
                "--agent".to_string(),
                "review".to_string(),
                "-m".to_string(),
                "p/m".to_string(),
            ],
            vec!["--agent=review".to_string(), "--model=p/m".to_string()],
        ] {
            let command = build_command(
                &resolved(AgentProvider::OpenCode),
                None,
                &[],
                forwarded.clone(),
                Some("go"),
            )
            .unwrap();

            let expected: Vec<String> = ["run", "--standalone"]
                .into_iter()
                .map(String::from)
                .chain(forwarded.clone())
                .chain(["go".to_string()])
                .collect();
            assert_eq!(command.args, expected, "{forwarded:?}");
        }
    }

    #[test]
    fn opencode_interactive_launch_carries_neither_agent_nor_model_flag() {
        let command = build_command(&resolved(AgentProvider::OpenCode), None, &[], vec![], None).unwrap();

        assert!(
            !command.args.iter().any(|a| a == "--model" || a == "--agent"),
            "{:?}",
            command.args
        );
    }
}

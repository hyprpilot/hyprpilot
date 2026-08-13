//! Auto-injection of the in-tree MCP servers.
//!
//! One `build_*_definition` per server. Under the `[mcp].enabled`
//! master gate the launcher prepends a stdio entry for each server its
//! own block enables, and the vendor spawns those sidecars itself.
//! Skills is the only one ALSO gated on content — an empty
//! `SkillsRegistry` means nothing to serve — and the only one that
//! passes state on the command line (`--skill-dir <json>` per root).
//!
//! References declared in each skill's frontmatter resolve relative
//! to the skill's own bundle directory at read time — the sidecar
//! maintains no separate references-root concept.
//!
//! Each server's resolved name is what vendors prefix tool calls with
//! (`mcp__hyprpilot_skills__list_skills`, …) and is RESERVED: a
//! same-named configured server is replaced. Auto-accept rides through
//! `HyprpilotExtension.auto_accept_tools` — the server's own globs when
//! set, else the `[mcp]`-level ones (default `["*"]`), so by default
//! every tool is projected as auto-approved unless the captain has
//! tightened them. **That default applies to the harness too**: turning
//! `[mcp.harness].enabled` on without per-server globs auto-approves
//! `spawn`.

use std::path::PathBuf;
use std::sync::Arc;

use crate::config::McpConfig;
use crate::mcp::skills::SkillsRegistry;
use crate::mcp::{HyprpilotExtension, MCPDefinition};

/// Build the general-tools catalog entry.
///
/// Like the harness and unlike skills, this is not gated on having
/// content to serve — the tool list is fixed, so the captain's
/// `enabled` flag is the whole gate.
#[must_use]
pub fn build_tools_definition(cfg: &McpConfig, source: PathBuf) -> Option<MCPDefinition> {
    let tools = cfg.serve.clone().unwrap_or_default();
    if !tools.is_enabled() {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let raw = serde_json::json!({
        "command": exe.display().to_string(),
        "args": ["mcp", "serve"],
        "env": serde_json::Map::<String, serde_json::Value>::new(),
    });

    Some(MCPDefinition {
        name: tools.server_name().to_string(),
        raw,
        hyprpilot: HyprpilotExtension {
            include_tools: None,
            exclude_tools: Vec::new(),
            auto_accept_tools: tools
                .auto_accept_tools
                .clone()
                .unwrap_or_else(|| cfg.auto_accept_tools().to_vec()),
            auto_reject_tools: tools
                .auto_reject_tools
                .clone()
                .unwrap_or_else(|| cfg.auto_reject_tools().to_vec()),
        },
        source,
    })
}

/// Build the harness catalog entry for a session that will run at
/// `spawn_depth`.
///
/// Separate from the skills entry so the two servers get independent
/// process lifetimes and independent tool policy — auto-accepting a
/// skill read and auto-accepting `spawn` are not the same decision.
///
/// Unlike skills, this is NOT gated on having content to serve: the
/// harness always has tools. Two gates apply instead.
///
/// **Depth first, and it is absolute.** A session at or past
/// `[mcp.harness] maxDepth` gets no entry even with `enabled = true` —
/// its harness could only ever refuse `spawn`, so injecting one buys a
/// long-lived process and seven tools that exist to error. Reading the
/// cap from the same block being injected is what makes it
/// self-adjusting: raise `maxDepth` and delegates get a harness again,
/// correctly, with nothing else to change.
///
/// Then the captain's `enabled`, which is deliberately off by default —
/// see [`HarnessServerConfig`].
#[must_use]
pub fn build_harness_definition(cfg: &McpConfig, source: PathBuf, spawn_depth: usize) -> Option<MCPDefinition> {
    let harness = cfg.harness.clone().unwrap_or_default();
    if spawn_depth >= harness.max_depth() {
        tracing::debug!(
            spawn_depth,
            max_depth = harness.max_depth(),
            "auto_inject: harness suppressed — session is at the delegation cap"
        );
        return None;
    }
    if !harness.is_enabled() {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let mut args = vec!["mcp".to_string(), "harness".to_string()];
    args.push("--max-sessions".to_string());
    args.push(harness.max_sessions().to_string());
    // The sidecar enforces the same cap on `spawn` that this function
    // enforced on injection. It cannot re-read the value: `[mcp.harness]`
    // is per-profile and only the launcher knows which profile was
    // picked.
    args.push("--max-depth".to_string());
    args.push(harness.max_depth().to_string());
    // Same shape as `--max-sessions`: the sidecar is spawned by the
    // launcher, which already resolved the PICKED profile's `[mcp]`
    // block. Re-reading config inside the sidecar cannot recover which
    // profile that was.
    if !harness.notifies_on_complete() {
        args.push("--no-notify-on-complete".to_string());
    }
    // Same reason again: which profiles this launch may delegate to is
    // the PICKED profile's `[mcp.harness]` scope, and the sidecar has no
    // way to work out which profile that was.
    //
    // An empty include list rides its own flag rather than zero
    // `--include-profile` occurrences, which is indistinguishable from
    // "unset" on the wire — and would silently mean *unrestricted*, the
    // exact opposite of what the captain wrote.
    //
    // `--flag=<glob>` rather than two argv items: `Glob::new("-foo")` is
    // valid and passes config validation, but as a separate item clap
    // reads it as flags and the sidecar dies on an error naming neither
    // the pattern nor the field it came from.
    match harness.include_profiles.as_deref() {
        Some([]) => args.push("--no-delegates".to_string()),
        Some(globs) => {
            for glob in globs {
                args.push(format!("--include-profile={glob}"));
            }
        }
        None => {}
    }
    for glob in harness.exclude_profiles.as_deref().unwrap_or_default() {
        args.push(format!("--exclude-profile={glob}"));
    }
    // Same reason a third time: the delegate overlay is the PICKED
    // profile's `[mcp.harness.mcp]`, and the sidecar cannot work out
    // which profile that was. Emitted only when the captain declared
    // one — absent means every delegate keeps its own resolved `[mcp]`,
    // which is also what a hand-started sidecar gets.
    if let Some(delegate_mcp) = harness.delegate_mcp() {
        match serde_json::to_string(delegate_mcp) {
            Ok(json) => args.push(format!("--delegate-mcp={json}")),
            // Unreachable in practice — the block is a plain typed
            // struct — but silently dropping it would widen what
            // delegates reach, so say so loudly rather than warn-and-go.
            Err(err) => {
                tracing::error!(%err, "auto_inject: delegate mcp overlay failed to serialize — not injecting the harness");
                return None;
            }
        }
    }
    let raw = serde_json::json!({
        "command": exe.display().to_string(),
        "args": args,
        "env": serde_json::Map::<String, serde_json::Value>::new(),
    });

    Some(MCPDefinition {
        name: harness.server_name().to_string(),
        raw,
        hyprpilot: HyprpilotExtension {
            include_tools: None,
            exclude_tools: Vec::new(),
            // Per-server policy wins; otherwise the `[mcp]` default.
            auto_accept_tools: harness
                .auto_accept_tools
                .clone()
                .unwrap_or_else(|| cfg.auto_accept_tools().to_vec()),
            auto_reject_tools: harness
                .auto_reject_tools
                .clone()
                .unwrap_or_else(|| cfg.auto_reject_tools().to_vec()),
        },
        source,
    })
}

/// Build the catalog entry the launcher prepends to the vendor's MCP
/// config.
///
/// Returns `None` when the registry is empty — auto-inject is gated
/// on having something to serve. The caller is responsible for the
/// `enabled` gate (see `effective_mcp_with` in
/// `resolve/mod.rs`); this builder assumes the captain
/// wants the server when called.
///
/// `cfg.auto_accept_tools` / `cfg.auto_reject_tools` ride through to
/// the projected entry's `HyprpilotExtension` namespace so the
/// per-server tool policy handles them uniformly with user-declared
/// `mcps`.
///
/// `raw` is constructed in the **user-input** JSON shape (matching
/// what `mcpServers[name]` carries on disk) — `project_transport`
/// re-projects from the user shape so it expects `command: <string>`
/// and `env: { K: V }`.
#[must_use]
pub fn build_skills_definition(
    skills: &Arc<SkillsRegistry>,
    cfg: &McpConfig,
    source: PathBuf,
) -> Option<MCPDefinition> {
    // Gate on dirs having at least one loaded skill — if the
    // directories are empty or all skills match the ignore globs,
    // there's nothing to serve.
    let skills_cfg = cfg.skills.clone().unwrap_or_default();
    if !skills_cfg.is_enabled() || skills.list().is_empty() {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let mut args: Vec<String> = vec!["mcp".to_string(), "skills".to_string()];
    // Pass directories + de-duplicated ignore globs instead of
    // enumerating individual `--skill slug=path` entries. The sidecar
    // scans dirs with the same `SkillsRegistry` discovery code the
    // launcher uses — adding a new `<slug>/SKILL.md` to a configured
    // directory is immediately visible on the next `reload` without
    // restarting the session.
    // Each directory is serialized as a JSON object so per-dir ignore
    // lists survive the CLI round-trip without flattening — the sidecar
    // can reconstruct the exact same `ResolvedSkillEntry` set the
    // launcher computed, with each root's suppression applied only to
    // that root's discoveries.
    for entry in skills.dirs() {
        let json = serde_json::json!({
            "dir": entry.dir.display().to_string(),
            "ignore": entry.ignore_patterns,
        });
        args.push("--skill-dir".to_string());
        args.push(json.to_string());
    }
    let raw = serde_json::json!({
        "command": exe.display().to_string(),
        "args": args,
        "env": serde_json::Map::<String, serde_json::Value>::new(),
    });
    Some(MCPDefinition {
        name: skills_cfg.server_name().to_string(),
        raw,
        hyprpilot: HyprpilotExtension {
            include_tools: None,
            exclude_tools: Vec::new(),
            auto_accept_tools: skills_cfg
                .auto_accept_tools
                .clone()
                .unwrap_or_else(|| cfg.auto_accept_tools().to_vec()),
            auto_reject_tools: skills_cfg
                .auto_reject_tools
                .clone()
                .unwrap_or_else(|| cfg.auto_reject_tools().to_vec()),
        },
        source,
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use crate::config::mcp::{HarnessServerConfig, ToolsServerConfig};
    use crate::mcp::skills::SkillsRegistry;

    use super::*;

    fn empty_registry() -> Arc<SkillsRegistry> {
        Arc::new(SkillsRegistry::new(Vec::new()))
    }

    fn default_cfg() -> McpConfig {
        McpConfig::default()
    }

    #[test]
    fn empty_registry_skips_injection() {
        assert!(build_skills_definition(&empty_registry(), &default_cfg(), PathBuf::from("<test>")).is_none());
    }

    /// The security-relevant default. `spawn` runs a profile's
    /// `command`, so a captain who never mentions the harness must not
    /// get an entry for it.
    #[test]
    fn harness_is_not_injected_by_default() {
        assert!(build_harness_definition(&default_cfg(), PathBuf::from("<test>"), 0).is_none());
    }

    #[test]
    fn harness_is_injected_once_enabled() {
        let cfg = McpConfig {
            harness: Some(HarnessServerConfig {
                enabled: Some(true),
                ..Default::default()
            }),
            ..McpConfig::default()
        };
        let def = build_harness_definition(&cfg, PathBuf::from("<test>"), 0).expect("enabled harness injects");
        assert_eq!(def.name, crate::config::mcp::DEFAULT_HARNESS_SERVER_NAME);
    }

    /// Enabling the harness without per-server globs inherits the
    /// `[mcp]`-level `["*"]`, which auto-approves `spawn`. Pinned so
    /// the consequence is a deliberate choice rather than a surprise —
    /// flip this test if the default ever tightens.
    #[test]
    fn enabling_harness_inherits_the_permissive_accept_default() {
        let cfg = McpConfig {
            harness: Some(HarnessServerConfig {
                enabled: Some(true),
                ..Default::default()
            }),
            ..McpConfig::default()
        };
        let def = build_harness_definition(&cfg, PathBuf::from("<test>"), 0).expect("injects");
        assert_eq!(def.hyprpilot.auto_accept_tools, vec!["*".to_string()]);
    }

    /// Default ON — and the flag only appears when the captain turns it
    /// OFF, so the sidecar's own default and the config agree.
    #[test]
    fn completion_notification_is_on_by_default() {
        let cfg = McpConfig {
            harness: Some(HarnessServerConfig {
                enabled: Some(true),
                ..Default::default()
            }),
            ..McpConfig::default()
        };
        let def = build_harness_definition(&cfg, PathBuf::from("<test>"), 0).expect("injects");
        let args = def.raw["args"].as_array().expect("args");
        assert!(
            !args.iter().any(|a| a == "--no-notify-on-complete"),
            "the opt-out flag must be absent by default, got: {args:?}"
        );
    }

    /// The knob has to ride the ARGV. A sidecar cannot re-read config to
    /// find it — `[mcp.harness]` is per-profile, and only the launcher
    /// knows which profile was picked.
    #[test]
    fn disabling_completion_notification_passes_the_flag() {
        let cfg = McpConfig {
            harness: Some(HarnessServerConfig {
                enabled: Some(true),
                notify_on_complete: Some(false),
                ..Default::default()
            }),
            ..McpConfig::default()
        };
        let def = build_harness_definition(&cfg, PathBuf::from("<test>"), 0).expect("injects");
        let args = def.raw["args"].as_array().expect("args");
        assert!(args.iter().any(|a| a == "--no-notify-on-complete"), "got: {args:?}");
    }

    fn harness_args(cfg: &McpConfig) -> Vec<String> {
        let def = build_harness_definition(cfg, PathBuf::from("<test>"), 0).expect("injects");
        def.raw["args"]
            .as_array()
            .expect("args")
            .iter()
            .map(|a| a.as_str().expect("string arg").to_string())
            .collect()
    }

    fn scoped(include: Option<Vec<&str>>, exclude: Option<Vec<&str>>) -> McpConfig {
        let owned = |v: Vec<&str>| v.into_iter().map(String::from).collect::<Vec<_>>();
        McpConfig {
            harness: Some(HarnessServerConfig {
                enabled: Some(true),
                include_profiles: include.map(owned),
                exclude_profiles: exclude.map(owned),
                ..Default::default()
            }),
            ..McpConfig::default()
        }
    }

    /// One occurrence per pattern, like `--skill-dir`.
    #[test]
    fn the_delegate_scope_rides_argv_one_flag_per_pattern() {
        let args = harness_args(&scoped(
            Some(vec!["personal/*", "scratch/*"]),
            Some(vec!["personal/codex/*"]),
        ));

        assert_eq!(
            args.iter().filter(|a| a.starts_with("--include-profile")).count(),
            2,
            "got: {args:?}"
        );
        assert!(
            args.iter().any(|a| a == "--include-profile=personal/*"),
            "got: {args:?}"
        );
        assert!(args.iter().any(|a| a == "--include-profile=scratch/*"), "got: {args:?}");
        assert!(
            args.iter().any(|a| a == "--exclude-profile=personal/codex/*"),
            "got: {args:?}"
        );
        assert!(!args.iter().any(|a| a == "--no-delegates"), "got: {args:?}");
    }

    /// `Glob::new("-foo")` is valid, so config validation accepts it and
    /// it reaches argv. As a separate item clap reads it as flags and the
    /// sidecar dies with an error naming neither the pattern nor the
    /// field it came from.
    #[test]
    fn a_pattern_starting_with_a_dash_still_round_trips() {
        let args = harness_args(&scoped(Some(vec!["-dash/*"]), Some(vec!["-other"])));

        assert!(args.iter().any(|a| a == "--include-profile=-dash/*"), "got: {args:?}");
        assert!(args.iter().any(|a| a == "--exclude-profile=-other"), "got: {args:?}");
        assert!(
            !args.iter().any(|a| a == "-dash/*" || a == "-other"),
            "a pattern must never be its own argv item: {args:?}"
        );
    }

    /// The wire cannot tell zero `--include-profile` occurrences from an
    /// absent list, and absent means UNRESTRICTED — so an empty
    /// `includeProfiles` must ride its own flag rather than decaying
    /// into the opposite of what the captain wrote.
    #[test]
    fn an_empty_include_list_becomes_no_delegates_not_silence() {
        let args = harness_args(&scoped(Some(vec![]), None));

        assert!(args.iter().any(|a| a == "--no-delegates"), "got: {args:?}");
        assert!(!args.iter().any(|a| a == "--include-profile"), "got: {args:?}");
    }

    /// An empty scope still injects the server — it simply has no
    /// candidates. Keeps one shape for "the harness is on", with the
    /// candidate list a separate question.
    #[test]
    fn an_empty_scope_still_injects_the_server() {
        assert!(build_harness_definition(&scoped(Some(vec![]), None), PathBuf::from("<test>"), 0).is_some());
    }

    #[test]
    fn an_unscoped_harness_passes_no_delegate_flags() {
        let cfg = McpConfig {
            harness: Some(HarnessServerConfig {
                enabled: Some(true),
                ..Default::default()
            }),
            ..McpConfig::default()
        };
        let args = harness_args(&cfg);

        assert!(
            !args.iter().any(|a| a.starts_with("--include-profile")
                || a.starts_with("--exclude-profile")
                || a == "--no-delegates"),
            "an unset scope must stay unrestricted: {args:?}"
        );
    }

    #[test]
    fn per_server_globs_override_rather_than_merge() {
        let cfg = McpConfig {
            harness: Some(HarnessServerConfig {
                enabled: Some(true),
                auto_accept_tools: Some(vec!["list_profiles".into()]),
                ..Default::default()
            }),
            ..McpConfig::default()
        };
        let def = build_harness_definition(&cfg, PathBuf::from("<test>"), 0).expect("injects");
        assert_eq!(
            def.hyprpilot.auto_accept_tools,
            vec!["list_profiles".to_string()],
            "the `[mcp]`-level `*` must not survive alongside a per-server list"
        );
    }

    /// The whole point of the gate. A delegate's harness could only
    /// refuse `spawn`, so it must not be injected however loudly the
    /// config asks — including through the delegate overlay, which is
    /// the one surface that could plausibly try.
    #[test]
    fn the_depth_gate_beats_an_explicit_enable() {
        let cfg = McpConfig {
            harness: Some(HarnessServerConfig {
                enabled: Some(true),
                ..Default::default()
            }),
            ..McpConfig::default()
        };

        assert!(build_harness_definition(&cfg, PathBuf::from("<test>"), 0).is_some());
        assert!(
            build_harness_definition(&cfg, PathBuf::from("<test>"), 1).is_none(),
            "a session at the default maxDepth of 1 must get no harness"
        );
    }

    /// Reading the cap from the block being injected is what makes the
    /// gate self-adjusting: raising `maxDepth` gives delegates a harness
    /// again with nothing else to change.
    #[test]
    fn a_raised_max_depth_reopens_injection_one_level_down() {
        let cfg = McpConfig {
            harness: Some(HarnessServerConfig {
                enabled: Some(true),
                max_depth: Some(2),
                ..Default::default()
            }),
            ..McpConfig::default()
        };

        assert!(build_harness_definition(&cfg, PathBuf::from("<test>"), 1).is_some());
        assert!(build_harness_definition(&cfg, PathBuf::from("<test>"), 2).is_none());
    }

    /// The sidecar enforces the same cap on `spawn` that this function
    /// enforced on injection, so the number has to reach it — a sidecar
    /// cannot recover which profile spawned it.
    #[test]
    fn the_resolved_ceilings_ride_argv() {
        let cfg = McpConfig {
            harness: Some(HarnessServerConfig {
                enabled: Some(true),
                max_depth: Some(3),
                max_sessions: Some(9),
                ..Default::default()
            }),
            ..McpConfig::default()
        };
        let args = harness_args(&cfg);

        let pair = |flag: &str| {
            args.iter()
                .position(|a| a == flag)
                .and_then(|i| args.get(i + 1))
                .cloned()
        };
        assert_eq!(pair("--max-depth").as_deref(), Some("3"), "got: {args:?}");
        assert_eq!(pair("--max-sessions").as_deref(), Some("9"), "got: {args:?}");
    }

    /// Absent means "every delegate keeps its own resolved `[mcp]`",
    /// which is also what a hand-started sidecar gets — so the flag must
    /// not appear merely because the harness is on.
    #[test]
    fn no_delegate_overlay_flag_without_a_declared_block() {
        let cfg = McpConfig {
            harness: Some(HarnessServerConfig {
                enabled: Some(true),
                ..Default::default()
            }),
            ..McpConfig::default()
        };

        assert!(
            !harness_args(&cfg).iter().any(|a| a.starts_with("--delegate-mcp")),
            "an undeclared overlay must not reach argv"
        );
    }

    /// The overlay round-trips as JSON on argv and comes back as the
    /// same block — the sidecar re-parses it into `McpConfig`.
    #[test]
    fn a_declared_delegate_overlay_round_trips_through_argv() {
        let overlay = McpConfig {
            serve: Some(ToolsServerConfig {
                enabled: Some(false),
                ..Default::default()
            }),
            ..Default::default()
        };
        let cfg = McpConfig {
            harness: Some(HarnessServerConfig {
                enabled: Some(true),
                mcp: Some(Box::new(overlay.clone())),
                ..Default::default()
            }),
            ..McpConfig::default()
        };

        let raw = harness_args(&cfg)
            .into_iter()
            .find_map(|a| a.strip_prefix("--delegate-mcp=").map(str::to_string))
            .expect("a declared overlay rides argv");
        let parsed: McpConfig = serde_json::from_str(&raw).expect("the sidecar can parse what we emit");

        assert_eq!(parsed, overlay);
    }

    #[test]
    fn tools_server_is_injected_by_default() {
        let def = build_tools_definition(&default_cfg(), PathBuf::from("<test>")).expect("serve defaults on");
        assert_eq!(def.name, crate::config::mcp::DEFAULT_TOOLS_SERVER_NAME);
    }

    #[test]
    fn disabling_a_server_skips_only_that_one() {
        let cfg = McpConfig {
            serve: Some(ToolsServerConfig {
                enabled: Some(false),
                ..Default::default()
            }),
            harness: Some(HarnessServerConfig {
                enabled: Some(true),
                ..Default::default()
            }),
            ..McpConfig::default()
        };
        assert!(build_tools_definition(&cfg, PathBuf::from("<test>")).is_none());
        assert!(build_harness_definition(&cfg, PathBuf::from("<test>"), 0).is_some());
    }

    #[test]
    fn a_renamed_server_reserves_its_new_name() {
        let cfg = McpConfig {
            serve: Some(ToolsServerConfig {
                name: Some("mytools".into()),
                ..Default::default()
            }),
            ..McpConfig::default()
        };
        let def = build_tools_definition(&cfg, PathBuf::from("<test>")).expect("injects");
        assert_eq!(def.name, "mytools");
    }
}

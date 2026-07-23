mod launch;
mod multiplexer;
mod picker;
mod providers;

use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use serde_json::Value;

pub use launch::{run, LaunchArgs};

use crate::config::Config;
use crate::resolve::{
    build_mcp_registry_with, build_skills_registry_with, count_matching_patches, resolve_effective_profile,
    resolve_into_instance_and_profile, ProfileSummary,
};

#[derive(Debug)]
pub(crate) struct SpawnRequest {
    pub profile_id: Option<String>,
    /// Explicit headless prompt from `--prompt` / `--file`. When set it
    /// forces headless and is the prompt source (wins over piped stdin);
    /// `None` falls back to piped stdin.
    pub prompt: Option<String>,
    pub cwd: Option<PathBuf>,
    pub mode: Option<String>,
    pub config_patches: Vec<Value>,
    pub provider_args: Vec<String>,
    /// `--with-config -` already drained stdin building the patches.
    /// A headless launch then has no stdin left for its prompt.
    pub stdin_consumed: bool,
}

pub(crate) fn launch_profile(cfg: Config, request: SpawnRequest) -> Result<ExitCode> {
    let SpawnRequest {
        profile_id,
        prompt,
        cwd,
        mode,
        config_patches,
        provider_args,
        stdin_consumed,
    } = request;

    let stdin_is_tty = std::io::stdin().is_terminal();
    // `--prompt` / `--file` force headless, so a bare launch resolves the
    // default profile directly rather than opening the picker.
    let prompt_given = prompt.is_some();

    let profile_id = match profile_id {
        Some(id) => id,
        None => select_profile_without_positional(&cfg, stdin_is_tty, prompt_given, cwd.as_deref(), &config_patches)?,
    };
    let (mut resolved, profile) = resolve_into_instance_and_profile(&cfg, Some(profile_id.as_str()), &config_patches)?;

    resolved.agent.cwd = resolve_launch_cwd(cwd, resolved.agent.cwd.take());
    if mode.is_some() {
        resolved.mode = mode;
    }

    let root_patch_count = count_matching_patches(&profile_id, cfg.patches.as_deref().unwrap_or_default(), "root");
    let external_patch_count = count_matching_patches(&profile_id, &config_patches, "external");
    tracing::info!(
        profile = %profile_id,
        agent = %resolved.agent.id,
        provider = resolved.agent.provider.wire_id(),
        model = ?resolved.model,
        mode = ?resolved.mode,
        effort = ?resolved.effort,
        cwd = ?resolved.agent.cwd,
        root_patches = root_patch_count,
        external_patches = external_patch_count,
        "cli: profile resolved"
    );

    let system_prompt = resolved.fresh_system_prompt();

    // Headless prompt source. An explicit `--prompt` / `--file` value
    // wins; otherwise the auto/generated path buffers stdin — when the
    // captain supplies the vendor's own invocation via trailing `-- …`,
    // `provider_args` is non-empty and fd0 stays inherited so the vendor
    // gets the raw pipe (the existing dedup suppresses hyprpilot's
    // projection).
    let prompt = headless_prompt(
        prompt,
        resolved.headless,
        stdin_is_tty,
        !provider_args.is_empty(),
        stdin_consumed,
        read_stdin_prompt,
    )?;
    if let Some(prompt) = prompt.as_deref() {
        tracing::info!(bytes = prompt.len(), "cli: headless launch — prompt resolved");
    }

    let skills = build_skills_registry_with(&profile);
    let mcp_defs = build_mcp_registry_with(&profile, Some(&skills));

    let command = providers::build_command(
        &resolved,
        system_prompt.as_deref(),
        &mcp_defs,
        provider_args,
        prompt.as_deref(),
    )?;

    if cfg
        .multiplexer
        .set_title
        .expect("[multiplexer] set_title seeded by defaults.toml")
    {
        // Skip the rename when `HYPRPILOT_NO_TITLE` is set (the explicit
        // launcher override) or an editor (nvim, emacs, VS Code) that
        // spawned hyprpilot owns the multiplexer pane — renaming its
        // window/tab from underneath it is wrong, regardless of
        // `set_title`.
        if let Some(reason) = multiplexer::title_rename_skip_reason() {
            tracing::debug!(reason, "cli spawn: skipping multiplexer title rename");
        } else if let Some(multiplexer) = multiplexer::Multiplexer::detect() {
            let launch_cwd = command
                .cwd()
                .map(Path::to_path_buf)
                .or_else(|| std::env::current_dir().ok());
            if let Some(launch_cwd) = launch_cwd {
                multiplexer.set_title(&multiplexer::title_for(&launch_cwd));
            }
        }
    }

    providers::exec(command)
}

/// cwd precedence for the launch: explicit `--cwd` flag wins, then
/// the profile/agent `cwd` (already projected onto
/// `resolved.agent.cwd` by `from_profile_explicit`), and only when
/// neither is set does `current_dir()` fill in. Kept a free fn so the
/// precedence — the K-740 bug's fix — is unit-testable without an
/// `exec()` at the end of `run`.
fn resolve_launch_cwd(flag: Option<PathBuf>, configured: Option<PathBuf>) -> Option<PathBuf> {
    flag.or(configured).or_else(|| std::env::current_dir().ok())
}

/// Pick the profile id when no positional `[PROFILE]` was passed.
///
/// A **headless** launch (piped stdin, OR the `[profile] default`
/// entry itself sets `headless = true`) must NOT open the interactive
/// picker — there may be no TTY, and stdin may be a consumed pipe. It
/// resolves `[profile] default` directly, erroring cleanly when no
/// default is configured. Only a genuinely interactive launch
/// (TTY + non-headless default) falls through to the picker, which
/// pre-selects the default under the cursor.
fn select_profile_without_positional(
    cfg: &Config,
    stdin_is_tty: bool,
    prompt_given: bool,
    cwd: Option<&Path>,
    config_patches: &[Value],
) -> Result<String> {
    // `[profile] default`'s own `headless` flag: a default profile that
    // forces headless can't be launched through the picker either, even
    // on a TTY (it needs a piped prompt, which the picker path can't
    // supply).
    let default_is_headless = cfg
        .profile
        .default
        .as_deref()
        .and_then(|id| cfg.profiles.iter().find(|p| p.id == id))
        .is_some_and(|p| p.headless.unwrap_or(false));

    // `--prompt` / `--file` also force headless (even on a TTY), so they
    // resolve the default directly instead of opening the picker.
    if !stdin_is_tty || default_is_headless || prompt_given {
        return cfg.profile.default.clone().ok_or_else(|| {
            anyhow::anyhow!(
                "headless launch requires a profile: pass one positionally \
                 (`hyprpilot <id>`) or set `[profile] default`"
            )
        });
    }

    Ok(picker::pick_profile(list_profiles(cfg, cwd, config_patches))?.id)
}

/// Decide the headless prompt for a launch. Effective headless is
/// `prompt_override.is_some() || profile_headless || !stdin_is_tty` (an
/// explicit `--prompt` / `--file`, a `headless` profile, or a piped
/// stdin each trigger it). Returns:
///
/// - `Ok(Some(prompt))` — the resolved prompt hyprpilot forwards to the
///   vendor. Source priority: an explicit `--prompt` / `--file` value
///   (`prompt_override`) wins over piped stdin; otherwise the buffered
///   stdin is used. An explicit `prompt_override` is delivered even
///   alongside trailing `-- <provider args>`: a deliberate `-p`/`-f`
///   COMPOSES with the escape hatch — the prompt rides its usual
///   delivery path (stdin for claude/codex, positional for opencode)
///   while the `-- <args>` still append to argv via the existing dedup.
/// - `Ok(None)` — interactive launch, OR the escape-hatch path where
///   the captain supplied trailing `-- <provider args>` **without** an
///   explicit `--prompt`/`--file`. In the escape-hatch case stdin is
///   NEVER read, so fd0 stays inherited and the vendor gets the raw pipe.
/// - `Err` — headless is active but no prompt source resolved: stdin is
///   a TTY (no pipe, no `--prompt`/`--file`), stdin was already drained
///   by `--with-config -`, or the resolved prompt is empty.
///
/// `read_stdin` is injected so the decision logic is unit-testable
/// without touching the process's real fd0.
fn headless_prompt(
    prompt_override: Option<String>,
    profile_headless: bool,
    stdin_is_tty: bool,
    has_provider_args: bool,
    stdin_consumed: bool,
    read_stdin: impl FnOnce() -> Result<String>,
) -> Result<Option<String>> {
    // An explicit `--prompt` / `--file` is a deliberate prompt: deliver
    // it BEFORE the escape hatch so it COMPOSES with trailing
    // `-- <provider args>` (prompt on its usual path + the extra flags
    // appended to argv). Honored even on a TTY (the non-pipe headless
    // entry point).
    if let Some(over) = prompt_override {
        let trimmed = over.trim();
        if trimmed.is_empty() {
            bail!("the headless prompt from `--prompt` / `--file` is empty");
        }
        return Ok(Some(trimmed.to_string()));
    }
    // Escape hatch: the captain supplied the vendor's own invocation via
    // trailing `-- <provider args>` and no explicit `--prompt`/`--file` —
    // never buffer or forward a prompt, even when a pipe would otherwise
    // trigger it, so fd0 stays inherited for the vendor's raw pipe.
    if has_provider_args {
        return Ok(None);
    }
    let effective = profile_headless || !stdin_is_tty;
    if !effective {
        return Ok(None);
    }
    if stdin_is_tty {
        bail!(
            "headless launch requires a prompt: pipe it on stdin \
             (e.g. `echo \"fix the bug\" | hyprpilot <profile>`) or pass `--prompt` / `--file`; stdin is a TTY"
        );
    }
    if stdin_consumed {
        // `--with-config -` already read stdin to EOF, so there is no
        // prompt left to buffer. Reading again would hit EOF and blame
        // an "empty prompt", so surface the real cause instead.
        bail!(
            "stdin was consumed by `--with-config -`; provide the prompt another way \
             (pass it via `--prompt` / `--file`, or forward it through a trailing `-- <provider args>`)"
        );
    }
    let raw = read_stdin()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("headless launch requires a non-empty prompt on stdin (or pass `--prompt` / `--file`)");
    }
    Ok(Some(trimmed.to_string()))
}

/// Buffer ALL of stdin into a `String` — the headless prompt source.
fn read_stdin_prompt() -> Result<String> {
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .context("headless: read prompt from stdin")?;
    Ok(buf)
}

pub(crate) fn list_profiles(cfg: &Config, cwd: Option<&Path>, config_patches: &[Value]) -> Vec<ProfileSummary> {
    let default_profile = cfg.profile.default.as_deref();
    cfg.profiles
        .iter()
        .map(|profile| {
            // On patch-resolution failure, fall back to the UNPATCHED
            // profile for the displayed fields but record the error so
            // the row is visibly marked — a broken patch must not pass
            // stale model/cwd off as the resolved shape (K-750 item 5).
            let (resolved, error) = match resolve_effective_profile(cfg, Some(profile.id.as_str()), config_patches) {
                Ok(resolved) => (resolved, None),
                Err(err) => {
                    let error = format!("{err:#}");
                    tracing::warn!(
                        profile = %profile.id,
                        %error,
                        "resolve: profile patch resolution failed; listing shows unpatched base values"
                    );
                    (profile.clone(), Some(error))
                }
            };
            ProfileSummary {
                id: resolved.id.clone(),
                agent: resolved.agent.clone(),
                model: resolved.model.clone(),
                cwd: cwd
                    .map(|cwd| cwd.display().to_string())
                    .or_else(|| resolved.cwd.as_ref().map(|cwd| cwd.display().to_string())),
                is_default: default_profile == Some(profile.id.as_str()),
                error,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;

    use super::*;
    use crate::config::{AgentConfig, AgentProvider, AgentsConfig, ProfileConfig, ProfileDefaults};

    fn cfg_with_profile_cwd() -> Config {
        Config {
            agents: AgentsConfig {
                agents: vec![AgentConfig {
                    id: "agent".into(),
                    provider: AgentProvider::ClaudeCode,
                    model: None,
                    effort: None,
                    command: "claude".into(),
                    args: Vec::new(),
                    cwd: None,
                    env: BTreeMap::new(),
                }],
            },
            profile: ProfileDefaults {
                default: Some("engineer".into()),
            },
            profiles: vec![ProfileConfig {
                id: "engineer".into(),
                agent: "agent".into(),
                model: None,
                effort: None,
                system_prompt: None,
                mcps: None,
                mcp: None,
                mode: None,
                cwd: Some(PathBuf::from("/configured")),
                headless: None,
                command: None,
                args: None,
                env: Default::default(),
            }],
            ..Default::default()
        }
    }

    #[test]
    fn profile_listing_prefers_effective_launch_cwd() {
        let profiles = list_profiles(&cfg_with_profile_cwd(), Some(Path::new("/launch")), &[]);

        assert_eq!(profiles[0].cwd.as_deref(), Some("/launch"));
    }

    #[test]
    fn profile_listing_falls_back_to_configured_cwd_without_launch_override() {
        let profiles = list_profiles(&cfg_with_profile_cwd(), None, &[]);

        assert_eq!(profiles[0].cwd.as_deref(), Some("/configured"));
    }

    // ── cwd precedence (K-740) ───────────────────────────────────

    #[test]
    fn launch_cwd_prefers_explicit_flag() {
        assert_eq!(
            resolve_launch_cwd(Some(PathBuf::from("/flag")), Some(PathBuf::from("/configured"))),
            Some(PathBuf::from("/flag")),
            "an explicit --cwd flag wins over the configured profile/agent cwd"
        );
    }

    #[test]
    fn launch_cwd_keeps_configured_cwd_when_flag_omitted() {
        // The K-740 regression: with no --cwd flag the configured
        // profile/agent cwd (already projected onto
        // `resolved.agent.cwd`) must survive — it used to be clobbered
        // by an eagerly-resolved current_dir().
        assert_eq!(
            resolve_launch_cwd(None, Some(PathBuf::from("/configured"))),
            Some(PathBuf::from("/configured"))
        );
    }

    #[test]
    fn launch_cwd_falls_back_to_current_dir_when_neither_set() {
        assert_eq!(resolve_launch_cwd(None, None), std::env::current_dir().ok());
    }

    #[test]
    fn configured_profile_cwd_resolves_when_flag_omitted() {
        // End-to-end at the resolve layer: a profile `cwd` projects
        // onto `resolved.agent.cwd`, and the launch-cwd precedence
        // then preserves it when `--cwd` is omitted — exactly the
        // value the `cli: profile resolved` info line surfaces.
        let cfg = cfg_with_profile_cwd();
        let (mut resolved, _profile) = resolve_into_instance_and_profile(&cfg, Some("engineer"), &[]).unwrap();
        resolved.agent.cwd = resolve_launch_cwd(None, resolved.agent.cwd.take());

        assert_eq!(resolved.agent.cwd.as_deref(), Some(Path::new("/configured")));
    }

    #[test]
    fn explicit_flag_overrides_configured_profile_cwd() {
        let cfg = cfg_with_profile_cwd();
        let (mut resolved, _profile) = resolve_into_instance_and_profile(&cfg, Some("engineer"), &[]).unwrap();
        resolved.agent.cwd = resolve_launch_cwd(Some(PathBuf::from("/flag")), resolved.agent.cwd.take());

        assert_eq!(resolved.agent.cwd.as_deref(), Some(Path::new("/flag")));
    }

    #[test]
    fn profile_listing_applies_profile_scoped_patches() {
        let mut cfg = cfg_with_profile_cwd();
        cfg.patches = Some(vec![serde_json::json!({
            "$match": { "profile": "engineer" },
            "model": "patched"
        })]);

        let profiles = list_profiles(&cfg, None, &[]);
        assert_eq!(profiles[0].model.as_deref(), Some("patched"));
    }

    #[test]
    fn profile_listing_flags_broken_patch_instead_of_showing_stale_data() {
        // An external patch that sets `model` to a non-string makes the
        // post-merge ProfileConfig fail to deserialize. The row must be
        // flagged (`error` set) rather than silently falling back to the
        // unpatched base values as if they resolved cleanly (K-750 item 5).
        let cfg = cfg_with_profile_cwd();
        let patches = vec![serde_json::json!({ "model": 123 })];
        let profiles = list_profiles(&cfg, None, &patches);

        assert!(
            profiles[0].error.is_some(),
            "a broken patch must flag the row, not silently show stale data"
        );
    }

    // ── headless prompt source (K-751) ───────────────────────────

    fn never_read() -> Result<String> {
        panic!("stdin must NOT be read on this path");
    }

    #[test]
    fn interactive_tty_no_pipe_reads_nothing() {
        // Not headless, stdin is a TTY, no provider args → interactive
        // launch. stdin must never be touched.
        let out = headless_prompt(None, false, true, false, false, never_read).unwrap();
        assert_eq!(out, None);
    }

    #[test]
    fn piped_stdin_auto_triggers_headless_and_buffers_prompt() {
        // The `echo … | hyprpilot <id>` path: stdin is piped, so
        // headless is auto-active and the buffered text becomes the
        // prompt (trailing newline trimmed).
        let out = headless_prompt(None, false, false, false, false, || Ok("fix the bug\n".into())).unwrap();
        assert_eq!(out.as_deref(), Some("fix the bug"));
    }

    #[test]
    fn profile_headless_flag_triggers_headless_on_pipe() {
        let out = headless_prompt(None, true, false, false, false, || Ok("do it".into())).unwrap();
        assert_eq!(out.as_deref(), Some("do it"));
    }

    #[test]
    fn prompt_override_wins_over_stdin_even_on_tty() {
        // `--prompt` / `--file` supplies the prompt inline: it triggers
        // headless even on a TTY and stdin is never read.
        let out = headless_prompt(Some("inline prompt".into()), false, true, false, false, never_read).unwrap();
        assert_eq!(out.as_deref(), Some("inline prompt"));
    }

    #[test]
    fn prompt_override_ignores_piped_stdin() {
        // An explicit override wins over a piped stdin (stdin never read).
        let out = headless_prompt(Some("flag wins".into()), false, false, false, false, never_read).unwrap();
        assert_eq!(out.as_deref(), Some("flag wins"));
    }

    #[test]
    fn prompt_override_trims_and_rejects_empty() {
        let err = headless_prompt(Some("  \n".into()), false, true, false, false, never_read).expect_err("must error");
        assert!(err.to_string().contains("`--prompt` / `--file` is empty"), "{err}");
    }

    #[test]
    fn profile_headless_on_tty_without_pipe_errors() {
        // `headless = true` but stdin is a TTY and no `--prompt`/`--file`
        // → no prompt source. The launch must error rather than open a
        // picker (impossible sans TTY prompt) or hang.
        let err = headless_prompt(None, true, true, false, false, never_read).expect_err("must error");
        assert!(err.to_string().contains("stdin is a TTY"), "{err}");
    }

    #[test]
    fn empty_piped_prompt_errors() {
        let err = headless_prompt(None, false, false, false, false, || Ok("   \n".into())).expect_err("must error");
        assert!(err.to_string().contains("non-empty prompt"), "{err}");
    }

    #[test]
    fn stdin_consumed_by_with_config_bails_with_targeted_error() {
        // `cat patch.json | hyprpilot <id> --with-config -`: the pipe
        // was drained building the overlay, so the auto-triggered
        // headless launch has no prompt left. Bail with the real cause
        // (never read stdin again — it would hit EOF and blame an
        // "empty prompt").
        let err = headless_prompt(None, false, false, false, true, never_read).expect_err("must error");
        assert!(err.to_string().contains("consumed by `--with-config -`"), "{err}");
    }

    #[test]
    fn provider_args_are_the_escape_hatch_and_never_consume_stdin() {
        // Trailing `-- <provider args>` WITHOUT an explicit
        // `--prompt`/`--file` → hyprpilot must NOT read stdin (fd0 stays
        // inherited for the vendor's raw pipe) and must NOT generate a
        // prompt projection, even though the piped stdin makes headless
        // "effective".
        let out = headless_prompt(None, false, false, true, false, never_read).unwrap();
        assert_eq!(out, None, "escape hatch: no buffered prompt");

        // Same with the profile `headless` flag set.
        let out = headless_prompt(None, true, true, true, false, never_read).unwrap();
        assert_eq!(out, None);
    }

    #[test]
    fn prompt_override_delivered_even_with_provider_args() {
        // An explicit `--prompt`/`--file` COMPOSES with the escape hatch:
        // the prompt is delivered (on its usual vendor path) while the
        // trailing `-- <provider args>` still append to argv. stdin is
        // never read — the override is the prompt source.
        let out = headless_prompt(Some("p".into()), false, false, true, false, never_read).unwrap();
        assert_eq!(out.as_deref(), Some("p"), "explicit prompt composes with `-- <args>`");

        // Also on a TTY with the profile `headless` flag set.
        let out = headless_prompt(Some("do it".into()), true, true, true, false, never_read).unwrap();
        assert_eq!(out.as_deref(), Some("do it"));

        // An empty override still errors, even with provider args.
        let err = headless_prompt(Some("  \n".into()), false, false, true, false, never_read)
            .expect_err("empty override must error even with provider args");
        assert!(err.to_string().contains("`--prompt` / `--file` is empty"), "{err}");
    }

    // ── headless profile selection without a positional (K-751) ──

    #[test]
    fn headless_no_positional_uses_default_not_picker() {
        // Piped stdin (`stdin_is_tty = false`) + no positional profile
        // must resolve `[profile] default` directly — never the picker,
        // which would return a `NotInteractive` error with no TTY.
        let cfg = cfg_with_profile_cwd(); // default = "engineer"
        let id = select_profile_without_positional(&cfg, false, false, None, &[]).unwrap();
        assert_eq!(id, "engineer");
    }

    #[test]
    fn headless_no_positional_no_default_errors_cleanly() {
        let mut cfg = cfg_with_profile_cwd();
        cfg.profile.default = None;
        let err = select_profile_without_positional(&cfg, false, false, None, &[]).expect_err("must error");
        assert!(err.to_string().contains("headless launch requires a profile"), "{err}");
    }

    #[test]
    fn headless_default_profile_bypasses_picker_even_on_tty() {
        // A `[profile] default` that itself sets `headless = true` must
        // resolve directly even at an interactive TTY — the picker path
        // can't feed it the piped prompt it needs.
        let mut cfg = cfg_with_profile_cwd();
        cfg.profiles[0].headless = Some(true);
        let id = select_profile_without_positional(&cfg, true, false, None, &[]).unwrap();
        assert_eq!(id, "engineer");
    }

    #[test]
    fn prompt_flag_bypasses_picker_on_tty() {
        // `--prompt` / `--file` (prompt_given = true) forces headless, so
        // a bare launch on a TTY resolves the default directly rather
        // than opening the picker.
        let cfg = cfg_with_profile_cwd(); // default = "engineer"
        let id = select_profile_without_positional(&cfg, true, true, None, &[]).unwrap();
        assert_eq!(id, "engineer");
    }
}

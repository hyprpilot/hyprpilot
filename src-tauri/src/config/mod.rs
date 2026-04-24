mod validations;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use garde::Validate;
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};

use crate::paths;
use validations::{
    validate_agent_default_id, validate_agents_ids, validate_default_profile_id, validate_profile_agent_references,
    validate_profile_tool_globs, validate_profiles_ids,
};

const DEFAULTS: &str = include_str!("defaults.toml");

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    #[garde(dive)]
    pub daemon: Daemon,
    #[garde(dive)]
    pub logging: Logging,
    #[garde(dive)]
    pub ui: Ui,
    /// `[[agents]]` + `[agent]` at TOML root, flattened here so
    /// `AgentsConfig` stays the single Rust-side unit.
    #[garde(dive)]
    #[garde(custom(validate_default_profile_id(&self.profiles)))]
    #[serde(flatten)]
    pub agents: AgentsConfig,
    /// `[[profiles]]` at TOML root. Each profile binds an agent id to an
    /// optional model override + optional system prompt; resolved into a
    /// flat `ResolvedInstance` at `session/submit` time.
    #[garde(dive)]
    #[garde(custom(validate_profiles_ids))]
    #[garde(custom(validate_profile_agent_references(&self.agents.agents)))]
    #[serde(default)]
    pub profiles: Vec<ProfileConfig>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct Daemon {
    #[garde(skip)]
    pub socket: Option<PathBuf>,
    #[garde(dive)]
    pub window: Window,
}

/// `[daemon.window]`. See CLAUDE.md "Window surface" for why `layer`
/// and `keyboard_interactivity` aren't config knobs.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct Window {
    #[garde(skip)]
    pub mode: Option<WindowMode>,
    #[garde(inner(length(min = 1)))]
    pub output: Option<String>,
    #[garde(dive)]
    pub anchor: AnchorWindow,
    #[garde(dive)]
    pub center: CenterWindow,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WindowMode {
    #[default]
    Anchor,
    Center,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    Top,
    #[default]
    Right,
    Bottom,
    Left,
}

/// Anchor-mode geometry. Unset `height` → full-height (top+bottom+edge pin).
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct AnchorWindow {
    #[garde(skip)]
    pub edge: Option<Edge>,
    #[garde(inner(range(min = 0, max = 10_000)))]
    pub margin: Option<i32>,
    #[garde(dive)]
    pub width: Option<Dimension>,
    #[garde(dive)]
    pub height: Option<Dimension>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct CenterWindow {
    #[garde(dive)]
    pub width: Option<Dimension>,
    #[garde(dive)]
    pub height: Option<Dimension>,
}

/// Pixel literal or a `"N%"` string. TOML accepts either at the same
/// key; custom `Deserialize` handles the `%` suffix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Dimension {
    Pixels(u32),
    Percent(u8),
}

impl Validate for Dimension {
    type Context = ();

    fn validate_into(&self, _ctx: &Self::Context, parent: &mut dyn FnMut() -> garde::Path, report: &mut garde::Report) {
        match *self {
            Dimension::Pixels(0) => {
                report.append(parent(), garde::Error::new("pixel dimension must be >= 1"));
            }
            Dimension::Pixels(px) if px > 10_000 => {
                report.append(
                    parent(),
                    garde::Error::new(format!("pixel dimension {px} exceeds 10000 — refusing absurd size")),
                );
            }
            Dimension::Pixels(_) => {}
            Dimension::Percent(p) if (1..=100).contains(&p) => {}
            Dimension::Percent(p) => {
                report.append(parent(), garde::Error::new(format!("percent must be 1..=100, got {p}")));
            }
        }
    }
}

impl<'de> Deserialize<'de> for Dimension {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error as _;

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Int(i64),
            Str(String),
        }

        match Raw::deserialize(deserializer)? {
            Raw::Int(n) => {
                let px: u32 = n.try_into().map_err(|_| {
                    D::Error::custom(format!("pixel dimension must be a non-negative integer, got {n}"))
                })?;

                Ok(Dimension::Pixels(px))
            }
            Raw::Str(s) => {
                let trimmed = s.trim();

                let digits = trimmed
                    .strip_suffix('%')
                    .ok_or_else(|| D::Error::custom(format!("dimension string must end with '%', got {s:?}")))?;

                let n: u8 = digits
                    .parse()
                    .map_err(|e| D::Error::custom(format!("invalid percent value {digits:?}: {e}")))?;

                Ok(Dimension::Percent(n))
            }
        }
    }
}

/// `#RRGGBB` or `#RRGGBBAA`. `#[serde(transparent)]` keeps the wire
/// shape a bare string; `impl Validate` runs under `#[garde(dive)]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HexColor(pub String);

impl Validate for HexColor {
    type Context = ();

    fn validate_into(&self, _ctx: &Self::Context, parent: &mut dyn FnMut() -> garde::Path, report: &mut garde::Report) {
        let v = &self.0;
        let ok = v.starts_with('#') && matches!(v.len(), 7 | 9) && v[1..].chars().all(|c| c.is_ascii_hexdigit());
        if !ok {
            report.append(
                parent(),
                garde::Error::new(format!("must be a hex color (#RRGGBB or #RRGGBBAA), got '{v}'")),
            );
        }
    }
}

impl AsRef<str> for HexColor {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::ops::Deref for HexColor {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl From<&str> for HexColor {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

impl From<String> for HexColor {
    fn from(s: String) -> Self {
        Self(s)
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct Logging {
    /// Unknown levels reject at TOML parse (serde closed enum).
    #[garde(skip)]
    pub level: Option<crate::logging::LogLevel>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct Ui {
    #[garde(dive)]
    pub theme: Theme,
}

/// Palette tokens surfaced to the webview as CSS custom properties. Each
/// leaf is `Option<String>` so partial overrides in user TOML compose
/// cleanly over the compiled defaults layer — `merge_theme` walks the tree
/// field-by-field using `or`. Leaf naming is consistent across groups:
/// `default` is the base value; siblings are variants.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct Theme {
    #[garde(dive)]
    pub font: ThemeFont,
    #[garde(dive)]
    pub window: ThemeWindow,
    #[garde(dive)]
    pub surface: ThemeSurface,
    #[garde(dive)]
    pub fg: ThemeFg,
    #[garde(dive)]
    pub border: ThemeBorder,
    #[garde(dive)]
    pub accent: ThemeAccent,
    #[garde(dive)]
    pub state: ThemeState,
    #[garde(dive)]
    pub kind: ThemeKind,
    #[garde(dive)]
    pub status: ThemeStatus,
    #[garde(dive)]
    pub permission: ThemePermission,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeFont {
    #[garde(inner(length(min = 1)))]
    pub mono: Option<String>,
    #[garde(inner(length(min = 1)))]
    pub sans: Option<String>,
}

/// Window frame tokens. `default` = background fill; `edge` = accent stripe.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeWindow {
    #[garde(dive)]
    pub default: Option<HexColor>,
    #[garde(dive)]
    pub edge: Option<HexColor>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeSurface {
    #[garde(dive)]
    pub default: Option<HexColor>,
    #[garde(dive)]
    pub bg: Option<HexColor>,
    #[garde(dive)]
    pub alt: Option<HexColor>,
    #[garde(dive)]
    pub card: SurfaceCard,
    #[garde(dive)]
    pub compose: Option<HexColor>,
    #[garde(dive)]
    pub text: Option<HexColor>,
}

/// Message cards, keyed by speaker.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct SurfaceCard {
    #[garde(dive)]
    pub user: Card,
    #[garde(dive)]
    pub assistant: Card,
}

/// One card's tokens. `bg` today; future accent/border/fg slot in alongside.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct Card {
    #[garde(dive)]
    pub bg: Option<HexColor>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeFg {
    #[garde(dive)]
    pub default: Option<HexColor>,
    #[garde(dive)]
    pub ink_2: Option<HexColor>,
    #[garde(dive)]
    pub dim: Option<HexColor>,
    #[garde(dive)]
    pub faint: Option<HexColor>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeBorder {
    #[garde(dive)]
    pub default: Option<HexColor>,
    #[garde(dive)]
    pub soft: Option<HexColor>,
    #[garde(dive)]
    pub focus: Option<HexColor>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeAccent {
    #[garde(dive)]
    pub default: Option<HexColor>,
    #[garde(dive)]
    pub user: Option<HexColor>,
    #[garde(dive)]
    pub user_soft: Option<HexColor>,
    #[garde(dive)]
    pub assistant: Option<HexColor>,
    #[garde(dive)]
    pub assistant_soft: Option<HexColor>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeState {
    #[garde(dive)]
    pub idle: Option<HexColor>,
    #[garde(dive)]
    pub stream: Option<HexColor>,
    #[garde(dive)]
    pub pending: Option<HexColor>,
    #[garde(dive)]
    pub awaiting: Option<HexColor>,
    #[garde(dive)]
    pub working: Option<HexColor>,
}

/// Per-tool-family dispatch colours keyed by `ToolCall.kind`.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeKind {
    #[garde(dive)]
    pub read: Option<HexColor>,
    #[garde(dive)]
    pub write: Option<HexColor>,
    #[garde(dive)]
    pub bash: Option<HexColor>,
    #[garde(dive)]
    pub search: Option<HexColor>,
    #[garde(dive)]
    pub agent: Option<HexColor>,
    #[garde(dive)]
    pub think: Option<HexColor>,
    #[garde(dive)]
    pub terminal: Option<HexColor>,
    #[garde(dive)]
    pub acp: Option<HexColor>,
}

/// Toast / banner status hues. Distinct from the `state` machine —
/// `ok`/`warn`/`err` are one-shot notifications, not phase transitions.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeStatus {
    #[garde(dive)]
    pub ok: Option<HexColor>,
    #[garde(dive)]
    pub warn: Option<HexColor>,
    #[garde(dive)]
    pub err: Option<HexColor>,
}

/// Warm-brown panel fills for the permission stack.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct ThemePermission {
    #[garde(dive)]
    pub bg: Option<HexColor>,
    #[garde(dive)]
    pub bg_active: Option<HexColor>,
}

/// `[[agents]]` registry + `[agent]` global scope. Entries override
/// by `id`; new ids append. Cross-field check on `agent.default`
/// closes over `&self.agents` via the garde custom hook.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct AgentsConfig {
    #[garde(dive)]
    #[garde(custom(validate_agents_ids))]
    pub agents: Vec<AgentConfig>,
    #[garde(dive)]
    #[garde(custom(validate_agent_default_id(&self.agents)))]
    pub agent: AgentDefaults,
}

/// `[agent]` — global agent-scope config. Future timeout / cwd /
/// env knobs slot in here.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct AgentDefaults {
    #[garde(skip)]
    pub default: Option<String>,
    #[garde(skip)]
    pub default_profile: Option<String>,
}

/// One `[[agents]]` entry. No `permission_policy` — vendors own
/// that; client-side auto-accept/reject is a future
/// `PermissionController` issue (see CLAUDE.md).
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Validate)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    #[garde(length(min = 1))]
    pub id: String,
    #[garde(skip)]
    pub provider: AgentProvider,
    /// Vendor-translated at spawn time: env var or CLI flag per vendor.
    #[garde(skip)]
    pub model: Option<String>,
    /// Missing → vendor's default command.
    #[garde(inner(length(min = 1)))]
    pub command: Option<String>,
    #[garde(skip)]
    #[serde(default)]
    pub args: Vec<String>,
    /// Missing → `std::env::current_dir()` at `new_session` time.
    #[garde(skip)]
    pub cwd: Option<PathBuf>,
    #[garde(skip)]
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

/// Closed enum — each variant maps to an `AcpAgent` impl. Wire
/// names are explicit to avoid `acp-open-code` for `AcpOpenCode`.
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
pub enum AgentProvider {
    #[default]
    #[serde(rename = "acp-claude-code")]
    AcpClaudeCode,
    #[serde(rename = "acp-codex")]
    AcpCodex,
    #[serde(rename = "acp-opencode")]
    AcpOpenCode,
}

/// One `[[profiles]]` entry. Binds an agent id to an optional model
/// override + optional system prompt. Exactly one of `system_prompt`
/// / `system_prompt_file` may be set; the file path is read at
/// resolve time, not at spawn time, so a missing file fails loudly.
///
/// `auto_accept_tools` / `auto_reject_tools` are glob patterns matched
/// against the ACP `ToolKind` wire name (falling back to the tool's
/// title when kind is absent) at permission-request time. Reject
/// beats accept; misses fall through to a user prompt. Patterns are
/// validated at load time — empty strings and invalid globs reject
/// with the profile id + offending pattern in the error.
///
/// Allowlists only apply to sessions that resolve through a profile.
/// Bare-agent sessions (no profile id on submit) always prompt the
/// user — the fallback to `[agent] default_profile` happens at
/// `ResolvedInstance` time in `adapters::profile`, so setting a
/// `default_profile` ensures allowlists are honored on unlabelled
/// submits.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Validate)]
#[serde(deny_unknown_fields)]
pub struct ProfileConfig {
    #[garde(length(min = 1))]
    pub id: String,
    #[garde(length(min = 1))]
    pub agent: String,
    #[garde(inner(length(min = 1)))]
    pub model: Option<String>,
    #[garde(inner(length(min = 1)))]
    pub system_prompt: Option<String>,
    #[garde(skip)]
    pub system_prompt_file: Option<PathBuf>,
    #[serde(default)]
    #[garde(custom(validate_profile_tool_globs(&self.id)))]
    pub auto_accept_tools: Vec<String>,
    #[serde(default)]
    #[garde(custom(validate_profile_tool_globs(&self.id)))]
    pub auto_reject_tools: Vec<String>,
}

impl ProfileConfig {
    /// Compile the accept/reject glob sets. Call once per resolved
    /// instance; `GlobSet` is immutable after build. Patterns are
    /// validated at TOML load time, so `unwrap()` on the build steps
    /// would also be fine — we return `Result` for robustness against
    /// hand-constructed `ProfileConfig` values in tests.
    pub fn compile_tool_globs(&self) -> anyhow::Result<(GlobSet, GlobSet)> {
        Ok((
            build_globset(&self.auto_accept_tools)?,
            build_globset(&self.auto_reject_tools)?,
        ))
    }
}

fn build_globset(patterns: &[String]) -> anyhow::Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for p in patterns {
        builder.add(Glob::new(p).with_context(|| format!("invalid glob pattern '{p}'"))?);
    }
    builder.build().context("failed to compile glob set")
}

impl ProfileConfig {
    /// Reject entries setting both `system_prompt` and
    /// `system_prompt_file` — they're wire-exclusive. Separate from
    /// the derive walk because it's a single-field pair check we
    /// want reported at `Config::validate()` time.
    pub(crate) fn validate_prompt_source(&self) -> garde::Result {
        if self.system_prompt.is_some() && self.system_prompt_file.is_some() {
            return Err(garde::Error::new(format!(
                "profile '{}' sets both system_prompt and system_prompt_file — pick one",
                self.id
            )));
        }
        Ok(())
    }
}

pub fn load(cli_path: Option<&Path>, profile: Option<&str>) -> Result<Config> {
    tracing::info!(cli_path = ?cli_path, profile = ?profile, "config::load: loading layers");
    let mut layers: Vec<String> = vec![DEFAULTS.to_string()];

    match cli_path {
        Some(p) => {
            if !p.exists() {
                tracing::error!(path = %p.display(), "config::load: cli path missing");
                bail!("config file not found: {}", p.display());
            }
            tracing::debug!(path = %p.display(), "config::load: reading cli-provided layer");
            layers.push(read_layer(p)?);
        }
        None => {
            let default = paths::config_file();
            if default.exists() {
                tracing::debug!(path = %default.display(), "config::load: reading default layer");
                layers.push(read_layer(&default)?);
            }
        }
    }

    if let Some(name) = profile {
        let p = paths::profile_config_file(name);
        if !p.exists() {
            tracing::error!(profile = name, path = %p.display(), "config::load: profile not found");
            bail!("profile '{name}' not found at {}", p.display());
        }
        tracing::debug!(profile = name, path = %p.display(), "config::load: reading profile layer");
        layers.push(read_layer(&p)?);
    }

    let cfg = layers
        .iter()
        .enumerate()
        .try_fold(Config::default(), |acc, (idx, body)| -> Result<Config> {
            let layer: Config = toml::from_str(body)
                .map_err(|e| {
                    tracing::error!(layer_index = idx, err = %e, "config::load: TOML parse failed");
                    e
                })
                .context("failed to parse TOML layer")?;
            Ok(acc.merge(layer))
        })?;

    tracing::info!(
        layers = layers.len(),
        agents = cfg.agents.agents.len(),
        profiles = cfg.profiles.len(),
        default_agent = ?cfg.agents.agent.default,
        default_profile = ?cfg.agents.agent.default_profile,
        "config::load: layers merged"
    );

    Ok(cfg)
}

fn read_layer(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("failed to read config {}", path.display()))
}

/// Layer two config values — `other` wins. Drives the fold in
/// `load()`: `acc.merge(layer)`. Scalar `Option<T>` leaves are
/// handled by the blanket impl; structs recurse via trivial
/// field-by-field impls; `AgentsConfig` has keyed-list semantics.
pub(crate) trait Merge: Sized {
    fn merge(self, other: Self) -> Self;
}

impl<T> Merge for Option<T> {
    fn merge(self, other: Self) -> Self {
        other.or(self)
    }
}

impl Merge for Config {
    fn merge(self, other: Self) -> Self {
        Self {
            daemon: self.daemon.merge(other.daemon),
            logging: self.logging.merge(other.logging),
            ui: self.ui.merge(other.ui),
            agents: self.agents.merge(other.agents),
            profiles: merge_profiles(self.profiles, other.profiles),
        }
    }
}

/// Keyed-list merge for `[[profiles]]`. Mirrors `AgentsConfig`'s
/// agent-entry semantics: override by `id`, append new ids in order.
/// Whole-entry replace — partial field merge inside a profile would
/// read as "override system_prompt, keep old model", which is
/// surprising.
fn merge_profiles(base: Vec<ProfileConfig>, over: Vec<ProfileConfig>) -> Vec<ProfileConfig> {
    let mut out: Vec<ProfileConfig> = Vec::with_capacity(base.len() + over.len());
    let base_ids: std::collections::HashSet<String> = base.iter().map(|p| p.id.clone()).collect();

    for b in base {
        match over.iter().find(|o| o.id == b.id) {
            Some(override_entry) => out.push(override_entry.clone()),
            None => out.push(b),
        }
    }

    for o in over {
        if !base_ids.contains(&o.id) {
            out.push(o);
        }
    }

    out
}

impl Merge for Daemon {
    fn merge(self, other: Self) -> Self {
        Self {
            socket: self.socket.merge(other.socket),
            window: self.window.merge(other.window),
        }
    }
}

impl Merge for Logging {
    fn merge(self, other: Self) -> Self {
        Self {
            level: self.level.merge(other.level),
        }
    }
}

impl Merge for Ui {
    fn merge(self, other: Self) -> Self {
        Self {
            theme: self.theme.merge(other.theme),
        }
    }
}

impl Merge for Window {
    fn merge(self, other: Self) -> Self {
        Self {
            mode: self.mode.merge(other.mode),
            output: self.output.merge(other.output),
            anchor: self.anchor.merge(other.anchor),
            center: self.center.merge(other.center),
        }
    }
}

impl Merge for AnchorWindow {
    fn merge(self, other: Self) -> Self {
        Self {
            edge: self.edge.merge(other.edge),
            margin: self.margin.merge(other.margin),
            width: self.width.merge(other.width),
            height: self.height.merge(other.height),
        }
    }
}

impl Merge for CenterWindow {
    fn merge(self, other: Self) -> Self {
        Self {
            width: self.width.merge(other.width),
            height: self.height.merge(other.height),
        }
    }
}

impl Merge for Theme {
    fn merge(self, other: Self) -> Self {
        Self {
            font: self.font.merge(other.font),
            window: self.window.merge(other.window),
            surface: self.surface.merge(other.surface),
            fg: self.fg.merge(other.fg),
            border: self.border.merge(other.border),
            accent: self.accent.merge(other.accent),
            state: self.state.merge(other.state),
            kind: self.kind.merge(other.kind),
            status: self.status.merge(other.status),
            permission: self.permission.merge(other.permission),
        }
    }
}

impl Merge for ThemeFont {
    fn merge(self, other: Self) -> Self {
        Self {
            mono: self.mono.merge(other.mono),
            sans: self.sans.merge(other.sans),
        }
    }
}

impl Merge for ThemeWindow {
    fn merge(self, other: Self) -> Self {
        Self {
            default: self.default.merge(other.default),
            edge: self.edge.merge(other.edge),
        }
    }
}

impl Merge for ThemeSurface {
    fn merge(self, other: Self) -> Self {
        Self {
            default: self.default.merge(other.default),
            bg: self.bg.merge(other.bg),
            alt: self.alt.merge(other.alt),
            card: self.card.merge(other.card),
            compose: self.compose.merge(other.compose),
            text: self.text.merge(other.text),
        }
    }
}

impl Merge for SurfaceCard {
    fn merge(self, other: Self) -> Self {
        Self {
            user: self.user.merge(other.user),
            assistant: self.assistant.merge(other.assistant),
        }
    }
}

impl Merge for Card {
    fn merge(self, other: Self) -> Self {
        Self {
            bg: self.bg.merge(other.bg),
        }
    }
}

impl Merge for ThemeFg {
    fn merge(self, other: Self) -> Self {
        Self {
            default: self.default.merge(other.default),
            ink_2: self.ink_2.merge(other.ink_2),
            dim: self.dim.merge(other.dim),
            faint: self.faint.merge(other.faint),
        }
    }
}

impl Merge for ThemeBorder {
    fn merge(self, other: Self) -> Self {
        Self {
            default: self.default.merge(other.default),
            soft: self.soft.merge(other.soft),
            focus: self.focus.merge(other.focus),
        }
    }
}

impl Merge for ThemeAccent {
    fn merge(self, other: Self) -> Self {
        Self {
            default: self.default.merge(other.default),
            user: self.user.merge(other.user),
            user_soft: self.user_soft.merge(other.user_soft),
            assistant: self.assistant.merge(other.assistant),
            assistant_soft: self.assistant_soft.merge(other.assistant_soft),
        }
    }
}

impl Merge for ThemeState {
    fn merge(self, other: Self) -> Self {
        Self {
            idle: self.idle.merge(other.idle),
            stream: self.stream.merge(other.stream),
            pending: self.pending.merge(other.pending),
            awaiting: self.awaiting.merge(other.awaiting),
            working: self.working.merge(other.working),
        }
    }
}

impl Merge for ThemeKind {
    fn merge(self, other: Self) -> Self {
        Self {
            read: self.read.merge(other.read),
            write: self.write.merge(other.write),
            bash: self.bash.merge(other.bash),
            search: self.search.merge(other.search),
            agent: self.agent.merge(other.agent),
            think: self.think.merge(other.think),
            terminal: self.terminal.merge(other.terminal),
            acp: self.acp.merge(other.acp),
        }
    }
}

impl Merge for ThemeStatus {
    fn merge(self, other: Self) -> Self {
        Self {
            ok: self.ok.merge(other.ok),
            warn: self.warn.merge(other.warn),
            err: self.err.merge(other.err),
        }
    }
}

impl Merge for ThemePermission {
    fn merge(self, other: Self) -> Self {
        Self {
            bg: self.bg.merge(other.bg),
            bg_active: self.bg_active.merge(other.bg_active),
        }
    }
}

/// Keyed-list merge: override by `id`, append new ids in order.
/// Whole-entry replace (no field-level merge inside `AgentConfig`)
/// — "override provider, keep old command" would be surprising.
/// Duplicate ids inside a single layer survive so
/// `validate_agents_ids` can flag them.
impl Merge for AgentsConfig {
    fn merge(self, other: Self) -> Self {
        let mut out: Vec<AgentConfig> = Vec::with_capacity(self.agents.len() + other.agents.len());
        let base_ids: std::collections::HashSet<String> = self.agents.iter().map(|a| a.id.clone()).collect();

        for b in self.agents {
            match other.agents.iter().find(|l| l.id == b.id) {
                Some(override_entry) => out.push(override_entry.clone()),
                None => out.push(b),
            }
        }

        for l in other.agents {
            if !base_ids.contains(&l.id) {
                out.push(l);
            }
        }

        Self {
            agents: out,
            agent: self.agent.merge(other.agent),
        }
    }
}

impl Merge for AgentDefaults {
    fn merge(self, other: Self) -> Self {
        Self {
            default: self.default.merge(other.default),
            default_profile: self.default_profile.merge(other.default_profile),
        }
    }
}

impl Config {
    /// Run garde's tree walk (every predicate including cross-field
    /// rules wired via higher-order `custom(fn(&self.x))` hooks).
    pub fn validate(&self) -> Result<()> {
        <Self as Validate>::validate(self).map_err(|report| {
            tracing::error!(%report, "config::validate: garde report");
            anyhow!("config is invalid:\n{report}")
        })?;
        for p in &self.profiles {
            p.validate_prompt_source().map_err(|e| {
                tracing::error!(profile = %p.id, err = %e, "config::validate: profile prompt source clash");
                anyhow!("config is invalid: profiles[{}]: {e}", p.id)
            })?;
        }
        tracing::debug!("config::validate: config validated");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;

    use super::*;

    fn write_tmp(name: &str, body: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("hyprpilot-test-{}-{}", std::process::id(), name));
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(body.as_bytes()).unwrap();

        path
    }

    #[test]
    fn defaults_parse_and_validate() {
        let cfg: Config = toml::from_str(DEFAULTS).expect("defaults must parse");
        cfg.validate().expect("defaults must validate");
    }

    /// The daemon consumes several `Option<T>` window fields via `.expect()`
    /// rather than carrying a second-layer Rust default — defaults.toml is
    /// the single source of truth. If a field is removed from the TOML
    /// without removing the `.expect()` call, the daemon panics at startup;
    /// this test fails before we ship that.
    #[test]
    fn defaults_populate_every_daemon_window_field() {
        let cfg: Config = toml::from_str(DEFAULTS).expect("defaults must parse");
        let w = &cfg.daemon.window;

        assert!(w.mode.is_some(), "daemon.window.mode");
        assert!(w.anchor.edge.is_some(), "daemon.window.anchor.edge");
        assert!(w.anchor.margin.is_some(), "daemon.window.anchor.margin");
        assert!(w.anchor.width.is_some(), "daemon.window.anchor.width");
        // anchor.height intentionally optional — None means full-height fill.
        assert!(w.center.width.is_some(), "daemon.window.center.width");
        assert!(w.center.height.is_some(), "daemon.window.center.height");
    }

    #[test]
    fn defaults_populate_every_theme_token() {
        let cfg: Config = toml::from_str(DEFAULTS).expect("defaults must parse");
        let t = &cfg.ui.theme;

        assert!(t.font.mono.is_some(), "font.mono");
        assert!(t.font.sans.is_some(), "font.sans");

        for (n, v) in [("window.default", &t.window.default), ("window.edge", &t.window.edge)] {
            assert!(v.is_some(), "{n}");
        }

        for (n, v) in [
            ("surface.default", &t.surface.default),
            ("surface.bg", &t.surface.bg),
            ("surface.alt", &t.surface.alt),
            ("surface.card.user.bg", &t.surface.card.user.bg),
            ("surface.card.assistant.bg", &t.surface.card.assistant.bg),
            ("surface.compose", &t.surface.compose),
            ("surface.text", &t.surface.text),
        ] {
            assert!(v.is_some(), "{n}");
        }

        for (n, v) in [
            ("fg.default", &t.fg.default),
            ("fg.ink_2", &t.fg.ink_2),
            ("fg.dim", &t.fg.dim),
            ("fg.faint", &t.fg.faint),
        ] {
            assert!(v.is_some(), "{n}");
        }

        for (n, v) in [
            ("border.default", &t.border.default),
            ("border.soft", &t.border.soft),
            ("border.focus", &t.border.focus),
        ] {
            assert!(v.is_some(), "{n}");
        }

        for (n, v) in [
            ("accent.default", &t.accent.default),
            ("accent.user", &t.accent.user),
            ("accent.user_soft", &t.accent.user_soft),
            ("accent.assistant", &t.accent.assistant),
            ("accent.assistant_soft", &t.accent.assistant_soft),
        ] {
            assert!(v.is_some(), "{n}");
        }

        for (n, v) in [
            ("state.idle", &t.state.idle),
            ("state.stream", &t.state.stream),
            ("state.pending", &t.state.pending),
            ("state.awaiting", &t.state.awaiting),
            ("state.working", &t.state.working),
        ] {
            assert!(v.is_some(), "{n}");
        }

        for (n, v) in [
            ("kind.read", &t.kind.read),
            ("kind.write", &t.kind.write),
            ("kind.bash", &t.kind.bash),
            ("kind.search", &t.kind.search),
            ("kind.agent", &t.kind.agent),
            ("kind.think", &t.kind.think),
            ("kind.terminal", &t.kind.terminal),
            ("kind.acp", &t.kind.acp),
        ] {
            assert!(v.is_some(), "{n}");
        }

        for (n, v) in [
            ("status.ok", &t.status.ok),
            ("status.warn", &t.status.warn),
            ("status.err", &t.status.err),
        ] {
            assert!(v.is_some(), "{n}");
        }

        for (n, v) in [
            ("permission.bg", &t.permission.bg),
            ("permission.bg_active", &t.permission.bg_active),
        ] {
            assert!(v.is_some(), "{n}");
        }
    }

    #[test]
    fn load_merges_cli_path_over_defaults() {
        let p = write_tmp(
            "merge.toml",
            r#"
[logging]
level = "debug"
"#,
        );
        let cfg = load(Some(&p), None).expect("load");
        assert_eq!(cfg.logging.level, Some(crate::logging::LogLevel::Debug));
        fs::remove_file(&p).ok();
    }

    #[test]
    fn theme_override_preserves_untouched_tokens() {
        let p = write_tmp(
            "theme.toml",
            r##"
[ui.theme.window]
default = "#101418"
edge = "#ff00aa"

[ui.theme.border]
focus = "#00ff00"

[ui.theme.surface.card.user]
bg = "#ff8800"

[ui.theme.kind]
read = "#123456"
"##,
        );
        let cfg = load(Some(&p), None).expect("load");

        // Overridden.
        assert_eq!(cfg.ui.theme.window.default.as_deref(), Some("#101418"));
        assert_eq!(cfg.ui.theme.window.edge.as_deref(), Some("#ff00aa"));
        assert_eq!(cfg.ui.theme.border.focus.as_deref(), Some("#00ff00"));
        assert_eq!(cfg.ui.theme.surface.card.user.bg.as_deref(), Some("#ff8800"));
        assert_eq!(cfg.ui.theme.kind.read.as_deref(), Some("#123456"));

        // Untouched in the same groups still fall back to defaults.
        assert_eq!(cfg.ui.theme.border.default.as_deref(), Some("#20242e"));
        assert_eq!(cfg.ui.theme.border.soft.as_deref(), Some("#2b2f3b"));
        assert_eq!(cfg.ui.theme.surface.card.assistant.bg.as_deref(), Some("#12141a"));
        assert_eq!(cfg.ui.theme.surface.compose.as_deref(), Some("#181b22"));
        assert_eq!(cfg.ui.theme.kind.write.as_deref(), Some("#e480d4"));

        // Groups not mentioned at all still come from defaults.
        assert_eq!(cfg.ui.theme.fg.default.as_deref(), Some("#d8dde5"));
        assert_eq!(cfg.ui.theme.accent.default.as_deref(), Some("#c99bf0"));
        assert_eq!(cfg.ui.theme.status.ok.as_deref(), Some("#7fcf8a"));
        assert_eq!(cfg.ui.theme.permission.bg.as_deref(), Some("#18130a"));

        fs::remove_file(&p).ok();
    }

    #[test]
    fn load_errors_when_cli_path_missing() {
        let missing = PathBuf::from("/nonexistent/hyprpilot-test-never.toml");
        let err = load(Some(&missing), None).expect_err("should error");
        assert!(err.to_string().contains("config file not found"));
    }

    #[test]
    fn load_rejects_unknown_fields() {
        let p = write_tmp("unknown.toml", "bogus = true\n");
        let err = load(Some(&p), None).expect_err("should error");
        assert!(err.to_string().contains("failed to parse TOML layer"));
        fs::remove_file(&p).ok();
    }

    #[test]
    fn toml_rejects_bad_log_level() {
        // With `LogLevel` as a closed enum, unknown levels fail at
        // TOML parse time rather than at validate time. anyhow's
        // top-level message is "failed to parse TOML layer"; the
        // serde detail lives in the underlying source.
        let p = write_tmp(
            "bad-level.toml",
            r#"
[logging]
level = "verbose"
"#,
        );
        let err = load(Some(&p), None).expect_err("should error on parse");
        let chain = err.chain().map(|e| e.to_string()).collect::<Vec<_>>().join("\n");
        assert!(chain.contains("verbose") || chain.contains("level"), "{chain}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn toml_accepts_known_levels() {
        for lvl in ["trace", "debug", "info", "warn", "error"] {
            let body = format!(
                r#"
[logging]
level = "{lvl}"
"#
            );
            let p = write_tmp(&format!("level-{lvl}.toml"), &body);
            let cfg = load(Some(&p), None).unwrap_or_else(|e| panic!("{lvl} parse: {e}"));
            cfg.validate().unwrap_or_else(|e| panic!("{lvl} validate: {e}"));
            fs::remove_file(&p).ok();
        }
    }

    #[test]
    fn validate_rejects_bad_hex_color_in_any_group() {
        for (name, cfg) in [
            (
                "window.default",
                Config {
                    ui: Ui {
                        theme: Theme {
                            window: ThemeWindow {
                                default: Some("not-a-color".into()),
                                ..Default::default()
                            },
                            ..Default::default()
                        },
                    },
                    ..Default::default()
                },
            ),
            (
                "surface.card.user.bg",
                Config {
                    ui: Ui {
                        theme: Theme {
                            surface: ThemeSurface {
                                card: SurfaceCard {
                                    user: Card {
                                        bg: Some("#xyz".into()),
                                    },
                                    ..Default::default()
                                },
                                ..Default::default()
                            },
                            ..Default::default()
                        },
                    },
                    ..Default::default()
                },
            ),
            (
                "accent.user",
                Config {
                    ui: Ui {
                        theme: Theme {
                            accent: ThemeAccent {
                                user: Some("#12345".into()),
                                ..Default::default()
                            },
                            ..Default::default()
                        },
                    },
                    ..Default::default()
                },
            ),
        ] {
            let err = cfg.validate().expect_err(&format!("{name} should reject"));
            assert!(
                err.to_string().contains(name),
                "error for {name} missing the field path: {err}"
            );
            assert!(
                err.to_string().contains("hex color"),
                "error for {name} missing 'hex color': {err}"
            );
        }
    }

    #[test]
    fn daemon_window_override_preserves_untouched_tokens() {
        let p = write_tmp(
            "window.toml",
            r#"
[daemon.window]
mode = "center"

[daemon.window.anchor]
edge = "left"

[daemon.window.center]
width = "70%"
"#,
        );
        let cfg = load(Some(&p), None).expect("load");

        // Overridden fields.
        assert_eq!(cfg.daemon.window.mode, Some(WindowMode::Center));
        assert_eq!(cfg.daemon.window.anchor.edge, Some(Edge::Left));
        assert_eq!(cfg.daemon.window.center.width, Some(Dimension::Percent(70)));

        // Untouched within the same subtree — fall through to defaults.
        assert_eq!(cfg.daemon.window.anchor.margin, Some(0));
        assert_eq!(cfg.daemon.window.anchor.width, Some(Dimension::Percent(40)));
        // Height is intentionally unset in defaults — signals full-height fill.
        assert_eq!(cfg.daemon.window.anchor.height, None);
        assert_eq!(cfg.daemon.window.center.height, Some(Dimension::Percent(50)));

        fs::remove_file(&p).ok();
    }

    #[test]
    fn dimension_parses_pixels_and_percent() {
        #[derive(Debug, Deserialize)]
        struct Holder {
            d: Dimension,
        }

        let pixels: Holder = toml::from_str("d = 480").unwrap();
        assert_eq!(pixels.d, Dimension::Pixels(480));

        let percent: Holder = toml::from_str("d = \"50%\"").unwrap();
        assert_eq!(percent.d, Dimension::Percent(50));

        // Non-percent string shape — rejected at parse time.
        let err = toml::from_str::<Holder>("d = \"50px\"").expect_err("should reject");
        assert!(err.to_string().contains("must end with '%'"), "{err}");

        // Interior whitespace between digits and '%' must reject — the `.trim()`
        // between `strip_suffix('%')` and `parse()` used to silently accept
        // `"50 %"`. Outer whitespace is still fine (serde/the surrounding
        // `trim()` handles it).
        let err2 = toml::from_str::<Holder>("d = \"50 %\"").expect_err("should reject interior whitespace");
        assert!(err2.to_string().contains("invalid percent"), "{err2}");
    }

    #[test]
    fn validate_rejects_oversized_percent_dimension() {
        let p = write_tmp(
            "bad-pct.toml",
            r#"
[daemon.window.center]
width = "200%"
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let err = cfg.validate().expect_err("should reject");
        let msg = err.to_string();
        assert!(msg.contains("daemon.window.center.width"), "{msg}");
        assert!(msg.contains("1..=100"), "{msg}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn validate_rejects_negative_anchor_margin() {
        let p = write_tmp(
            "bad-margin.toml",
            r#"
[daemon.window.anchor]
margin = -5
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let err = cfg.validate().expect_err("should reject");
        let msg = err.to_string();
        assert!(msg.contains("daemon.window.anchor.margin"), "{msg}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn validate_rejects_zero_pixel_dimension() {
        let cfg = Config {
            daemon: Daemon {
                window: Window {
                    center: CenterWindow {
                        width: Some(Dimension::Pixels(0)),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let err = cfg.validate().expect_err("should error");
        assert!(err.to_string().contains(">= 1"), "{err}");
    }

    /// Mirrors `defaults_populate_every_daemon_window_field` for the
    /// agents registry. If the seeded entries drift — wrong provider
    /// name, missing id, policy variant removed — this fires before
    /// the daemon starts panicking at runtime against a bad schema.
    #[test]
    fn defaults_populate_every_required_agent_field() {
        let cfg: Config = toml::from_str(DEFAULTS).expect("defaults must parse");

        assert_eq!(
            cfg.agents.agent.default.as_deref(),
            Some("claude-code"),
            "agent.default must be seeded to a concrete id"
        );

        let ids: Vec<&str> = cfg.agents.agents.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["claude-code", "codex", "opencode"],
            "defaults must seed the three built-in vendors"
        );

        for a in &cfg.agents.agents {
            assert!(a.command.is_some(), "agents[{}].command", a.id);
            assert!(!a.args.is_empty(), "agents[{}].args", a.id);
        }

        // Provider mapping per id.
        let by_id: std::collections::HashMap<&str, AgentProvider> =
            cfg.agents.agents.iter().map(|a| (a.id.as_str(), a.provider)).collect();
        assert_eq!(by_id["claude-code"], AgentProvider::AcpClaudeCode);
        assert_eq!(by_id["codex"], AgentProvider::AcpCodex);
        assert_eq!(by_id["opencode"], AgentProvider::AcpOpenCode);
    }

    #[test]
    fn user_agent_entry_overrides_default_by_id() {
        // Override claude-code's command; leave codex + opencode
        // untouched; add a new entry with a fresh id.
        let p = write_tmp(
            "agents.toml",
            r#"
[[agents]]
id = "claude-code"
provider = "acp-claude-code"
command = "my-claude"
args = ["--custom"]

[[agents]]
id = "my-local"
provider = "acp-codex"
command = "local-codex"
args = []
"#,
        );
        let cfg = load(Some(&p), None).expect("load");

        let ids: Vec<&str> = cfg.agents.agents.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["claude-code", "codex", "opencode", "my-local"],
            "overrides keep position, new ids append"
        );

        let cc = cfg.agents.agents.iter().find(|a| a.id == "claude-code").unwrap();
        assert_eq!(cc.command.as_deref(), Some("my-claude"));
        assert_eq!(cc.args, vec!["--custom".to_string()]);

        // Untouched defaults keep everything.
        let codex = cfg.agents.agents.iter().find(|a| a.id == "codex").unwrap();
        assert_eq!(codex.command.as_deref(), Some("bunx"));

        // Appended entry survived.
        let ml = cfg.agents.agents.iter().find(|a| a.id == "my-local").unwrap();
        assert_eq!(ml.provider, AgentProvider::AcpCodex);

        fs::remove_file(&p).ok();
    }

    #[test]
    fn validate_rejects_duplicate_agent_ids() {
        let p = write_tmp(
            "dup.toml",
            r#"
[[agents]]
id = "dupe"
provider = "acp-claude-code"
command = "a"

[[agents]]
id = "dupe"
provider = "acp-codex"
command = "b"
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let err = cfg.validate().expect_err("should reject");
        let msg = err.to_string();
        assert!(msg.contains("duplicate agent id 'dupe'"), "{msg}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn validate_rejects_unknown_agent_default() {
        let p = write_tmp(
            "bad-default.toml",
            r#"
[agent]
default = "does-not-exist"
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let err = cfg.validate().expect_err("should reject");
        let msg = err.to_string();
        // garde's report prefixes the field path: the cross-field
        // custom is attached to `AgentsConfig.agent`, which flattens
        // to `agents.agent` on `Config`.
        assert!(msg.contains("agents.agent"), "missing path: {msg}");
        assert!(msg.contains("default = 'does-not-exist'"), "missing detail: {msg}");
        assert!(msg.contains("Configured ids:"), "missing id list: {msg}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn user_override_of_agent_default_wins() {
        let p = write_tmp(
            "agent-default.toml",
            r#"
[agent]
default = "codex"
"#,
        );
        let cfg = load(Some(&p), None).expect("load");
        assert_eq!(cfg.agents.agent.default.as_deref(), Some("codex"));
        cfg.validate().expect("codex exists in defaults, so valid");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn agent_provider_round_trips_kebab_case() {
        // Spot-check each variant. Names match the TOML literals in
        // defaults.toml — a rename would require updating defaults
        // AND every user config out there.
        for (v, literal) in [
            (AgentProvider::AcpClaudeCode, "\"acp-claude-code\""),
            (AgentProvider::AcpCodex, "\"acp-codex\""),
            (AgentProvider::AcpOpenCode, "\"acp-opencode\""),
        ] {
            assert_eq!(serde_json::to_string(&v).unwrap(), literal);
            let back: AgentProvider = serde_json::from_str(literal).unwrap();
            assert_eq!(back, v);
        }
    }

    #[test]
    fn agent_without_model_parses() {
        let p = write_tmp(
            "no-model.toml",
            r##"
[[agents]]
id = "bare"
provider = "acp-claude-code"
command = "my-agent"
args = ["--flag"]
"##,
        );
        let cfg = load(Some(&p), None).expect("load");
        let bare = cfg.agents.agents.iter().find(|a| a.id == "bare").expect("bare entry");
        assert_eq!(bare.model, None, "model must be absent when not set in TOML");
        cfg.validate().expect("valid");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn validate_accepts_hex_with_alpha() {
        let cfg = Config {
            ui: Ui {
                theme: Theme {
                    window: ThemeWindow {
                        default: Some("#1e2127ff".into()),
                        edge: Some("#D3B051".into()),
                    },
                    ..Default::default()
                },
            },
            ..Default::default()
        };
        cfg.validate().expect("6- and 8-digit hex both accepted");
    }

    /// Defaults ship zero profiles and no `agent.default_profile` —
    /// profiles are user-supplied, the daemon falls back to the
    /// `[agent] default` agent when none is selected.
    #[test]
    fn defaults_seed_no_profiles() {
        let cfg: Config = toml::from_str(DEFAULTS).expect("defaults must parse");

        assert!(cfg.profiles.is_empty(), "defaults must not seed any profiles");
        assert!(
            cfg.agents.agent.default_profile.is_none(),
            "agent.default_profile must not be seeded"
        );

        cfg.validate().expect("empty profile set validates");
    }

    #[test]
    fn profile_references_missing_agent_fails() {
        let p = write_tmp(
            "missing-agent.toml",
            r#"
[[profiles]]
id = "ghost"
agent = "does-not-exist"
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let err = cfg.validate().expect_err("should reject");
        let msg = err.to_string();
        assert!(msg.contains("profile 'ghost'"), "{msg}");
        assert!(msg.contains("'does-not-exist'"), "{msg}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn profile_rejects_both_system_prompt_fields() {
        let p = write_tmp(
            "both-prompts.toml",
            r#"
[[profiles]]
id = "clash"
agent = "claude-code"
system_prompt = "inline"
system_prompt_file = "/tmp/whatever.md"
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let err = cfg.validate().expect_err("should reject");
        let msg = err.to_string();
        assert!(
            msg.contains("system_prompt") && msg.contains("system_prompt_file"),
            "{msg}"
        );
        assert!(msg.contains("pick one"), "{msg}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn profile_ids_unique() {
        let p = write_tmp(
            "dup-profiles.toml",
            r#"
[[profiles]]
id = "dupe"
agent = "claude-code"

[[profiles]]
id = "dupe"
agent = "codex"
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let err = cfg.validate().expect_err("should reject");
        let msg = err.to_string();
        assert!(msg.contains("duplicate profile id 'dupe'"), "{msg}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn default_profile_references_missing_profile_fails() {
        let p = write_tmp(
            "bad-default-profile.toml",
            r#"
[agent]
default_profile = "ghost-profile"
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let err = cfg.validate().expect_err("should reject");
        let msg = err.to_string();
        assert!(msg.contains("default_profile = 'ghost-profile'"), "{msg}");
        assert!(msg.contains("Configured ids:"), "{msg}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn profile_without_allowlists_defaults_to_empty_vecs() {
        let p = write_tmp(
            "allowlists-default.toml",
            r#"
[[profiles]]
id = "plain"
agent = "claude-code"
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let plain = cfg.profiles.iter().find(|p| p.id == "plain").expect("plain entry");
        assert!(plain.auto_accept_tools.is_empty());
        assert!(plain.auto_reject_tools.is_empty());
        cfg.validate().expect("empty allowlists validate");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn profile_parses_glob_allowlists() {
        let p = write_tmp(
            "allowlists.toml",
            r#"
[[profiles]]
id = "lax"
agent = "claude-code"
auto_accept_tools = ["Read", "Edit*"]
auto_reject_tools = ["Bash"]
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let lax = cfg.profiles.iter().find(|p| p.id == "lax").expect("lax entry");
        assert_eq!(lax.auto_accept_tools, vec!["Read".to_string(), "Edit*".to_string()]);
        assert_eq!(lax.auto_reject_tools, vec!["Bash".to_string()]);
        cfg.validate().expect("valid glob set");
        let (accept, reject) = lax.compile_tool_globs().expect("compiles");
        assert!(accept.is_match("Read"));
        assert!(accept.is_match("EditFile"));
        assert!(!accept.is_match("Bash"));
        assert!(reject.is_match("Bash"));
        fs::remove_file(&p).ok();
    }

    #[test]
    fn profile_rejects_empty_glob_pattern() {
        let p = write_tmp(
            "empty-glob.toml",
            r#"
[[profiles]]
id = "bad"
agent = "claude-code"
auto_accept_tools = [""]
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let err = cfg.validate().expect_err("should reject");
        let msg = err.to_string();
        assert!(msg.contains("profile 'bad'"), "{msg}");
        assert!(msg.contains("empty string"), "{msg}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn profile_rejects_invalid_glob_pattern() {
        let p = write_tmp(
            "bad-glob.toml",
            r#"
[[profiles]]
id = "busted"
agent = "claude-code"
auto_reject_tools = ["["]
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let err = cfg.validate().expect_err("should reject");
        let msg = err.to_string();
        assert!(msg.contains("profile 'busted'"), "{msg}");
        assert!(msg.contains("invalid tool glob"), "{msg}");
        fs::remove_file(&p).ok();
    }

    #[test]
    /// With no seeded profiles in `defaults.toml` the merged list is
    /// exactly what the user supplies, in file order. The keyed-list
    /// merge semantics are pinned separately by
    /// `user_agent_entry_overrides_default_by_id`; this test just
    /// confirms that user profiles flow through cleanly.
    fn user_profiles_flow_through_in_order() {
        let p = write_tmp(
            "user-profiles.toml",
            r#"
[[profiles]]
id = "strict"
agent = "opencode"
model = "custom-model"

[[profiles]]
id = "my-profile"
agent = "claude-code"
"#,
        );
        let cfg = load(Some(&p), None).expect("load");

        let ids: Vec<&str> = cfg.profiles.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, vec!["strict", "my-profile"], "user profiles appear in file order");

        let strict = cfg.profiles.iter().find(|p| p.id == "strict").unwrap();
        assert_eq!(strict.agent, "opencode");
        assert_eq!(strict.model.as_deref(), Some("custom-model"));
        assert!(strict.system_prompt.is_none());

        fs::remove_file(&p).ok();
    }
}

mod validations;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use garde::Validate;
use serde::{Deserialize, Serialize};

use crate::paths;
use validations::{validate_agent_default_id, validate_agents_ids};

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
    /// `[[agents]]` entries + the `[agent]` global section live at
    /// the TOML root; flattened onto `Config` so the user doesn't
    /// have to type `[agents.agents]`-style nested paths. The nested
    /// struct keeps the Rust-side code cohesive — `AgentsConfig` is
    /// what the ACP module reads off Tauri managed state.
    #[garde(dive)]
    #[serde(flatten)]
    pub agents: AgentsConfig,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct Daemon {
    #[garde(skip)]
    pub socket: Option<PathBuf>,
    #[garde(dive)]
    pub window: Window,
}

/// Surface behavior of the daemon's main window.
///
/// `mode = "anchor"` wraps the GTK window in a `zwlr_layer_shell_v1` surface
/// pinned to one edge; `mode = "center"` falls back to a regular top-level
/// sized as a fraction of the target monitor. The `layer = overlay` /
/// `keyboard_interactivity = on_demand` choices are intentionally not
/// configurable — see `CLAUDE.md` for why.
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

/// Per-edge anchor geometry. `width` and `height` accept either a pixel
/// integer or an `"N%"` string (resolved against the active monitor at
/// map-time). When `height` is unset the daemon pins the surface to
/// top + bottom + configured `edge` so it fills the monitor vertically —
/// the default overlay shape. Re-mapping on monitor swaps is handled by
/// the compositor restaging the layer surface.
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

/// Pixel literal or a "N%" string. `#[serde(untagged)]` lets TOML use either
/// an integer (`width = 480`) or a string (`width = "50%"`) at the same key.
/// A custom `Deserialize` on the `Percent` variant parses the `%` suffix.
///
/// Hand-implements `garde::Validate` so `#[garde(dive)]` on any
/// `Option<Dimension>` field picks it up automatically — no per-field
/// `#[garde(inner(custom(fn)))]` plumbing needed.
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

/// Hex colour string — `#RRGGBB` or `#RRGGBBAA`. `#[serde(transparent)]`
/// keeps the wire format identical to a bare string, so TOML writes
/// `default = "#1e2127"` and the `get_theme` Tauri command round-trips
/// a flat JSON string to the webview. Consumers that need the raw
/// string use `as_ref()` (`AsRef<str>` impl below) or `&color.0`.
///
/// The hex invariant is enforced by `impl Validate` — garde runs the
/// check whenever a theme field tagged `#[garde(dive)]` carries one.
/// `From<&str>` / `From<String>` exist for test ergonomics; they
/// accept any string, so tests can still construct invalid values
/// to prove `validate()` rejects them.
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
    /// Unknown levels fail at TOML parse time (serde rejects
    /// unrecognised enum variants), not at validate time. That's
    /// stricter than the old `Option<String>` + custom validator
    /// pair and encodes the closed set in the type.
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
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeFont {
    #[garde(inner(length(min = 1)))]
    pub family: Option<String>,
}

/// The outer container — everything intrinsic to the window frame. `default`
/// is the window's background fill; `edge` is the accent stripe on the
/// left edge.
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
    pub card: SurfaceCard,
    #[garde(dive)]
    pub compose: Option<HexColor>,
    #[garde(dive)]
    pub text: Option<HexColor>,
}

/// Cards carry messages — the palette splits them by speaker so user and
/// assistant cards can diverge in bg (and future accent, border, fg…)
/// without needing two disjoint config trees.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct SurfaceCard {
    #[garde(dive)]
    pub user: Card,
    #[garde(dive)]
    pub assistant: Card,
}

/// A single card's painted tokens. `bg` is the base paint; future fields
/// (accent stripe, border, text-on-card) slot in alongside without a
/// schema rewrite.
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
    pub dim: Option<HexColor>,
    #[garde(dive)]
    pub muted: Option<HexColor>,
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
    pub assistant: Option<HexColor>,
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
}

/// User-facing agent registry.
///
/// `agents` is an array of per-agent config blocks, each identified by
/// its `id` (`claude-code`, `codex`, `opencode`, …). The singleton
/// `[agent]` section holds global agent-scope config; today that's
/// only `agent.default`, but future generic settings (timeout, cwd
/// defaults, shared env overlay, …) slot in alongside without
/// another top-level key.
///
/// Merge semantics: user TOML entries with an existing `id` override
/// the compiled default fields for that id; entries with a new `id`
/// extend the registry. `[agent]` fields last-writer-win through the
/// generic `Merge` blanket on `Option`.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct AgentsConfig {
    /// Every configured agent. Validation dives into each entry and
    /// additionally asserts ids are unique.
    #[garde(dive)]
    #[garde(custom(validate_agents_ids))]
    pub agents: Vec<AgentConfig>,
    /// Global agent-scope config (`[agent]` in TOML). Kept singular
    /// (`agent`) to parallel the plural `[[agents]]` registry. The
    /// cross-field custom validator closes over `&self.agents` to
    /// assert `default` (when set) names a real entry.
    #[garde(dive)]
    #[garde(custom(validate_agent_default_id(&self.agents)))]
    pub agent: AgentDefaults,
}

/// Global agent-scope config. Today only `default` (the agent id to
/// use when neither `ctl submit` nor the webview specifies one);
/// future additions (timeout, cwd defaults, global env) land here.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct AgentDefaults {
    /// Id of the agent to use when none is addressed explicitly.
    /// Cross-field check against `agents[].id` runs on the parent
    /// `AgentsConfig.agent` field via
    /// `agent_default_references_id(&self.agents)`; this leaf
    /// itself has no per-value rules.
    #[garde(skip)]
    pub default: Option<String>,
}

/// A single configured agent. `id` is the user-facing handle;
/// `provider` selects the Rust vendor struct that encodes that
/// backend's quirks. `command` + `args` + `env` determine how the
/// daemon spawns the ACP server subprocess.
///
/// Note: there is intentionally no `permission_policy` field.
/// Vendors now ship their own plan / build / approval modes, and
/// client-side auto-accept / auto-reject rules will live on the
/// separate `PermissionController` (future issue) with per-tool
/// allow / reject lists rather than a three-way enum that
/// duplicates vendor behavior.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Validate)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    #[garde(length(min = 1))]
    pub id: String,
    #[garde(skip)]
    pub provider: AgentProvider,
    /// Missing → fall back to the provider's default command (each
    /// vendor struct supplies one).
    #[garde(inner(length(min = 1)))]
    pub command: Option<String>,
    #[garde(skip)]
    #[serde(default)]
    pub args: Vec<String>,
    /// Working directory for the agent subprocess. Missing →
    /// `std::env::current_dir()` at `new_session` time.
    #[garde(skip)]
    pub cwd: Option<PathBuf>,
    /// Additional environment variables forwarded to the child. Keys
    /// are inherited as-is; blank values are allowed.
    #[garde(skip)]
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

/// Closed enum — each variant maps to a Rust struct that encodes that
/// vendor's quirks (launch command, permission kinds, tool-content
/// shape). Adding a provider means one new struct + one new enum
/// variant + one new match arm in `acp::agents::match_provider_agent`.
///
/// Wire names are explicit because `rename_all = "kebab-case"` would
/// turn `AcpOpenCode` into `acp-open-code` — opencode's product name
/// is one word, and spending configuration on the hyphen placement
/// would be a papercut.
///
/// The shared `Acp` prefix is load-bearing (the additive naming rule
/// groups ACP providers apart from future `HttpAgent*` or
/// `LocalAgent*` siblings), so clippy's `enum_variant_names` lint is
/// silenced at the type level.
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

pub fn load(cli_path: Option<&Path>, profile: Option<&str>) -> Result<Config> {
    let mut layers: Vec<String> = vec![DEFAULTS.to_string()];

    match cli_path {
        Some(p) => {
            if !p.exists() {
                bail!("config file not found: {}", p.display());
            }
            layers.push(read_layer(p)?);
        }
        None => {
            let default = paths::config_file();
            if default.exists() {
                layers.push(read_layer(&default)?);
            }
        }
    }

    if let Some(name) = profile {
        let p = paths::profile_config_file(name);
        if !p.exists() {
            bail!("profile '{name}' not found at {}", p.display());
        }
        layers.push(read_layer(&p)?);
    }

    layers.iter().try_fold(Config::default(), |acc, body| {
        let layer: Config = toml::from_str(body).context("failed to parse TOML layer")?;
        Ok(acc.merge(layer))
    })
}

fn read_layer(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("failed to read config {}", path.display()))
}

/// Layer one config value on top of another.
///
/// Semantics: `self.merge(other)` returns a value where `other` wins
/// on every scalar leaf it populates (`Option::Some` beats `None` or
/// a prior `Some`), structs recurse field-by-field, and key-aware
/// collections like [`AgentsConfig`] override by key and append new
/// keys. The signature matches the fold direction in `load()`:
/// `acc.merge(layer)` — each successive config layer overrides the
/// accumulated one.
///
/// Every scalar leaf on the config tree is an `Option<T>`, so the
/// blanket impl below covers them all generically. Every struct in
/// the tree gets a trivial field-by-field impl; [`AgentsConfig`] is
/// the one exception and carries the keyed-list merge logic
/// directly.
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
        }
    }
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
        }
    }
}

impl Merge for ThemeFont {
    fn merge(self, other: Self) -> Self {
        Self {
            family: self.family.merge(other.family),
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
            dim: self.dim.merge(other.dim),
            muted: self.muted.merge(other.muted),
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
            assistant: self.assistant.merge(other.assistant),
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
        }
    }
}

/// Keyed-list merge. For each `id` in `self.agents`, the first
/// matching entry in `other.agents` wins if present (whole-entry
/// replace — no field-level merge, since "override provider keeps
/// old command" would be surprising). Entries in `other.agents`
/// with new ids append in order. Duplicate ids inside a single
/// layer survive (appended twice) so `validate_agents_ids` can
/// flag them. `[agent]` recurses field-by-field through its own
/// `Merge` impl (generic `Option<T>` blanket).
///
/// Per-field merging inside `AgentConfig` would be overkill:
/// `AgentConfig` has <10 leaves and is read as a whole unit by the
/// spawn flow, so override-in-place-by-id is the useful grain.
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
        }
    }
}

impl Config {
    /// Run the full validation chain: garde's derive-driven tree
    /// walk. Every predicate (field-scoped + cross-field) is wired
    /// into the derive — cross-field rules via higher-order custom
    /// validators that close over sibling references (see
    /// `agent_default_references_id` in `config::validations`). On
    /// failure wraps the garde report with an `anyhow!` so callers
    /// see a single readable error chain.
    pub fn validate(&self) -> Result<()> {
        <Self as Validate>::validate(self).map_err(|report| anyhow!("config is invalid:\n{report}"))
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

        assert!(t.font.family.is_some(), "font.family");

        for (n, v) in [("window.default", &t.window.default), ("window.edge", &t.window.edge)] {
            assert!(v.is_some(), "{n}");
        }

        for (n, v) in [
            ("surface.card.user.bg", &t.surface.card.user.bg),
            ("surface.card.assistant.bg", &t.surface.card.assistant.bg),
            ("surface.compose", &t.surface.compose),
            ("surface.text", &t.surface.text),
        ] {
            assert!(v.is_some(), "{n}");
        }

        for (n, v) in [
            ("fg.default", &t.fg.default),
            ("fg.dim", &t.fg.dim),
            ("fg.muted", &t.fg.muted),
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
            ("accent.assistant", &t.accent.assistant),
        ] {
            assert!(v.is_some(), "{n}");
        }

        for (n, v) in [
            ("state.idle", &t.state.idle),
            ("state.stream", &t.state.stream),
            ("state.pending", &t.state.pending),
            ("state.awaiting", &t.state.awaiting),
        ] {
            assert!(v.is_some(), "{n}");
        }
    }

    #[test]
    fn load_merges_cli_path_over_defaults() {
        let p = write_tmp("merge.toml", "[logging]\nlevel = \"debug\"\n");
        let cfg = load(Some(&p), None).expect("load");
        assert_eq!(cfg.logging.level, Some(crate::logging::LogLevel::Debug));
        fs::remove_file(&p).ok();
    }

    #[test]
    fn theme_override_preserves_untouched_tokens() {
        let p = write_tmp(
            "theme.toml",
            "[ui.theme.window]\ndefault = \"#101418\"\nedge = \"#ff00aa\"\n\n[ui.theme.border]\nfocus = \"#00ff00\"\n\n[ui.theme.surface.card.user]\nbg = \"#ff8800\"\n",
        );
        let cfg = load(Some(&p), None).expect("load");

        // Overridden.
        assert_eq!(cfg.ui.theme.window.default.as_deref(), Some("#101418"));
        assert_eq!(cfg.ui.theme.window.edge.as_deref(), Some("#ff00aa"));
        assert_eq!(cfg.ui.theme.border.focus.as_deref(), Some("#00ff00"));
        assert_eq!(cfg.ui.theme.surface.card.user.bg.as_deref(), Some("#ff8800"));

        // Untouched in the same groups still fall back to defaults.
        assert_eq!(cfg.ui.theme.border.default.as_deref(), Some("#4b5263"));
        assert_eq!(cfg.ui.theme.border.soft.as_deref(), Some("#2c333d"));
        assert_eq!(cfg.ui.theme.surface.card.assistant.bg.as_deref(), Some("#22282f"));
        assert_eq!(cfg.ui.theme.surface.compose.as_deref(), Some("#2c333d"));

        // Groups not mentioned at all still come from defaults.
        assert_eq!(cfg.ui.theme.fg.default.as_deref(), Some("#abb2bf"));
        assert_eq!(cfg.ui.theme.accent.default.as_deref(), Some("#abb2bf"));

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
        let p = write_tmp("bad-level.toml", "[logging]\nlevel = \"verbose\"\n");
        let err = load(Some(&p), None).expect_err("should error on parse");
        let chain = err.chain().map(|e| e.to_string()).collect::<Vec<_>>().join("\n");
        assert!(chain.contains("verbose") || chain.contains("level"), "{chain}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn toml_accepts_known_levels() {
        for lvl in ["trace", "debug", "info", "warn", "error"] {
            let p = write_tmp(&format!("level-{lvl}.toml"), &format!("[logging]\nlevel = \"{lvl}\"\n"));
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
            "[daemon.window]\nmode = \"center\"\n\n[daemon.window.anchor]\nedge = \"left\"\n\n[daemon.window.center]\nwidth = \"70%\"\n",
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
        let p = write_tmp("bad-pct.toml", "[daemon.window.center]\nwidth = \"200%\"\n");
        let cfg = load(Some(&p), None).expect("parses");
        let err = cfg.validate().expect_err("should reject");
        let msg = err.to_string();
        assert!(msg.contains("daemon.window.center.width"), "{msg}");
        assert!(msg.contains("1..=100"), "{msg}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn validate_rejects_negative_anchor_margin() {
        let p = write_tmp("bad-margin.toml", "[daemon.window.anchor]\nmargin = -5\n");
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
    fn defaults_populate_every_agent_field() {
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
            "[[agents]]\nid = \"claude-code\"\nprovider = \"acp-claude-code\"\ncommand = \"my-claude\"\nargs = [\"--custom\"]\n\n[[agents]]\nid = \"my-local\"\nprovider = \"acp-codex\"\ncommand = \"local-codex\"\nargs = []\n",
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
            "[[agents]]\nid = \"dupe\"\nprovider = \"acp-claude-code\"\ncommand = \"a\"\n\n[[agents]]\nid = \"dupe\"\nprovider = \"acp-codex\"\ncommand = \"b\"\n",
        );
        let cfg = load(Some(&p), None).expect("parses");
        let err = cfg.validate().expect_err("should reject");
        let msg = err.to_string();
        assert!(msg.contains("duplicate agent id 'dupe'"), "{msg}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn validate_rejects_unknown_agent_default() {
        let p = write_tmp("bad-default.toml", "[agent]\ndefault = \"does-not-exist\"\n");
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
        let p = write_tmp("agent-default.toml", "[agent]\ndefault = \"codex\"\n");
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
}

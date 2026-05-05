//! Formatter dispatch registry. Three-tier lookup:
//!
//! 1. **Per-(adapter, leading-token)** — adapters register here for
//!    their vendor-specific tools. Lookup key is the snake-cased
//!    first whitespace-delimited token of `wire_name`, so a single
//!    registration of `"Edit"` matches both `"Edit"` (claude-code-acp
//!    ≤0.31) and `"Edit /tmp/foo"` (≥0.32 prose-title shape) and
//!    codex's `"Edit a.rs, b.rs"` — same `edit` key for all three.
//!    The mcp__ prefix exception routes `mcp__server__leaf` titles
//!    to the literal key `"mcp"` regardless of leading token.
//! 2. **Per-(adapter, matcher)** — predicate-driven dispatch for
//!    tools whose title gives no stable signal (claude-code-acp's
//!    `switch_mode` emits `"Ready to code?"` / `"EnterPlanMode"`
//!    / etc.; the discriminator is rawInput shape — `plan` is a
//!    non-empty string). Matchers iterate in registration order;
//!    first `true` wins. Register via `register_adapter_match`.
//! 3. **Per-kind** — closed ACP-spec set (`read` / `edit` / `delete`
//!    / `move` / `search` / `execute` / `think` / `fetch` / `other`).
//!    The default tier; every adapter falls through here.
//!
//! Dispatch order:
//!
//! ```text
//! (adapter, leading_token_snake) exact
//!   → (adapter, "mcp") if wire_name.starts_with("mcp__")
//!   → (adapter, matcher) — first match wins
//!   → kind default
//!   → "other"
//! ```

use std::collections::HashMap;

use convert_case::{Case, Casing};

use crate::tools::formatter::types::FormattedToolCall;

/// Per-formatter input. Carries everything a formatter needs to
/// produce a `FormattedToolCall` from a single tool-call observation.
pub struct FormatterContext<'a> {
    /// Wire tool name from `tool_call.title` (the stable first-
    /// observed string, never the prose-overwritten title from later
    /// updates). Used both for adapter override dispatch and as the
    /// default formatter's title fallback.
    pub wire_name: &'a str,
    /// ACP `tool_call.kind` — the closed-set classification. Drives
    /// the default tier when adapter override misses.
    pub kind: &'a str,
    /// Agent's structured arg dict (`tool_call.rawInput`).
    pub raw_input: Option<&'a serde_json::Value>,
    /// Agent provider id (`acp-claude-code` / `acp-codex` /
    /// `acp-opencode` / `acp`). Required — the registry caller
    /// always knows which adapter emitted the call.
    pub adapter: &'a str,
    /// Output content blocks the agent attached.
    pub content: &'a [serde_json::Value],
    /// Wall-clock (epoch ms) of the first `tool_call` observation
    /// for this id. Captured by the per-instance ACP cache.
    pub started_at: u64,
    /// Wall-clock (epoch ms) of the first `tool_call_update` whose
    /// state transitioned to `Completed` / `Failed`. `None` while
    /// the call is mid-flight; per-vendor formatters that want a
    /// `Stat::Duration` only emit it once this is `Some`.
    pub completed_at: Option<u64>,
}

/// Trait every formatter implements.
pub trait ToolFormatter: Send + Sync {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall;
}

/// Predicate matched against a `FormatterContext` — used by the
/// matcher tier to dispatch on rawInput shape when wire_name +
/// leading-token can't discriminate.
pub type MatcherFn = Box<dyn Fn(&FormatterContext) -> bool + Send + Sync>;

/// Closed registry of formatters. Three lookup structures; dispatch
/// precedence is documented at the module level.
pub struct FormatterRegistry {
    /// Per-(adapter, snake'd-leading-token) overrides. The lookup key
    /// is `wire_name.split_whitespace().next().to_case(Snake)` so the
    /// same registration of `"Edit"` matches every shape vendors emit
    /// — `"Edit"` (claude-code ≤0.31), `"Edit /tmp/foo"` (≥0.32 prose),
    /// `"Edit a.rs, b.rs"` (codex's parsed-shell joins).
    overrides: HashMap<(String, String), Box<dyn ToolFormatter>>,
    /// Per-(adapter, predicate) matchers. Iterated in registration
    /// order; first matcher whose predicate returns `true` wins.
    matchers: Vec<(String, MatcherFn, Box<dyn ToolFormatter>)>,
    /// Per-kind defaults. Always contains an `"other"` entry by
    /// construction (`build_default_registry`).
    defaults: HashMap<String, Box<dyn ToolFormatter>>,
}

impl FormatterRegistry {
    /// New empty registry. Caller MUST register an `"other"` kind
    /// formatter before `dispatch` — the registry contract bottoms
    /// out there.
    pub fn new() -> Self {
        Self {
            overrides: HashMap::new(),
            matchers: Vec::new(),
            defaults: HashMap::new(),
        }
    }

    /// Register a per-kind default formatter. `kind` is the raw ACP
    /// kind string (`"read"` / `"edit"` / `"other"` / etc.).
    pub fn register_kind(&mut self, kind: &str, formatter: Box<dyn ToolFormatter>) {
        self.defaults.insert(kind.to_string(), formatter);
    }

    /// Register a per-adapter override. `wire_name` is the tool's
    /// identity verb (`"Edit"` / `"Bash"` / `"switch_mode"` / etc.) —
    /// we snake_case it at registration AND at dispatch lookup
    /// against the wire title's leading token, so a single call
    /// covers every prose variant the SDK emits.
    pub fn register_adapter(&mut self, adapter: &str, wire_name: &str, formatter: Box<dyn ToolFormatter>) {
        let key = wire_name.to_case(Case::Snake);
        self.overrides.insert((adapter.to_string(), key), formatter);
    }

    /// Register a matcher-driven formatter. The predicate is invoked
    /// with the live `FormatterContext` at dispatch time; the first
    /// matcher whose predicate returns `true` for an `adapter` match
    /// wins. Use this when the agent's `title` is variable prose
    /// (claude-code-acp's `switch_mode` emits `"Ready to code?"`
    /// among others) and the discriminating signal is the rawInput
    /// shape.
    pub fn register_adapter_match<F>(&mut self, adapter: &str, matcher: F, formatter: Box<dyn ToolFormatter>)
    where
        F: Fn(&FormatterContext) -> bool + Send + Sync + 'static,
    {
        self.matchers.push((adapter.to_string(), Box::new(matcher), formatter));
    }

    /// Pick a formatter for `ctx` and invoke it. See module docs for
    /// the precedence rules.
    pub fn dispatch(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let leading = ctx
            .wire_name
            .split_whitespace()
            .next()
            .unwrap_or(ctx.wire_name)
            .to_case(Case::Snake);

        // (1) per-(adapter, snake'd-leading-token)
        if let Some(f) = self.overrides.get(&(ctx.adapter.to_string(), leading)) {
            return f.format(ctx);
        }

        // (2) `mcp__server__leaf` → (adapter, "mcp") prefix exception
        if ctx.wire_name.starts_with("mcp__") {
            if let Some(f) = self.overrides.get(&(ctx.adapter.to_string(), "mcp".to_string())) {
                return f.format(ctx);
            }
        }

        // (3) per-(adapter, matcher) — rawInput-shape dispatch for
        // tools whose title is variable prose
        for (adapter, matcher, formatter) in &self.matchers {
            if adapter == ctx.adapter && matcher(ctx) {
                return formatter.format(ctx);
            }
        }

        // (4) kind default
        if let Some(f) = self.defaults.get(ctx.kind) {
            return f.format(ctx);
        }

        // (5) other fallback (registry contract: always populated)
        self.defaults
            .get("other")
            .expect("FormatterRegistry: 'other' default formatter missing — register_kind(\"other\", ...) before dispatch()")
            .format(ctx)
    }
}

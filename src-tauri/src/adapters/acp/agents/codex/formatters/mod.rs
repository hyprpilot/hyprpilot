//! codex-acp formatter overrides. codex builds tool-call titles
//! dynamically per event (`"Read foo.rs"`, `"Edit a.rs, b.rs"`,
//! `"Search query in path"`, …); the registry's leading-token
//! dispatch matches each on its first word. Source: codex-acp's
//! `src/thread.rs`.

pub mod approve;
pub mod edit;
pub mod exec;
pub mod guardian;
pub mod tool;
pub mod view;
pub mod web;

use crate::config::AgentProvider;
use crate::tools::formatter::registry::FormatterRegistry;

pub fn register_all(reg: &mut FormatterRegistry) {
    let adapter = AgentProvider::AcpCodex.wire_id();
    // Parsed-shell verbs — same `ExecCommandBeginEvent` rawInput.
    reg.register_adapter(adapter, "Read", Box::new(exec::ExecFormatter));
    reg.register_adapter(adapter, "List", Box::new(exec::ExecFormatter));
    reg.register_adapter(adapter, "Search", Box::new(exec::ExecFormatter));
    // ApplyPatch — `Edit ...`.
    reg.register_adapter(adapter, "Edit", Box::new(edit::EditFormatter));
    // ViewImage — `View Image <path>`. Leading token is `View`.
    reg.register_adapter(adapter, "View", Box::new(view::ViewFormatter));
    // WebSearch title evolves through several leading tokens
    // (`Searching the Web` → `Searching for: …` / `Opening: …` /
    // `Finding: …` / `Web search`); register all variants.
    let web = || Box::new(web::WebSearchFormatter) as Box<dyn crate::tools::formatter::registry::ToolFormatter>;
    reg.register_adapter(adapter, "Searching", web());
    reg.register_adapter(adapter, "Opening", web());
    reg.register_adapter(adapter, "Finding", web());
    reg.register_adapter(adapter, "Find", web());
    reg.register_adapter(adapter, "Web", web());
    // Plugin / MCP tool — `Tool: <tool>` / `Tool: <server>/<leaf>`.
    // `Tool:` snake-cases to `tool` (the colon drops); registering
    // `Tool` matches both `"Tool:"` and bare `"Tool"`.
    reg.register_adapter(adapter, "Tool", Box::new(tool::ToolFormatterCodex));
    // MCP elicitation approval — `Approve <tool>` / `Approve MCP tool call`.
    reg.register_adapter(adapter, "Approve", Box::new(approve::ApproveFormatter));
    // Guardian assessment — `Guardian Review`.
    reg.register_adapter(adapter, "Guardian", Box::new(guardian::GuardianFormatter));
}

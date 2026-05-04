//! Default per-kind formatters. The closed ACP-spec set
//! ([spec](https://agentclientprotocol.com/protocol/tool-calls#param-kind))
//! plus an `other` fallback. Bare-minimum content — these don't know
//! vendor-specific arg shapes; rich content lives in per-adapter
//! overrides registered out of `adapters/acp/agents/<vendor>/formatters/`.
//!
//! Each kind formatter reads from `ctx.wire_name` (the agent's
//! composed title) for the pill title and projects `ctx.raw_input`
//! to structured fields. `think` extracts `thought`/`text` since
//! that's the only arg shape standardised across vendors.

pub mod delete;
pub mod edit;
pub mod execute;
pub mod fetch;
pub mod r#move;
pub mod other;
pub mod read;
pub mod search;
pub mod think;

use crate::tools::formatter::registry::FormatterRegistry;

/// Register every default-kind formatter on the supplied registry.
pub fn register_all(reg: &mut FormatterRegistry) {
    reg.register_kind("read", Box::new(read::ReadFormatter));
    reg.register_kind("edit", Box::new(edit::EditFormatter));
    reg.register_kind("delete", Box::new(delete::DeleteFormatter));
    reg.register_kind("move", Box::new(r#move::MoveFormatter));
    reg.register_kind("search", Box::new(search::SearchFormatter));
    reg.register_kind("execute", Box::new(execute::ExecuteFormatter));
    reg.register_kind("think", Box::new(think::ThinkFormatter));
    reg.register_kind("fetch", Box::new(fetch::FetchFormatter));
    reg.register_kind("other", Box::new(other::OtherFormatter));
}

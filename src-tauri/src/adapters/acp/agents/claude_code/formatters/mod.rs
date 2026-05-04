//! claude-code-specific formatter overrides. Registered against the
//! shared formatter registry under the `acp-claude-code` adapter
//! id. Each module exposes `register(reg, adapter)` and lands its
//! formatter at `(adapter, <wire-name>)`.

pub mod bash;
pub mod edit;
pub mod glob;
pub mod grep;
pub mod kill_shell;
pub mod mcp;
pub mod multi_edit;
pub mod notebook_edit;
pub mod plan_exit;
pub mod read;
pub mod skill;
pub mod task;
pub mod terminal;
pub mod todo;
pub mod tool_search;
pub mod web_fetch;
pub mod web_search;
pub mod write;

use crate::tools::formatter::registry::FormatterRegistry;

pub const ADAPTER_ID: &str = "acp-claude-code";

pub fn register_all(reg: &mut FormatterRegistry) {
    bash::register(reg, ADAPTER_ID);
    edit::register(reg, ADAPTER_ID);
    glob::register(reg, ADAPTER_ID);
    grep::register(reg, ADAPTER_ID);
    kill_shell::register(reg, ADAPTER_ID);
    mcp::register(reg, ADAPTER_ID);
    multi_edit::register(reg, ADAPTER_ID);
    notebook_edit::register(reg, ADAPTER_ID);
    plan_exit::register(reg, ADAPTER_ID);
    read::register(reg, ADAPTER_ID);
    skill::register(reg, ADAPTER_ID);
    task::register(reg, ADAPTER_ID);
    terminal::register(reg, ADAPTER_ID);
    todo::register(reg, ADAPTER_ID);
    tool_search::register(reg, ADAPTER_ID);
    web_fetch::register(reg, ADAPTER_ID);
    web_search::register(reg, ADAPTER_ID);
    write::register(reg, ADAPTER_ID);
}

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

use crate::config::AgentProvider;
use crate::tools::formatter::registry::FormatterRegistry;

pub fn register_all(reg: &mut FormatterRegistry) {
    let adapter = AgentProvider::AcpClaudeCode.wire_id();
    bash::register(reg, adapter);
    edit::register(reg, adapter);
    glob::register(reg, adapter);
    grep::register(reg, adapter);
    kill_shell::register(reg, adapter);
    mcp::register(reg, adapter);
    multi_edit::register(reg, adapter);
    notebook_edit::register(reg, adapter);
    plan_exit::register(reg, adapter);
    read::register(reg, adapter);
    skill::register(reg, adapter);
    task::register(reg, adapter);
    terminal::register(reg, adapter);
    todo::register(reg, adapter);
    tool_search::register(reg, adapter);
    web_fetch::register(reg, adapter);
    web_search::register(reg, adapter);
    write::register(reg, adapter);
}

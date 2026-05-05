//! opencode-acp formatter overrides. opencode emits ACP `tool_call`
//! titles as the lowercase tool ID (`read` / `edit` / `bash` / …),
//! making the standard exact-match `(adapter, wire_name_snake)`
//! dispatch sufficient. MCP tools follow the
//! `<sanitized_server>_<sanitized_tool>` convention (single
//! underscore, NOT claude-code's double-underscore prefix), so the
//! generic `mcp__` prefix exception in the registry does NOT fire
//! for opencode — those land on the kind defaults.
//!
//! Source: opencode's `packages/opencode/src/acp/agent.ts` +
//! per-tool definitions under `packages/opencode/src/tool/`.

pub mod bash;
pub mod edit;
pub mod glob;
pub mod grep;
pub mod lsp;
pub mod patch;
pub mod read;
pub mod skill;
pub mod task;
pub mod todo;
pub mod webfetch;
pub mod websearch;
pub mod write;

use crate::config::AgentProvider;
use crate::tools::formatter::registry::FormatterRegistry;

pub fn register_all(reg: &mut FormatterRegistry) {
    let adapter = AgentProvider::AcpOpenCode.wire_id();
    reg.register_adapter(adapter, "read", Box::new(read::ReadFormatter));
    reg.register_adapter(adapter, "edit", Box::new(edit::EditFormatter));
    reg.register_adapter(adapter, "write", Box::new(write::WriteFormatter));
    reg.register_adapter(adapter, "bash", Box::new(bash::BashFormatter));
    reg.register_adapter(adapter, "grep", Box::new(grep::GrepFormatter));
    reg.register_adapter(adapter, "glob", Box::new(glob::GlobFormatter));
    reg.register_adapter(adapter, "webfetch", Box::new(webfetch::WebFetchFormatter));
    reg.register_adapter(adapter, "websearch", Box::new(websearch::WebSearchFormatter));
    reg.register_adapter(adapter, "task", Box::new(task::TaskFormatter));
    reg.register_adapter(adapter, "todowrite", Box::new(todo::TodoFormatter));
    reg.register_adapter(adapter, "skill", Box::new(skill::SkillFormatter));
    reg.register_adapter(adapter, "patch", Box::new(patch::PatchFormatter));
    reg.register_adapter(adapter, "lsp", Box::new(lsp::LspFormatter));
}

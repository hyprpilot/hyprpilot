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

use crate::tools::formatter::registry::FormatterRegistry;

pub const ADAPTER_ID: &str = "acp-opencode";

pub fn register_all(reg: &mut FormatterRegistry) {
    reg.register_adapter(ADAPTER_ID, "read", Box::new(read::ReadFormatter));
    reg.register_adapter(ADAPTER_ID, "edit", Box::new(edit::EditFormatter));
    reg.register_adapter(ADAPTER_ID, "write", Box::new(write::WriteFormatter));
    reg.register_adapter(ADAPTER_ID, "bash", Box::new(bash::BashFormatter));
    reg.register_adapter(ADAPTER_ID, "grep", Box::new(grep::GrepFormatter));
    reg.register_adapter(ADAPTER_ID, "glob", Box::new(glob::GlobFormatter));
    reg.register_adapter(ADAPTER_ID, "webfetch", Box::new(webfetch::WebFetchFormatter));
    reg.register_adapter(ADAPTER_ID, "websearch", Box::new(websearch::WebSearchFormatter));
    reg.register_adapter(ADAPTER_ID, "task", Box::new(task::TaskFormatter));
    reg.register_adapter(ADAPTER_ID, "todowrite", Box::new(todo::TodoFormatter));
    reg.register_adapter(ADAPTER_ID, "skill", Box::new(skill::SkillFormatter));
    reg.register_adapter(ADAPTER_ID, "patch", Box::new(patch::PatchFormatter));
    reg.register_adapter(ADAPTER_ID, "lsp", Box::new(lsp::LspFormatter));
}

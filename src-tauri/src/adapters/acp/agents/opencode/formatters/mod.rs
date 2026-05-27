//! opencode-acp formatter overrides. opencode emits ACP `tool_call`
//! titles as the lowercase tool ID (`read` / `edit` / `bash` / …),
//! making the standard exact-match `(adapter, wire_name_snake)`
//! dispatch sufficient. MCP tools follow the
//! `<sanitized_server>_<sanitized_tool>` convention (single
//! underscore, NOT claude-code's canonical MCP title), so they land
//! on the kind defaults until the adapter maps a structured MCP
//! identity for them.
//!
//! Source: opencode's `packages/opencode/src/acp/agent.ts` +
//! per-tool definitions under `packages/opencode/src/tool/`.

pub mod bash;
pub mod edit;
pub mod glob;
pub mod grep;
pub mod lsp;
pub mod mcp;
pub mod patch;
pub mod permission;
pub mod plan;
pub mod read;
pub mod repo;
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
    reg.register_adapter(adapter, "apply_patch", Box::new(patch::PatchFormatter));
    // Older local traces used `patch`; upstream opencode's current
    // tool id is `apply_patch`, but keep the alias harmlessly routed
    // through the same formatter.
    reg.register_adapter(adapter, "patch", Box::new(patch::PatchFormatter));
    reg.register_adapter(adapter, "lsp", Box::new(lsp::LspFormatter));
    reg.register_adapter(adapter, "mcp", Box::new(mcp::McpFormatter));
    reg.register_adapter(adapter, "repo_clone", Box::new(repo::RepoCloneFormatter));
    reg.register_adapter(adapter, "repo_overview", Box::new(repo::RepoOverviewFormatter));
    reg.register_adapter(adapter, "plan_exit", Box::new(plan::PlanExitFormatter));
    reg.register_adapter(
        adapter,
        "external_directory",
        Box::new(permission::ExternalDirectoryFormatter),
    );
    reg.register_adapter(
        adapter,
        "workflow_tool_approval",
        Box::new(permission::WorkflowApprovalFormatter),
    );
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::config::AgentProvider;
    use crate::tools::formatter::build_default_registry;
    use crate::tools::formatter::registry::FormatterContext;
    use crate::tools::ToolKind;

    #[test]
    fn apply_patch_uses_opencode_patch_formatter() {
        let registry = build_default_registry();
        let raw_input = json!({ "patchText": "*** Begin Patch\n*** Add File: a.txt\n+hello\n*** End Patch" });
        let ctx = FormatterContext {
            wire_name: "apply_patch",
            tool_kind: &ToolKind::Other,
            raw_input: Some(&raw_input),
            adapter: AgentProvider::AcpOpenCode.wire_id(),
            content: &[],
            started_at: 0,
            completed_at: None,
        };
        let formatted = registry.dispatch(&ctx);

        assert_eq!(formatted.title, "patch");
        assert!(formatted
            .description
            .as_deref()
            .unwrap_or_default()
            .contains("*** Begin Patch"));
    }
}

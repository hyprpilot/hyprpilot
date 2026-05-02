/**
 * Closed-set discriminator a tool-call formatter emits onto its
 * `ToolCallView`. One variant per distinct tool — sibling formatters
 * for the same family (e.g. `bash` + `bash_output`) can share a
 * variant when they render as one logical unit; otherwise each
 * formatter owns its own variant.
 *
 * Distinct from the ACP wire `tool_call.kind` (read|edit|execute|…) —
 * that lives unchanged on the wire and is consumed by adapter-side
 * code. `ToolType` is the UI's own classification.
 */
export enum ToolType {
  Bash = 'bash',
  KillShell = 'kill-shell',
  Terminal = 'terminal',
  Read = 'read',
  Write = 'write',
  Edit = 'edit',
  MultiEdit = 'multi-edit',
  NotebookEdit = 'notebook-edit',
  Grep = 'grep',
  Glob = 'glob',
  ToolSearch = 'tool-search',
  WebFetch = 'web-fetch',
  WebSearch = 'web-search',
  PlanExit = 'plan-exit',
  Todo = 'todo',
  Think = 'think',
  Skill = 'skill',
  Task = 'task',
  Mcp = 'mcp',
  Other = 'other'
}

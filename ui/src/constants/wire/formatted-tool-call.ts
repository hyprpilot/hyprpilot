/**
 * ACP `tool_call.kind` — the closed-set classification per
 * [the spec](https://agentclientprotocol.com/protocol/tool-calls#param-kind).
 * Drives the frontend's `(kind, adapter, wireName) → Presentation`
 * lookup table.
 */
export enum ToolKind {
  Read = 'read',
  Edit = 'edit',
  Delete = 'delete',
  Move = 'move',
  Search = 'search',
  Execute = 'execute',
  Think = 'think',
  Fetch = 'fetch',
  Other = 'other'
}

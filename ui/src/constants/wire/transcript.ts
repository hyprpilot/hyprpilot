/**
 * Closed-set discriminators on the transcript / terminal wire shapes.
 * Mirrors the Rust `adapters::transcript::*` enums.
 */

/**
 * Typed transcript item kind — discriminator for the `TranscriptItem`
 * enum. Switch arms on this give TS compile-time exhaustive checking
 * on the demuxer.
 */
export enum TranscriptItemKind {
  UserPrompt = 'user_prompt',
  UserText = 'user_text',
  AgentText = 'agent_text',
  AgentThought = 'agent_thought',
  AgentAttachment = 'agent_attachment',
  ToolCall = 'tool_call',
  ToolCallUpdate = 'tool_call_update',
  Plan = 'plan',
  PermissionRequest = 'permission_request',
  Unknown = 'unknown'
}

export enum ToolCallState {
  Pending = 'pending',
  Running = 'running',
  Completed = 'completed',
  Failed = 'failed'
}

/**
 * Live terminal-stream event kind. Mirrors
 * `adapters::InstanceEvent::Terminal` — the Rust runtime pushes one
 * `Output` per stdout / stderr chunk and a single `Exit` on close.
 */
export enum TerminalChunkKind {
  Output = 'output',
  Exit = 'exit'
}

export enum TerminalStream {
  Stdout = 'stdout',
  Stderr = 'stderr'
}

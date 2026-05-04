/**
 * Mirrors the Rust `AgentProvider` enum (closed set keyed by wire
 * name). The formatter dispatch consumes this to pick per-adapter
 * overrides; passing `undefined` falls back to the per-tool fallback
 * formatter.
 */
export enum AdapterId {
  ClaudeCode = 'acp-claude-code',
  Codex = 'acp-codex',
  OpenCode = 'acp-opencode',
  /// User-supplied ACP-speaking binary. No per-adapter formatter
  /// overrides — every tool lookup falls through to its fallback.
  Acp = 'acp'
}

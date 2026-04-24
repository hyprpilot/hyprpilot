import { Phase, PlanStatus, StreamKind, ToolKind, ToolState, type BreadcrumbCount, type GitStatus, type PlanItem, type ToolChipItem } from '@components'

/**
 * D5_Conversation — single user+assistant exchange with an active plan,
 * running thinking block, tool chips, and a closing assistant message
 * block. Data lifted verbatim from `wf-d5-fusion.jsx:484-535`.
 */

export const conversationFrame = {
  profile: 'captain',
  phase: Phase.Streaming,
  modeTag: 'ask',
  provider: 'claude',
  model: 'opus-4',
  title: 'refactor fs tools out of AcpClient',
  cwd: '~/dev/hyprpilot',
  gitStatus: { branch: 'main', ahead: 2, behind: 0 } satisfies GitStatus,
  counts: [
    { label: '+22 mcps', count: 0, color: '#b4b9c3' },
    { label: '+4 skills', count: 0, color: '#b4b9c3' }
  ] satisfies BreadcrumbCount[]
}

// User message verbatim from the JSX. The inline `~/dev/…` span is
// rendered as plain text here — primitive lacks an inline-code slot
// today; follow-up issue if the full inline treatment matters.
export const userPrompt = 'Can you reanalyze ~/dev/hyprpilot/src-tauri/src/daemon/mod.rs and come up with the refactor plan?'

export const assistantElapsed = '6.7s'

// Thinking StreamCard content — <pre> block body, `kind=thinking`, active.
export const thinkingText = `Re-reading daemon/mod.rs from lines 240–262 — this is where
AcpClient owns the fs primitives directly.

The refactor moves fs.read_text_file / fs.write_text_file into
a tools/fs.rs module that AcpClient *composes*, not owns. That
keeps the ACP adapter thin and the domain logic testable.

Checking for terminal plumbing too — same story.`

export const thinkingCard = {
  kind: StreamKind.Thinking,
  active: true,
  label: 'thinking',
  elapsed: '2.4s'
}

export const planningCard = {
  kind: StreamKind.Planning,
  active: true,
  label: 'planning',
  elapsed: '0.9s'
}

export const planItems: PlanItem[] = [
  { status: PlanStatus.Completed, text: 'extract tools/fs.rs with read_text_file + write_text_file' },
  { status: PlanStatus.InProgress, text: 'extract tools/terminal.rs for ACP terminal plumbing' },
  { status: PlanStatus.Pending, text: 'AcpClient holds &Tools, delegates through' },
  { status: PlanStatus.Pending, text: 'update tests/fs_tools.rs to hit tools/* directly' },
  { status: PlanStatus.Pending, text: 'run cargo check + cargo test' }
]

export const tools: ToolChipItem[] = [
  { label: 'R', arg: 'daemon/mod.rs', detail: 'lines 240–262', stat: '640 lines', state: ToolState.Done, kind: ToolKind.Read },
  { label: '/', arg: 'AcpClient', detail: 'usages across crate', stat: '14 matches', state: ToolState.Done, kind: ToolKind.Search },
  { label: 'R', arg: 'config/defaults.toml', detail: 'check mcps block', stat: '42 lines', state: ToolState.Done, kind: ToolKind.Read },
  { label: '$', arg: 'cargo check', detail: 'workspace', stat: '0.8s', state: ToolState.Done, kind: ToolKind.Bash },
  { label: '⇲', arg: 'tools/fs.rs', detail: 'extract read/write', stat: '+23 / −11', state: ToolState.Running, kind: ToolKind.Write }
]

// Closing text block (rendered as a card on the assistant side).
export const assistantClosing = 'Agree — the fs + terminal code is domain logic wired through ACP. AcpClient should be a thin adapter, not the owner of these primitives.'

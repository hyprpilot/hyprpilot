import { Phase, StreamKind, ToolState, type BreadcrumbCount, type PlanItem, type ToolChipItem } from '@components'
import { PlanStatus } from '@components'

/**
 * D5_ToolCalls — single assistant turn with a done thinking summary,
 * active planning checklist, small-tool chips, and a running Bash card
 * streaming test output. Data verbatim from `wf-d5-fusion.jsx:539-611`.
 */

export const toolCallsFrame = {
  profile: 'captain',
  phase: Phase.Working,
  modeTag: 'build',
  provider: 'claude',
  model: 'opus-4',
  title: 'run the test suite, fix the flaky one',
  cwd: '~/dev/hyprpilot',
  counts: [
    { label: '+22 mcps', count: 0, color: '#b4b9c3' },
    { label: '+4 skills', count: 0, color: '#b4b9c3' }
  ] satisfies BreadcrumbCount[]
}

export const thinkingCard = {
  kind: StreamKind.Thinking,
  active: false,
  label: 'thought',
  elapsed: '3.1s',
  summary: 'reasoned through the failing test — tokio runtime races with the ACP stream on drop'
}

export const planningCard = {
  kind: StreamKind.Planning,
  active: true,
  label: 'planning',
  elapsed: '1.2s'
}

export const planItems: PlanItem[] = [
  { status: PlanStatus.Completed, text: 'read tools/fs.rs + tests/fs_tools.rs for the current shape' },
  { status: PlanStatus.Completed, text: 'run the flaky test in isolation 20x to confirm reproduction' },
  { status: PlanStatus.InProgress, text: 'patch the runtime-drop ordering in AcpClient::shutdown' },
  { status: PlanStatus.Pending, text: 're-run the full suite' },
  { status: PlanStatus.Pending, text: 'if green, write the regression test and commit' }
]

// Small tools — packed as an inline 2-col grid by ToolChips.
export const smallTools: ToolChipItem[] = [
  { label: 'Grep', arg: 'flaky', detail: 'across tests/', stat: '3 matches', state: ToolState.Done },
  { label: 'Read', arg: 'tools/fs.rs', detail: 'confirm extracted module', stat: '118 lines', state: ToolState.Done }
]

// Big tools rendered as standalone rows. JSX hand-rolls these rather
// than funneling through D5ToolChips; the Vue story mirrors by passing
// them to ToolRowBig one by one so the fixtures stay inspectable.
export const bashDone: ToolChipItem = {
  label: 'Bash',
  arg: 'cargo test --package hyprcaptain tools::fs',
  stat: '2.8s',
  state: ToolState.Done
}

export const bashRunning: ToolChipItem = {
  label: 'Bash',
  arg: 'cargo test --package hyprcaptain daemon::acp',
  stat: 'running · 1.4s',
  state: ToolState.Running
}

export const writeDone: ToolChipItem = {
  label: 'Write',
  arg: 'tools/fs.rs',
  // Rendered as colored +green / −red diff in the JSX — see writeDoneDiff
  // for the paired primary stats (used by the story to render the
  // colored diff inline).
  state: ToolState.Done
}

export const writeDoneDiff = { added: 23, removed: 11 }

// Running bash streams the real cargo output — verbatim from the JSX
// except the trailing caret, which the story renders as its own pulse
// element.
export const runningStdout = `   Compiling hyprcaptain v0.7.0 (~/dev/hyprpilot/src-tauri)
    Finished test [unoptimized + debuginfo] target(s) in 4.31s
     Running unittests src/lib.rs

running 12 tests
test daemon::acp::tests::handshake_v011 ... ok
test daemon::acp::tests::resume_from_log ... ok
test daemon::acp::tests::tool_search ...     ok
test daemon::acp::tests::fs_read_text_file ..`

// Legacy export name kept for backward-compat with `stories.test.ts`
// counting total tool chips.
export const tools: ToolChipItem[] = [...smallTools, bashDone, bashRunning, writeDone]

export const terminal = {
  command: bashRunning.arg ?? '',
  cwd: '~/dev/hyprpilot',
  stdout: runningStdout,
  running: true
}

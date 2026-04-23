import { Phase, StreamKind, ToolState, type BreadcrumbCount, type QueuedMessage, type ToolChipItem } from '@components'

/**
 * D5_Queue — session resumed from NDJSON log (note the `resumed` pill +
 * ok toast), one completed assistant turn with a single Bash tool chip
 * + thought summary, then a 4-message FIFO queue below. Data verbatim
 * from `wf-d5-fusion.jsx:870-914`.
 */

export const queueFrame = {
  profile: 'opencode',
  phase: Phase.Pending,
  modeTag: 'build',
  provider: 'moonshot',
  model: 'kimi-k2.6',
  title: 'curl google permission test',
  cwd: '~/dev/hyprpilot',
  counts: [
    { label: '+22 mcps', count: 0, color: '#b4b9c3' },
    { label: '+4 skills', count: 0, color: '#b4b9c3' },
    { label: '↻ resumed', count: 0, color: '#7fcf8a' }
  ] satisfies BreadcrumbCount[]
}

export const toast = {
  tone: 'ok' as const,
  message: 'session resumed from NDJSON log · 5 turns replayed byte-identical'
}

export const bashDone: ToolChipItem = {
  label: 'Bash',
  arg: 'curl -sI https://google.com',
  state: ToolState.Done
}

export const thinkingCard = {
  kind: StreamKind.Thinking,
  active: false,
  label: 'thought',
  elapsed: '0.6s',
  summary: 'works. outbound HTTPS allowed. 301 → www.google.com'
}

export const messages: QueuedMessage[] = [
  { id: 'q1', text: 'fix the flaky test in tools/fs.rs' },
  { id: 'q2', text: 'regenerate docs for acp/resolve' },
  { id: 'q3', text: 'review the leftover warnings from cargo check' },
  { id: 'q4', text: 'commit with message "k-240: extract tools/fs + terminal"' }
]

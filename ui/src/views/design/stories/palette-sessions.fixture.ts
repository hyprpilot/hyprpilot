import { Phase, type PaletteRowItem } from '@components'

/**
 * D5_Sessions — palette with a preview pane. Data verbatim from
 * `wf-d5-fusion.jsx:1012-1017`. Each row's `label` is the profile name,
 * `hint` is the session title, `right` is the time since last turn.
 *
 * chat1.md reviewer decision — sessions preview splits meta into
 * per-row pills rather than a single right-hand column; we keep the
 * primitive's right slot as a simple time stamp, matching the JSX.
 * The full preview breakout lives on the right-side pane.
 */

export enum SessionState {
  Streaming = 'streaming',
  Awaiting = 'awaiting',
  Idle = 'idle',
  Paused = 'paused'
}

export interface SessionRow extends PaletteRowItem {
  t: string
  state: SessionState
  profile: string
  meta: string
  title: string
  turns: number
  live: boolean
}

function phaseFor(state: SessionState): Phase {
  switch (state) {
    case SessionState.Streaming:
      return Phase.Streaming
    case SessionState.Awaiting:
      return Phase.Awaiting
    case SessionState.Idle:
      return Phase.Idle
    case SessionState.Paused:
    default:
      return Phase.Pending
  }
}

export const sessionRows: SessionRow[] = [
  {
    id: 's1',
    t: 'now',
    state: SessionState.Streaming,
    profile: 'captain',
    meta: 'ask · opus-4',
    title: 'refactor fs tools out of AcpClient',
    turns: 24,
    live: true,
    label: 'captain',
    hint: 'refactor fs tools out of AcpClient',
    right: 'now'
  },
  {
    id: 's2',
    t: '2m',
    state: SessionState.Awaiting,
    profile: 'opencode',
    meta: 'build · kimi-k2.6',
    title: 'curl google permission test',
    turns: 5,
    live: true,
    label: 'opencode',
    hint: 'curl google permission test',
    right: '2m'
  },
  {
    id: 's3',
    t: '8m',
    state: SessionState.Idle,
    profile: 'captain',
    meta: 'plan · opus-4',
    title: 'review MR !15 K-240 ACP runtime',
    turns: 47,
    live: true,
    label: 'captain',
    hint: 'review MR !15 K-240 ACP runtime',
    right: '8m'
  },
  {
    id: 's4',
    t: 'yday',
    state: SessionState.Paused,
    profile: 'captain',
    meta: 'plan · opus-4',
    title: 'profiles config draft',
    turns: 18,
    live: false,
    label: 'captain',
    hint: 'profiles config draft',
    right: 'yday'
  },
  {
    id: 's5',
    t: '2d',
    state: SessionState.Paused,
    profile: 'scratch',
    meta: 'ask · gpt-5',
    title: 'daemon.rs line 257 context',
    turns: 9,
    live: false,
    label: 'scratch',
    hint: 'daemon.rs line 257 context',
    right: '2d'
  }
]

export const selectedId = 's1'

export const resultCount = '3 live · 2 resumable'

export function phaseForState(state: SessionState): Phase {
  return phaseFor(state)
}

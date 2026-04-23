import { Phase, type BreadcrumbCount, type LiveSession } from '@components'

/**
 * D5_Idle — three live sessions centered on screen, `hyprpilot` wordmark
 * + `LFG.` tagline + four keyboard hints above. Data lifted verbatim
 * from `wf-d5-fusion.jsx:427-431`; cwds adjusted from `~/dev/hyprcaptain`
 * to `~/dev/hyprpilot` to match the repo name.
 */

export const idleFrame = {
  profile: 'captain',
  phase: Phase.Idle,
  modeTag: 'plan',
  provider: 'claude',
  model: 'opus-4',
  title: 'new session',
  cwd: '~/dev/hyprpilot',
  counts: [
    { label: '+22 mcps', count: 0, color: '#b4b9c3' },
    { label: '+4 skills', count: 0, color: '#b4b9c3' }
  ] satisfies BreadcrumbCount[]
}

export const liveSessions: LiveSession[] = [
  {
    id: 's1',
    title: 'refactor fs tools out of AcpClient',
    cwd: '~/dev/hyprpilot',
    adapter: 'opus-4',
    doing: 'writing tools/fs.rs',
    phase: Phase.Streaming
  },
  {
    id: 's2',
    title: 'curl google permission test',
    cwd: '~/dev/hyprpilot',
    adapter: 'kimi-k2.6',
    doing: 'permission',
    phase: Phase.Awaiting
  },
  {
    id: 's3',
    title: 'update changelog for 0.7',
    cwd: '~/dev/notes',
    adapter: 'opus-4',
    doing: '4m',
    phase: Phase.Idle
  }
]

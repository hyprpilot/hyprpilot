import { Phase, type BreadcrumbCount, type PermissionPrompt } from '@components'

/**
 * D5_Permission — three queued permission requests stacked under a
 * "3 pending" warn banner. Oldest (top) carries `allow once` / `deny`
 * buttons; the other two render their commands only. Data verbatim
 * from `wf-d5-fusion.jsx:619-623`.
 *
 * design-skip: the bundle mapped five option ids (allow-once /
 * allow-session / allow-always / deny-once / deny-always); chat1.md
 * reduced to two (`allow once` / `deny`) and that's what the port
 * carries. `kind` is kept as a typed prop but rendered as empty — the
 * JSX just shows `Bash` / `Write` as the tool name, no kind label.
 */

export const permissionFrame = {
  profile: 'opencode',
  phase: Phase.Awaiting,
  modeTag: 'build',
  provider: 'moonshot',
  model: 'kimi-k2.6',
  title: 'curl google permission test',
  cwd: '~/dev/hyprpilot',
  counts: [
    { label: '+22 mcps', count: 0, color: '#b4b9c3' },
    { label: '+4 skills', count: 0, color: '#b4b9c3' }
  ] satisfies BreadcrumbCount[]
}

export const prompts: PermissionPrompt[] = [
  { id: 'p1', tool: 'Bash', kind: '', args: 'curl -sI https://google.com | head -5' },
  { id: 'p2', tool: 'Write', kind: '', args: 'src-tauri/src/tools/fs.rs · +23 / −11', queued: true },
  { id: 'p3', tool: 'Bash', kind: '', args: 'git push origin k-240-extract-tools', queued: true }
]

export const userPrompt = "can you try to curl google, i'm testing the permissions"

export const assistantSummary = `no "curl *" rule yet — prompting you.`

import type { PaletteRowItem } from '@components'

/**
 * D5_Pickers — single multi-select palette (skills). Rows carry two
 * independent flags per JSX: `on` = currently enabled (filled checkbox
 * `☑`); `sel` = the fuzzy cursor position (yellow accent border). Two
 * rows are flagged `sel` in the bundle so that "multi-select" reads
 * unambiguously. Data verbatim from `wf-d5-fusion.jsx:947-955`.
 *
 * `PaletteRowItem` carries `selected` (cursor) — we synthesize `icon` off
 * the `enabled` state and pass `right='on'` when enabled so the row
 * keeps the same one-glance shape as the JSX.
 */

export interface SkillRow extends PaletteRowItem {
  enabled: boolean
}

export const skillRows: SkillRow[] = [
  { id: 'tdd', enabled: true, label: 'tdd', hint: 'test-first rust · red→green→refactor', icon: ['fas', 'square-check'], right: 'on' },
  { id: 'review', enabled: true, label: 'review', hint: 'MR rubric · line-by-line', icon: ['fas', 'square-check'], right: 'on' },
  { id: 'rust-api', enabled: false, label: 'rust-api', hint: 'API design · error types + traits', icon: ['far', 'square'] },
  { id: 'python', enabled: false, label: 'python', hint: 'style + types · black + mypy', icon: ['far', 'square'] },
  { id: 'docs', enabled: false, label: 'docs', hint: 'markdown notes · changelog', icon: ['far', 'square'] },
  { id: 'commit-msg', enabled: true, label: 'commit-msg', hint: 'conventional commits', icon: ['fas', 'square-check'], right: 'on' },
  { id: 'sql', enabled: false, label: 'sql', hint: 'postgres conventions', icon: ['far', 'square'] },
  { id: 'security', enabled: false, label: 'security', hint: 'threat model checklist', icon: ['far', 'square'] }
]

// Two rows share `selected` on the JSX so the multi-select shape
// reads at a glance — `tdd` + `commit-msg`.
export const selectedIds = new Set<string>(['tdd', 'commit-msg'])

export const resultCount = `${skillRows.filter((s) => s.enabled).length} / ${skillRows.length} enabled`

// Legacy exports kept to match `stories.test.ts` imports. Multi-select
// picker is the only pickers shape the bundle carries today; the other
// columns are not in scope.
export const agentRows: PaletteRowItem[] = []
export const profileRows: PaletteRowItem[] = []

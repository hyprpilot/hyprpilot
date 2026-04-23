import type { PaletteRowItem } from '@components'

/**
 * D5_Modes — single-select palette example. Rows verbatim from
 * `wf-d5-fusion.jsx:919-924`, including the `danger` flag on `yolo`.
 * "DANGER" label was explicitly dropped per chat1.md: "we do not now
 * nothing that is different for us it is all the same when we are
 * displaying".
 */
export const modeRows: PaletteRowItem[] = [
  { id: 'plan', icon: ['fas', 'circle'], label: 'plan', hint: 'read-only. think + propose, never mutate.', right: 'CURRENT' },
  { id: 'ask', icon: ['far', 'circle'], label: 'ask', hint: 'answer questions. read only, no writes, no bash.' },
  { id: 'build', icon: ['far', 'circle'], label: 'build', hint: 'full access. read, write, bash, mcps.' },
  { id: 'yolo', icon: ['far', 'circle'], label: 'yolo', hint: 'no permission prompts. auto-allow everything.', danger: true }
]

export const currentId = 'plan'

export const resultCount = `${modeRows.length} modes`

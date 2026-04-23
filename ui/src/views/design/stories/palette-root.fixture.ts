import type { PaletteRowItem } from '@components'

/**
 * D5_Palette — root command palette. Labels + descriptions verbatim
 * from `wf-d5-fusion.jsx:713-725` with three scope cuts per K-250:
 *   - "Trust store" leaf dropped (review: "nobody needs to see the logs").
 *   - "Wire log" leaf dropped (same reason — NDJSON inspector is out of scope).
 *   - "References" leaf dropped (review: "no references").
 * `skills` leaf retained. No `/slash` labels on rows — review decision
 * ("we do not have slash bullshit just fuzzy search").
 *
 * `Profiles` is the highlighted row (JSX `hl === true`), representing
 * the currently-selected item in the fuzzy list.
 */
export const rootRows: PaletteRowItem[] = [
  { id: 'profiles', label: 'Profiles', hint: 'switch agent profile — primary multiplex unit' },
  { id: 'sessions', label: 'Sessions', hint: '3 live · restore or start fresh' },
  { id: 'models', label: 'Models', hint: 'override the active model for this profile' },
  { id: 'modes', label: 'Modes', hint: 'plan · ask · build · yolo' },
  { id: 'commands', label: 'Commands', hint: 'run advertised slash commands' },
  { id: 'cwd', label: 'CWD', hint: 'browse + pick working directory' },
  { id: 'skills', label: 'Skills', hint: 'attach skills as resources on next turn' },
  { id: 'mcps', label: 'MCPs', hint: 'toggle MCP servers for this session' }
]

export const selectedId = 'profiles'

export const paletteQuery = 'profil'

export const resultCount = `${rootRows.length} results`

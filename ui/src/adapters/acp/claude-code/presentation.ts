/**
 * claude-code-acp's per-tool presentation overrides. Mirrors Rust's
 * `src-tauri/src/adapters/acp/agents/claude_code/formatters/`.
 *
 * Keys are snake_case wire names (matches the daemon's
 * `wire_name.to_case(Case::Snake)` lookup so frontend resolution
 * stays in lockstep). The literal `"mcp"` key is the dispatch
 * target for every dynamic `mcp__server__leaf` tool — handled
 * by the central `presentationFor()` prefix exception.
 */

import {
  faBookOpen,
  faClipboardList,
  faListCheck,
  faMagnifyingGlass,
  faMagnifyingGlassChart,
  faMagnifyingGlassPlus,
  faPen,
  faPenToSquare,
  faPlug,
  faPuzzlePiece,
  faSkull,
  faStarOfLife,
  faTerminal,
  faUserGear
} from '@fortawesome/free-solid-svg-icons'

import { PermissionUi, PillKind } from '@constants/ui'
import type { Presentation } from '@lib/tools/presentation'

const row = (icon: Presentation['icon']): Presentation => ({
  icon,
  pill: PillKind.Default,
  permissionUi: PermissionUi.Row
})

const modal = (icon: Presentation['icon']): Presentation => ({
  icon,
  pill: PillKind.Default,
  permissionUi: PermissionUi.Modal
})

export const claudeCodeOverrides: Record<string, Presentation> = {
  bash: row(faTerminal),
  bash_output: row(faTerminal),
  kill_shell: row(faSkull),
  terminal: row(faTerminal),
  read: row(faPenToSquare),
  write: modal(faPenToSquare),
  edit: modal(faPen),
  multi_edit: modal(faPen),
  notebook_edit: modal(faBookOpen),
  grep: row(faMagnifyingGlass),
  glob: row(faStarOfLife),
  tool_search: row(faMagnifyingGlassChart),
  web_fetch: row(faPlug),
  web_search: row(faMagnifyingGlassPlus),
  exit_plan_mode: modal(faClipboardList),
  // claude-code-acp ≥0.32 renamed the tool to `switch_mode`; same
  // modal-permission shape as `ExitPlanMode`.
  switch_mode: modal(faClipboardList),
  todo_write: row(faListCheck),
  skill: row(faPuzzlePiece),
  task: row(faUserGear),
  // Dispatch target for every dynamic `mcp__<server>__<leaf>` name —
  // central `presentationFor()` routes mcp__-prefixed wire names here.
  mcp: row(faPlug)
}

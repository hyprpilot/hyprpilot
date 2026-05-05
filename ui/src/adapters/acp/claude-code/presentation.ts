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

import { PermissionUi } from '@constants/ui'
import type { Presentation } from '@lib/tools/presentation'

const Row = PermissionUi.Row
const Modal = PermissionUi.Modal

export const claudeCodeOverrides: Record<string, Presentation> = {
  bash: { icon: faTerminal, permissionUi: Row },
  bash_output: { icon: faTerminal, permissionUi: Row },
  kill_shell: { icon: faSkull, permissionUi: Row },
  terminal: { icon: faTerminal, permissionUi: Row },
  read: { icon: faPenToSquare, permissionUi: Row },
  write: { icon: faPenToSquare, permissionUi: Modal },
  edit: { icon: faPen, permissionUi: Modal },
  multi_edit: { icon: faPen, permissionUi: Modal },
  notebook_edit: { icon: faBookOpen, permissionUi: Modal },
  grep: { icon: faMagnifyingGlass, permissionUi: Row },
  glob: { icon: faStarOfLife, permissionUi: Row },
  tool_search: { icon: faMagnifyingGlassChart, permissionUi: Row },
  web_fetch: { icon: faPlug, permissionUi: Row },
  web_search: { icon: faMagnifyingGlassPlus, permissionUi: Row },
  exit_plan_mode: { icon: faClipboardList, permissionUi: Modal },
  // claude-code-acp ≥0.32 renamed the tool to `switch_mode`; same
  // modal-permission shape as `ExitPlanMode`.
  switch_mode: { icon: faClipboardList, permissionUi: Modal },
  todo_write: { icon: faListCheck, permissionUi: Row },
  skill: { icon: faPuzzlePiece, permissionUi: Row },
  task: { icon: faUserGear, permissionUi: Row },
  // Dispatch target for every dynamic `mcp__<server>__<leaf>` name —
  // central `presentationFor()` routes mcp__-prefixed wire names here.
  mcp: { icon: faPlug, permissionUi: Row }
}

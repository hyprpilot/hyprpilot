/**
 * Tool-call presentation lookup. The daemon emits *content*; this
 * module layers *chrome* — icon, pill style, permission-flow surface.
 * Resolution mirrors the Rust dispatcher in
 * `src-tauri/src/tools/formatter/registry.rs`:
 *
 * ```text
 * (adapter, wire_name_snake) exact
 *   → (adapter, "mcp") if wire_name.startsWith("mcp__")
 *   → kind default
 *   → "other"
 * ```
 *
 * Per-vendor overrides live in `@adapters/acp/<vendor>/presentation.ts`.
 * Only the icon resolution is per-frontend; a Neovim plugin would
 * carry its own presentation map onto its own icon system (ASCII
 * glyphs, nerd-font codepoints).
 */

import type { IconDefinition } from '@fortawesome/fontawesome-svg-core'
import {
  faBrain,
  faFileLines,
  faGlobe,
  faMagnifyingGlass,
  faPen,
  faPenToSquare,
  faPlug,
  faTerminal,
  faTrash
} from '@fortawesome/free-solid-svg-icons'

import { acpOverrides } from '@adapters/acp/acp/presentation'
import { claudeCodeOverrides } from '@adapters/acp/claude-code/presentation'
import { codexOverrides } from '@adapters/acp/codex/presentation'
import { opencodeOverrides } from '@adapters/acp/opencode/presentation'
import { AdapterId, PermissionUi, ToolKind } from '@constants/ui'

export interface Presentation {
  icon: IconDefinition
  permissionUi: PermissionUi
}

/// Default presentation per ACP `tool_call.kind`. Bare-minimum tone +
/// icon assignments — adapters extend with richer per-wireName
/// overrides.
const kindDefaults: Record<ToolKind, Presentation> = {
  [ToolKind.Read]: { icon: faFileLines, permissionUi: PermissionUi.Row },
  [ToolKind.Edit]: { icon: faPen, permissionUi: PermissionUi.Modal },
  [ToolKind.Delete]: { icon: faTrash, permissionUi: PermissionUi.Modal },
  [ToolKind.Move]: { icon: faPenToSquare, permissionUi: PermissionUi.Modal },
  [ToolKind.Search]: { icon: faMagnifyingGlass, permissionUi: PermissionUi.Row },
  [ToolKind.Execute]: { icon: faTerminal, permissionUi: PermissionUi.Row },
  [ToolKind.Think]: { icon: faBrain, permissionUi: PermissionUi.Row },
  [ToolKind.Fetch]: { icon: faGlobe, permissionUi: PermissionUi.Row },
  [ToolKind.Other]: { icon: faPlug, permissionUi: PermissionUi.Row }
}

const adapterOverrides: Record<AdapterId, Record<string, Presentation>> = {
  [AdapterId.ClaudeCode]: claudeCodeOverrides,
  [AdapterId.Codex]: codexOverrides,
  [AdapterId.OpenCode]: opencodeOverrides,
  [AdapterId.Acp]: acpOverrides
}

/// Convert a wire tool-name (PascalCase, snake_case, kebab-case, …)
/// to the snake_case key both Rust and TS use for adapter override
/// lookup. Mirrors `convert_case::Case::Snake` for the patterns we
/// see — handles consecutive caps (`XMLHttpRequest` → `xml_http_request`)
/// and hyphens / spaces / dots → underscores.
function toSnake(name: string): string {
  if (!name) {
    return ''
  }
  let out = ''
  let prev: string | undefined

  for (let i = 0; i < name.length; i++) {
    const ch = name[i]!
    const next = name[i + 1]

    if (/[A-Za-z0-9]/.test(ch)) {
      if (/[A-Z]/.test(ch)) {
        const prevIsLowerOrDigit = prev !== undefined && /[a-z0-9]/.test(prev)
        const isAcronymBreak = prev !== undefined && /[A-Z]/.test(prev) && next !== undefined && /[a-z]/.test(next)

        if (out.length > 0 && !out.endsWith('_') && (prevIsLowerOrDigit || isAcronymBreak)) {
          out += '_'
        }
      }
      out += ch.toLowerCase()
    } else if (out.length > 0 && !out.endsWith('_')) {
      out += '_'
    }
    prev = ch
  }

  while (out.endsWith('_')) {
    out = out.slice(0, -1)
  }

  return out
}

export function presentationFor(
  kind: ToolKind | string | undefined,
  adapter: AdapterId | undefined,
  wireName: string | undefined,
  rawInput?: Record<string, unknown>
): Presentation {
  if (adapter !== undefined && wireName !== undefined && wireName.length > 0) {
    const overrides = adapterOverrides[adapter]

    if (overrides) {
      const key = toSnake(wireName)
      const hit = overrides[key]

      if (hit) {
        return hit
      }

      // mcp__server__leaf prefix shortcut for MCP tools.
      if (wireName.startsWith('mcp__')) {
        const mcp = overrides.mcp

        if (mcp) {
          return mcp
        }
      }
    }
  }

  // claude-code's switch_mode tool — title varies (`Ready to code?`,
  // `EnterPlanMode`, …) so neither the snake'd-wireName lookup nor the
  // kind classification (`other`) routes to the modal-permission UI.
  // Detect on the rawInput shape (`plan` is a non-empty string), same
  // signal the daemon-side PlanExitFormatter matcher uses. Adapter
  // gate keeps other vendors out — only claude-code-acp ships
  // switch_mode this way today.
  if (adapter === AdapterId.ClaudeCode && typeof rawInput?.plan === 'string' && rawInput.plan.length > 0) {
    const overrides = adapterOverrides[adapter]
    const planExit = overrides?.switch_mode ?? overrides?.exit_plan_mode

    if (planExit) {
      return planExit
    }
  }
  const k = (kind as ToolKind) ?? ToolKind.Other

  return kindDefaults[k] ?? kindDefaults[ToolKind.Other]!
}

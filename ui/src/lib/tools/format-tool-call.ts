import type { ToolCallView } from '@composables'

import { mapToolStatus, normaliseArgs, normaliseName } from './helpers'
import { extractContent } from './output'
import { resolveRegistry } from './registry'
import type { ToolChipItem } from '@interfaces/ui/chat'
import type { FormatterContext, ToolFormatterRegistry } from '@interfaces/ui/tools'

function resolveCanonical(registry: ToolFormatterRegistry, raw: string): string {
  const normalised = normaliseName(raw)
  const aliased = registry.aliases[normalised]

  return aliased ?? normalised
}

/**
 * Public facade: turn a `ToolCallView` into a `ToolChipItem`. Routes
 * through the registry resolved for `provider` (base when omitted /
 * unknown). Surfaces three derived fields the formatters don't
 * compute themselves:
 *
 *  - `title` — the agent-supplied `tool_call.title` when it differs
 *    from the kind label (otherwise dropped; "Bash" twice is noise).
 *  - `description` — first prose-shaped content block, rendered as
 *    markdown in the expanded body. Agents emit a descriptive
 *    paragraph here ("Reading auth.ts to find the validation hook").
 *  - `output` — remaining text blocks joined (terminal stdout, diff
 *    prose, raw tool result).
 *
 * Per ACP spec — `agent-client-protocol-schema-0.12.0::tool_call::ToolCall`
 * has no top-level `description` field; the prose lives in the first
 * text content block and the rest is output. claude-code-acp,
 * codex-acp, opencode all follow this convention.
 */
export function formatToolCall(call: ToolCallView, provider?: string): ToolChipItem {
  const registry = resolveRegistry(provider)
  const rawName = call.title ?? ''
  const canon = resolveCanonical(registry, rawName)
  const args = normaliseArgs(call.rawInput)
  const state = mapToolStatus(call.status)
  const ctx: FormatterContext = { name: canon, rawName, kind: call.kind ?? '', args, state }

  const formatter = registry.formatters[canon] ?? registry.fallback
  const chip = formatter.format(ctx)
  const { description, output } = extractContent(call.content)

  // Surface the agent's `title` only when it adds info beyond the
  // kind label — `tool_call.title === 'Bash'` matches `chip.label
  // === 'Execute'` semantically and shouldn't render twice. Compare
  // case-insensitively against the kind label, the formatter's
  // canonical key (e.g. `bash`), and that key with underscores
  // unfolded (`multi_edit` → `multi edit`). The canonical-from-rawName
  // path (when no formatter matches) is NOT checked — for unknown
  // tools the canonical IS just a normalised echo of the title, so
  // matching there would always strip otherwise-useful prose
  // ("Run unit tests" canonicalises to `run_unit_tests` whose
  // unfolded form matches the title).
  //
  // Formatter-supplied `chip.title` wins over the rawName-derived one:
  // the fallback's MCP path explicitly sets a humanised leaf name
  // (`browser navigate`) that's strictly more useful than the raw
  // `mcp__server__leaf` rawName. Only fall back to the rawName-based
  // dedup when the formatter didn't pick a title.
  const trimmedTitle = rawName.trim()
  const labelLower = chip.label.toLowerCase()
  const titleLower = trimmedTitle.toLowerCase()
  const isKnownFormatter = formatter !== registry.fallback
  const isRedundantTitle =
    titleLower === labelLower ||
    (isKnownFormatter &&
      (titleLower === formatter.canonical.toLowerCase() ||
        titleLower === formatter.canonical.replace(/_/g, ' ')))
  const fallbackTitle = trimmedTitle.length > 0 && !isRedundantTitle ? trimmedTitle : undefined
  const title = chip.title ?? fallbackTitle

  return {
    ...chip,
    ...(title ? { title } : {}),
    ...(description ? { description } : {}),
    ...(output ? { output } : {})
  }
}

/**
 * Public lookup: short verb word for a tool name. MCP tools fall back
 * to the leaf name title-cased; unknown built-ins get the bullet `·`
 * since the chip's visual is FA-icon-driven and a long lowercased
 * canonical name would just be noise on text-only surfaces.
 */
export function shortHeader(toolName: string, provider?: string): string {
  const registry = resolveRegistry(provider)
  const canon = resolveCanonical(registry, toolName)
  const hit = registry.shortHeaders[canon]
  if (hit) {
    return hit
  }

  // Surface MCP-style names with a friendly leaf title. Anything else
  // collapses to the bullet — see `fallbackFormatter` for the same
  // policy applied to the visible chip.
  if (canon.startsWith('mcp__')) {
    const parts = canon.split('__')
    const tool = parts.slice(2).join('__').replace(/_/g, ' ').trim()

    return tool.length > 0 ? tool.charAt(0).toUpperCase() + tool.slice(1).toLowerCase() : '·'
  }

  return '·'
}

/** Stub kept for the legacy import path. Expanded-row markdown isn't wired here today. */
export function formatToolBody(_call: ToolCallView, _provider?: string): string {
  throw new Error('formatToolBody: expanded-row markdown not implemented')
}

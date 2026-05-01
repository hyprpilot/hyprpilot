import { titleCaseFromCanonical } from './casing'
import { bashFormatter, bashOutputFormatter, killShellFormatter } from './formatters/bash'
import { editFormatter, multiEditFormatter } from './formatters/edit'
import { fallbackFormatter } from './formatters/fallback'
import { globFormatter } from './formatters/glob'
import { grepFormatter } from './formatters/grep'
import { notebookEditFormatter } from './formatters/notebook'
import { planEnterFormatter, planExitFormatter } from './formatters/plan'
import { readFormatter } from './formatters/read'
import { skillFormatter } from './formatters/skill'
import { taskFormatter } from './formatters/task'
import { terminalFormatter } from './formatters/terminal'
import { todoWriteFormatter } from './formatters/todo'
import { toolSearchFormatter } from './formatters/tool-search'
import { webFetchFormatter, webSearchFormatter } from './formatters/web'
import { writeFormatter } from './formatters/write'
import type { ToolFormatter, ToolFormatterRegistry } from '@interfaces/ui/tools'

/**
 * Every base-shipped formatter. Adding a tool = drop a file in
 * `formatters/`, export the `ToolFormatter`, and append to this list.
 * `buildRegistry` derives the lookup tables; canonical name + aliases
 * are read directly off the formatter so the metadata stays beside
 * its implementation.
 */
const BASE_FORMATTERS: readonly ToolFormatter[] = [
  bashFormatter,
  bashOutputFormatter,
  killShellFormatter,
  editFormatter,
  multiEditFormatter,
  globFormatter,
  grepFormatter,
  notebookEditFormatter,
  planEnterFormatter,
  planExitFormatter,
  readFormatter,
  skillFormatter,
  taskFormatter,
  terminalFormatter,
  todoWriteFormatter,
  toolSearchFormatter,
  webFetchFormatter,
  webSearchFormatter,
  writeFormatter
]

/**
 * Casing-collapse aliases (claude-code-acp emits `BashOutput` etc.,
 * which `normaliseName` lowercases to `bashoutput`; the canonical
 * formatter keys carry underscores). Cross-vendor synonyms — `patch`
 * → `edit`, `agent` → `task`, `todo` → `todo_write`, …  — are
 * declared on the formatter itself via its `aliases` array and folded
 * in alongside this map by `buildRegistry`.
 */
const CASING_COLLAPSE_ALIASES: Record<string, string> = {
  bashoutput: 'bash_output',
  killshell: 'kill_shell',
  multiedit: 'multi_edit',
  webfetch: 'web_fetch',
  websearch: 'web_search',
  notebookedit: 'notebook_edit',
  todowrite: 'todo_write',
  toolsearch: 'tool_search'
}

/**
 * Default chip text label for a formatter — the canonical key
 * title-cased per `titleCaseFromCanonical`. Formatters can override
 * with their own `label` field when the natural form reads worse
 * than a custom label.
 */
function labelFor(f: ToolFormatter): string {
  return f.label ?? titleCaseFromCanonical(f.canonical)
}

function buildRegistry(formatters: readonly ToolFormatter[], fallback: ToolFormatter): ToolFormatterRegistry {
  const formattersMap: Record<string, ToolFormatter> = {}
  const shortHeaders: Record<string, string> = {}
  const aliases: Record<string, string> = { ...CASING_COLLAPSE_ALIASES }

  for (const f of formatters) {
    formattersMap[f.canonical] = f
    shortHeaders[f.canonical] = labelFor(f)
    for (const alias of f.aliases ?? []) {
      aliases[alias] = f.canonical
      shortHeaders[alias] = labelFor(f)
    }
  }

  return { formatters: formattersMap, shortHeaders, aliases, fallback }
}

export const baseRegistry: ToolFormatterRegistry = buildRegistry(BASE_FORMATTERS, fallbackFormatter)

/**
 * Compose a registry layered onto `base`. Per-adapter divergence
 * (opencode `diagnostics` leftover surfacing, codex `$bash_id`
 * semantics, claude-code MCP tools) lands as a `{ provider:
 * extendRegistry(baseRegistry, {...}) }` map the moment the first
 * real override appears. Today every vendor inherits base untouched,
 * so `resolveRegistry` is a pass-through.
 */
export function extendRegistry(
  base: ToolFormatterRegistry,
  patch: {
    shortHeaders?: Record<string, string>
    aliases?: Record<string, string>
    formatters?: Record<string, ToolFormatter>
    fallback?: ToolFormatter
  }
): ToolFormatterRegistry {
  return {
    shortHeaders: { ...base.shortHeaders, ...(patch.shortHeaders ?? {}) },
    aliases: { ...base.aliases, ...(patch.aliases ?? {}) },
    formatters: { ...base.formatters, ...(patch.formatters ?? {}) },
    fallback: patch.fallback ?? base.fallback
  }
}

/** Resolve the registry for a provider (or the base when omitted). */
export function resolveRegistry(_provider?: string): ToolFormatterRegistry {
  return baseRegistry
}

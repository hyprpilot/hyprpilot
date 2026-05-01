/**
 * Tool-call → `ToolChipItem` registry. Each canonical formatter
 * lives in `formatters/<name>.ts` implementing the `ToolFormatter`
 * interface (`canonical`, `label`, `kind`, `aliases?`, `format()`)
 * and is assembled into the lookup map by `registry.ts`.
 *
 * Adding a new formatter:
 *   1. Drop a new file under `formatters/<canonical-name>.ts`.
 *   2. Export a `ToolFormatter` const. Use `aliases` for cross-vendor
 *      synonyms; casing-collapse aliases (`bashoutput` →
 *      `bash_output`) live in `registry.ts::CASING_COLLAPSE_ALIASES`.
 *   3. Append to the `BASE_FORMATTERS` list in `registry.ts`. The
 *      registry derives the lookup map automatically.
 *
 * Per-adapter divergence (claude-code MCP tools, codex `$bash_id`
 * semantics, opencode quirks) layers via `extendRegistry(baseRegistry,
 * { formatters, aliases, … })`.
 */
export { titleCaseFromCanonical } from './casing'
export { formatToolCall, formatToolBody, shortHeader } from './format-tool-call'
export { baseRegistry, extendRegistry, resolveRegistry } from './registry'
export type { FormatterContext, ToolFormatter, ToolFormatterRegistry, Args } from '@interfaces/ui'

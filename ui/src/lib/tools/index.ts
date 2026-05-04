/**
 * Tool-call formatter facade. `format(call, adapter?)` is the single
 * public entry — every consumer (chat pill, permission row,
 * permission modal) calls this and reads the unified `ToolCallView`.
 *
 * Adding a tool:
 *   1. Add a variant to `constants/ui/tools/type::ToolType`.
 *   2. Drop a folder under `lib/tools/<canonical>/` with `fallback.ts`
 *      (the shared formatter) + `index.ts` (`pickFormatter(fallback)`).
 *   3. Register the canonical key in `formatters` below.
 *
 * Per-adapter overrides land alongside `<canonical>/fallback.ts` as
 * `<canonical>/claude-code.ts` / `<canonical>/codex.ts` /
 * `<canonical>/opencode.ts`; pass them into `pickFormatter` in the
 * folder's `index.ts`.
 */

import bash from './bash'
import { canonicalise } from './canonicalise'
import editEntry from './edit'
import fallbackFormatter from './fallback'
import glob from './glob'
import grep from './grep'
import killShell from './kill-shell'
import mcp from './mcp'
import multiEdit from './multi-edit'
import notebookEdit from './notebook-edit'
import planExit from './plan-exit'
import read from './read'
import { isMcpName, mapState, normaliseArgs } from './shared'
import skill from './skill'
import task from './task'
import terminal from './terminal'
import think from './think'
import todo from './todo'
import toolSearch from './tool-search'
import webFetch from './web-fetch'
import webSearch from './web-search'
import write from './write'
import type { AdapterId } from '@constants/ui'
import type { FormatterContext, Formatters, ToolCallView, WireToolCall } from '@interfaces/ui'

/**
 * Dispatch by canonical wire name. Includes ACP `tool_call.kind` values
 * as aliases for the matching tool family (`execute` → bash, `search`
 * → grep, `fetch` → web_fetch, `switch_mode` → plan_exit) so wire
 * payloads that ship the kind verb instead of a PascalCase tool name
 * (codex, opencode permission requests) route to the right formatter
 * without falling through to the generic last-resort renderer.
 */

const formatters: Record<string, Formatters> = {
  bash,
  bash_output: bash,
  execute: bash,
  kill_shell: killShell,
  terminal,
  read,
  write,
  edit: editEntry,
  multi_edit: multiEdit,
  notebook_edit: notebookEdit,
  grep,
  search: grep,
  glob,
  tool_search: toolSearch,
  web_fetch: webFetch,
  fetch: webFetch,
  web_search: webSearch,
  plan_exit: planExit,
  exit_plan_mode: planExit,
  switch_mode: planExit,
  todo_write: todo,
  todo,
  think,
  skill,
  task,
  agent: task
}

export function format(call: WireToolCall, adapter?: AdapterId): ToolCallView {
  // Routing key — prefer the frozen `wireName` (set on the first
  // `tool_call.start` and never overwritten) over `title`. opencode
  // rewrites `title` to a prose state-title on every update (e.g.
  // `task` → `Find files matching pattern`), which would otherwise
  // collapse the formatter dispatch to the generic fallback.
  const wireName = call.wireName ?? call.title ?? ''
  // Two dispatch passes: (1) the ACP `kind` discriminator
  // (`read`/`edit`/`execute`/…) routes to the canonical family
  // formatter — works even when the agent ships a long descriptive
  // title like `"Write /tmp/foo.txt"` that snakeCase would mangle
  // into a non-matching `write_tmp_foo_txt`. (2) Title-based
  // canonicalisation as fallback for vendor-native PascalCase names
  // (`MultiEdit` / `WebFetch`) that don't have a matching ACP kind
  // keyword.
  const titleCanon = canonicalise(wireName)
  const kindCanon = canonicalise(call.kind)
  const ctx: FormatterContext = {
    name: wireName,
    args: normaliseArgs(call.rawInput),
    state: mapState(call.status),
    raw: call,
    adapter
  }

  const entry = formatters[titleCanon] ?? formatters[kindCanon]

  if (entry) {
    return entry(adapter).format(ctx)
  }

  if (isMcpName(titleCanon) || isMcpName(kindCanon)) {
    return mcp(adapter).format(ctx)
  }

  return fallbackFormatter.format(ctx)
}

import type { ToolCallView } from '@composables'

/**
 * Strip a wrapping markdown code fence (` ```lang\n…\n``` `) so the
 * inner content renders as plain text in the chip's output `<pre>`.
 * Agents commonly wrap shell output in fences for markdown rendering
 * elsewhere; the chip's body is already preformatted, so the fences
 * just clutter. Non-fenced text passes through unchanged.
 */
export function unwrapCodeFence(text: string): string {
  const m = text.match(/^\s*```[a-zA-Z0-9_+-]*\n([\s\S]*?)\n?```\s*$/)

  return m ? m[1] : text
}

/**
 * Decide whether a text block looks like prose (a description) vs
 * output (terminal log / diff). Heuristic: prose contains at least
 * one space and doesn't start with a typical log-line marker. ACP
 * doesn't formally separate the two — claude-code-acp emits a
 * descriptive prose block first, then output blocks; this split
 * mirrors that convention.
 */
function looksLikeProse(text: string): boolean {
  const trimmed = text.trim()
  if (trimmed.length === 0) {
    return false
  }
  // Markdown-fenced text is output, never prose.
  if (trimmed.startsWith('```')) {
    return false
  }
  // Diff-shaped first line.
  if (/^(diff --git|---\s|\+\+\+\s|@@\s)/.test(trimmed)) {
    return false
  }
  // Single-token / no-space lines are usually identifiers / output.
  return /\s/.test(trimmed) && /[A-Za-z]{4,}/.test(trimmed)
}

export interface ExtractedContent {
  /// First prose block (markdown) — what the tool is "about". Goes
  /// in the chip's expanded body above `output`.
  description?: string
  /// Remaining text blocks joined — terminal stdout, diff text, etc.
  output?: string
}

/**
 * Walk the tool call's content blocks and split into a single
 * markdown-friendly description (the first prose-shaped text block)
 * and the rest as concatenated output.
 *
 * Heuristic — narration is only present when the agent emitted
 * multiple blocks: a prose preamble FIRST followed by the actual
 * result (claude-code-acp's pattern: "Reading the auth module to
 * find …" + the file contents). When the agent emits a single
 * content block — the common shape for MCP tools, raw Read / Edit
 * results, terminal stdout — that block IS the result, not
 * narration. Splitting it would leave the output block empty and
 * the captain has to scroll the description prose to see what came
 * back.
 *
 * Rule: only treat a leading prose-shaped block as `description`
 * when there are ≥2 non-empty blocks, AND the first block looks
 * like prose. Otherwise everything is output.
 *
 * Each block has its wrapping markdown code fence stripped before
 * joining so the chip's output `<pre>` doesn't show literal
 * ```` ``` ```` markup.
 */
export function extractContent(content: ToolCallView['content']): ExtractedContent {
  const blocks = content.map((b) => (typeof b.text === 'string' ? b.text : '')).filter((t) => t.length > 0)
  if (blocks.length === 0) {
    return {}
  }

  let description: string | undefined
  let outputStart = 0
  if (blocks.length >= 2 && looksLikeProse(blocks[0])) {
    description = blocks[0]
    outputStart = 1
  }
  const outputs = blocks.slice(outputStart).map(unwrapCodeFence)

  return {
    description,
    output: outputs.length > 0 ? outputs.join('\n') : undefined
  }
}

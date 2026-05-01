import { ToolKind } from '@components'

import { pickString } from '../helpers'
import type { ToolFormatter } from '@components'

/**
 * `Skill` is claude-code-acp's first-party tool for invoking a
 * registered skill bundle. Wire shape: `{ skill: "<slug>" }` (the
 * slug is the agent-side skill identifier — e.g.
 * `superpowers:using-superpowers`). Without this formatter, the
 * fallback rendered the raw JSON string in the chip's `arg` slot
 * (`{"skill":"superpowers:using-superpowers"}`) which buries the
 * actual signal under syntax noise.
 *
 * We surface the slug directly as `arg` and rely on the kind icon +
 * label for the visual.
 */
export const skillFormatter: ToolFormatter = {
  canonical: 'skill',
  kind: ToolKind.Agent,
  format({ args, state }) {
    const slug = pickString(args, 'skill')

    return {
      label: 'Skill',
      arg: slug,
      state,
      kind: this.kind
    }
  }
}

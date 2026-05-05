/**
 * Read-only palette leaf for the resolved MCP catalog. Lists every
 * server in the active instance's effective set (profile's `mcps` ⊕
 * global `mcps`) on the left; the master-detail `preview` slot on the
 * right shows the structured entry — name, source path (relativized),
 * command/args, env keys (values redacted), per-server hyprpilot
 * extension globs, and a collapsible raw JSON disclosure for any
 * unknown extension keys.
 *
 * Captains who want to toggle a server off edit the source JSON file
 * + restart the daemon (`mcps` on disk is the authoritative state;
 * UI is read-only because ACP fixes mcpServers at session/new and
 * mid-session toggling would force a per-toggle restart).
 */

import MCPsPreview from './MCPsPreview.vue'
import { type PaletteEntry, PaletteMode, useHomeDir, usePalette } from '@composables'
import { invoke, TauriCommand } from '@ipc'

export interface OpenMcpsLeafOptions {
  instanceId?: string
}

/**
 * Open the MCP catalog palette. `instanceId` is optional — when
 * present the daemon resolves the per-instance effective file set;
 * when absent the global default is shown.
 */
export async function openMcpsLeaf(opts: OpenMcpsLeafOptions = {}): Promise<void> {
  const { open } = usePalette()
  const { displayPath } = useHomeDir()
  const result = await invoke(TauriCommand.McpsList, { instanceId: opts.instanceId })
  const items = result.mcps
  const entries: PaletteEntry[] = items.map((m) => ({
    id: m.name,
    name: m.name,
    description: displayPath(m.source)
  }))

  open({
    mode: PaletteMode.Select,
    title: 'mcps',
    entries,
    preview: { component: MCPsPreview, props: { items } },
    onCommit(): void {
      // Read-only — Enter is a no-op. Captain edits source JSON +
      // restarts to change the set.
    }
  })
}

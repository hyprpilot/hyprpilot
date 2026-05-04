/**
 * Module-state cache of `agents/list` results so any UI consumer can
 * synchronously resolve `agentId → AdapterId` without re-fetching.
 * Populated on first call to `useAgentRegistry()` (lazy boot) and
 * refreshable via `refresh()` after daemon reload events.
 *
 * The presentation layer (`lib/tools/presentation::presentationFor`)
 * needs the adapter id to dispatch per-vendor overrides; the daemon's
 * transcript / permission events carry only `agentId` (the
 * config-defined name like `claude-code`), so this composable is the
 * bridge.
 */

import { ref, type Ref } from 'vue'

import { AdapterId } from '@constants/ui'
import { invoke, TauriCommand, type AgentSummary } from '@ipc'
import { log } from '@lib'

const cache = ref<Map<string, AdapterId>>(new Map())
let bootPromise: Promise<void> | undefined

function providerToAdapter(provider: string): AdapterId | undefined {
  switch (provider) {
    case AdapterId.ClaudeCode:
      return AdapterId.ClaudeCode
    case AdapterId.Codex:
      return AdapterId.Codex
    case AdapterId.OpenCode:
      return AdapterId.OpenCode
    case AdapterId.Acp:
      return AdapterId.Acp
    default:
      return undefined
  }
}

async function load(): Promise<void> {
  try {
    const r = await invoke(TauriCommand.AgentsList)
    const next = new Map<string, AdapterId>()

    for (const agent of r.agents as AgentSummary[]) {
      const adapter = providerToAdapter(agent.provider)

      if (adapter !== undefined) {
        next.set(agent.id, adapter)
      }
    }
    cache.value = next
  } catch(err) {
    log.warn('agent-registry: agents/list failed', { err: String(err) })
  }
}

export interface UseAgentRegistryApi {
  /// Reactive map of `agentId → AdapterId`. Empty until first
  /// `agents/list` resolves.
  byId: Ref<Map<string, AdapterId>>
  /// Synchronous lookup. Returns `undefined` if the registry hasn't
  /// loaded yet OR the agent isn't registered.
  adapterFor: (agentId: string | undefined) => AdapterId | undefined
  /// Force a re-fetch (after `daemon/reload` events).
  refresh: () => Promise<void>
}

export function useAgentRegistry(): UseAgentRegistryApi {
  if (!bootPromise) {
    bootPromise = load()
  }

  function adapterFor(agentId: string | undefined): AdapterId | undefined {
    if (agentId === undefined) {
      return undefined
    }

    return cache.value.get(agentId)
  }

  async function refresh(): Promise<void> {
    bootPromise = load()
    await bootPromise
  }

  return {
    byId: cache,
    adapterFor,
    refresh
  }
}

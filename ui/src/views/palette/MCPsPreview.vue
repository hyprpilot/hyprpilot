<script setup lang="ts">
/**
 * Right-pane preview for the read-only `openMcpsLeaf()`. Receives
 * the highlighted entry from `CommandPalette.vue` and the full item
 * collection via `props` so it can look up structured fields by
 * name without re-fetching. Renders a structured `<dl>` for the
 * known fields (name, source, command/args, env keys with redacted
 * values, hyprpilot.autoAcceptTools / autoRejectTools) plus a
 * collapsible raw JSON disclosure for anything else (vendor
 * extensions, future MCP-spec additions).
 *
 * Empty state when no entry is highlighted (filter yielded zero
 * rows). No loading state — items are loaded synchronously by the
 * leaf opener and passed in.
 */
import { computed } from 'vue'

import { type PaletteEntry } from '@composables'
import { useHomeDir } from '@composables'
import type { MCPItem } from '@ipc'

const props = defineProps<{
  entry?: PaletteEntry
  items: MCPItem[]
}>()

const { homeDir } = useHomeDir()

const active = computed<MCPItem | undefined>(() => {
  if (!props.entry) {
    return undefined
  }
  return props.items.find((m) => m.name === props.entry?.id)
})

const sourceDisplay = computed(() => {
  const src = active.value?.source
  if (!src) {
    return ''
  }
  if (homeDir.value && src.startsWith(homeDir.value)) {
    return `~${src.slice(homeDir.value.length)}`
  }
  return src
})

const commandStr = computed<string | undefined>(() => {
  const raw = active.value?.raw
  if (!raw) {
    return undefined
  }
  const cmd = typeof raw.command === 'string' ? raw.command : undefined
  if (!cmd) {
    return undefined
  }
  const args = Array.isArray(raw.args) ? raw.args.filter((a): a is string => typeof a === 'string') : []
  return [cmd, ...args].join(' ')
})

const urlStr = computed<string | undefined>(() => {
  const raw = active.value?.raw
  if (!raw) {
    return undefined
  }
  return typeof raw.url === 'string' ? raw.url : undefined
})

const envKeys = computed<string[]>(() => {
  const raw = active.value?.raw
  if (!raw || typeof raw.env !== 'object' || raw.env === null) {
    return []
  }
  return Object.keys(raw.env as Record<string, unknown>).sort()
})

const acceptGlobs = computed<string[]>(() => active.value?.hyprpilot.autoAcceptTools ?? [])
const rejectGlobs = computed<string[]>(() => active.value?.hyprpilot.autoRejectTools ?? [])

const rawJson = computed(() => {
  const raw = active.value?.raw
  if (!raw) {
    return ''
  }
  return JSON.stringify(raw, null, 2)
})
</script>

<template>
  <div v-if="active" class="mcp-preview">
    <dl class="mcp-preview-dl">
      <dt>name</dt>
      <dd>{{ active.name }}</dd>

      <dt>source</dt>
      <dd class="mcp-preview-mono">{{ sourceDisplay }}</dd>

      <template v-if="commandStr">
        <dt>command</dt>
        <dd class="mcp-preview-mono">{{ commandStr }}</dd>
      </template>

      <template v-if="urlStr">
        <dt>url</dt>
        <dd class="mcp-preview-mono">{{ urlStr }}</dd>
      </template>

      <template v-if="envKeys.length > 0">
        <dt>env</dt>
        <dd>
          <ul class="mcp-preview-list">
            <li v-for="k in envKeys" :key="k" class="mcp-preview-mono">
              {{ k }} = <span class="mcp-preview-redacted">***</span>
            </li>
          </ul>
        </dd>
      </template>

      <template v-if="acceptGlobs.length > 0">
        <dt>auto-accept</dt>
        <dd>
          <ul class="mcp-preview-list">
            <li v-for="g in acceptGlobs" :key="g" class="mcp-preview-mono">{{ g }}</li>
          </ul>
        </dd>
      </template>

      <template v-if="rejectGlobs.length > 0">
        <dt>auto-reject</dt>
        <dd>
          <ul class="mcp-preview-list">
            <li v-for="g in rejectGlobs" :key="g" class="mcp-preview-mono">{{ g }}</li>
          </ul>
        </dd>
      </template>
    </dl>

    <details class="mcp-preview-raw">
      <summary>raw JSON</summary>
      <pre class="mcp-preview-mono">{{ rawJson }}</pre>
    </details>
  </div>
  <div v-else class="mcp-preview-empty">no MCP highlighted</div>
</template>

<style scoped>
@reference '../../assets/styles.css';

.mcp-preview {
  @apply flex flex-col gap-3 overflow-y-auto;
  padding: 12px 14px;
  font-size: 0.72rem;
}

.mcp-preview-dl {
  @apply grid;
  grid-template-columns: max-content 1fr;
  column-gap: 12px;
  row-gap: 6px;
}

.mcp-preview-dl dt {
  color: var(--theme-fg-dim);
  font-weight: 600;
}

.mcp-preview-dl dd {
  color: var(--theme-fg);
}

.mcp-preview-mono {
  font-family: var(--theme-font-mono);
}

.mcp-preview-list {
  @apply flex flex-col gap-[2px];
}

.mcp-preview-redacted {
  color: var(--theme-fg-faint);
  font-family: var(--theme-font-mono);
}

.mcp-preview-raw {
  border-top: 1px solid var(--theme-border-soft);
  padding-top: 10px;
}

.mcp-preview-raw summary {
  @apply cursor-pointer;
  color: var(--theme-fg-dim);
}

.mcp-preview-raw pre {
  @apply mt-2 overflow-x-auto;
  padding: 8px;
  background-color: var(--theme-surface-alt);
  border-radius: 3px;
  font-size: 0.68rem;
}

.mcp-preview-empty {
  @apply flex items-center justify-center;
  padding: 16px;
  color: var(--theme-fg-dim);
  font-size: 0.72rem;
}
</style>

<script setup lang="ts">
import { faCheck, faCheckDouble, faXmark } from '@fortawesome/free-solid-svg-icons'
import { computed } from 'vue'

import ToolSpecSheet from '../chat/ToolSpecSheet.vue'
import { iconForToolKind, type PermissionPrompt } from '@components'
import { titleCaseFromCanonical } from '@lib'

/**
 * permission panel — pinned bottom band, max-height 45vh.
 *
 * Renders the oldest active prompt as a structured spec sheet
 * (kind icon + tool tag + counter pill on the left, action buttons
 * + match-status chip on the right; per-field rows below).
 *
 * Multiple permissions are processed one at a time — only the
 * oldest non-queued prompt drives the panel. The header counter
 * shows `current of total` so the captain knows how many are
 * lined up. The match status (whether the trust store has a rule
 * for this tool yet) rides on a small chip immediately left of
 * the action buttons — verbose enough to be informative, brief
 * enough not to crowd the action row.
 *
 * The orange (`awaiting`) phase is set on the parent Frame; the
 * panel pairs with that — bg `permission-bg`, top border 2px in
 * `warn`. All four signals (frame border, profile pill, panel bg,
 * action buttons) agree per visual law #1.
 */
const props = defineProps<{
  prompts: PermissionPrompt[]
}>()

const emit = defineEmits<{
  allow: [id: string]
  deny: [id: string]
}>()

const active = computed(() => props.prompts.find((p) => !p.queued))
const total = computed(() => props.prompts.length)
const activeIndex = computed(() => {
  if (!active.value) {
    return 0
  }

  return props.prompts.findIndex((p) => p.id === active.value!.id) + 1
})
const kindIcon = computed(() => iconForToolKind(active.value?.kind))

// Human-readable label for the wire `kind` enum (`execute` → "Execute",
// `web_fetch` → "Web fetch"). Same casing rule the chat pill uses.
const kindLabel = computed(() => {
  const raw = active.value?.kind
  return raw ? titleCaseFromCanonical(raw) : ''
})

// Drop the `· tool` suffix when the agent's `tool` string matches the
// kind label case-insensitively (e.g. kind=`bash`, tool="Bash"). Same
// dedup the chat pill applies via `formatToolCall::isRedundantTitle`.
const showToolName = computed(() => {
  const tool = active.value?.tool?.trim()
  if (!tool) {
    return false
  }
  return tool.toLowerCase() !== kindLabel.value.toLowerCase()
})

/**
 * Best-effort flag extraction from the `args` string. Anything starting
 * with `-` is treated as a flag chip; words preceded by a flag get
 * folded into that chip. Quoted strings are skipped (they're values,
 * not flags). The wireframe spec calls for a flag-by-flag breakdown
 * with KV chips; without per-tool grammars we surface raw flag tokens
 * so the captain can scan the dangerous-looking ones at a glance.
 */
const parsedFlags = computed<string[]>(() => {
  const args = active.value?.args
  if (!args) return []
  const flags: string[] = []
  for (const token of args.split(/\s+/)) {
    if (token.startsWith('-')) {
      flags.push(token)
    }
  }
  return flags
})
</script>

<template>
  <section v-if="active" class="permission-panel" data-testid="permission-stack">
    <header class="permission-panel-header">
      <span class="permission-panel-tool" :aria-label="`${kindLabel}${showToolName ? ` · ${active.tool}` : ''}`">
        <FaIcon :icon="kindIcon" class="permission-panel-tool-icon" aria-hidden="true" />
        <span class="permission-panel-tool-kind">{{ kindLabel }}</span>
        <template v-if="showToolName">
          <span class="permission-panel-tool-sep" aria-hidden="true">·</span>
          <span class="permission-panel-tool-name">{{ active.tool }}</span>
        </template>
      </span>
      <span v-if="total > 1" class="permission-panel-counter">
        {{ activeIndex }} of {{ total }}
      </span>
      <span class="permission-panel-spacer" />
      <div class="permission-panel-actions">
        <button
          type="button"
          class="permission-panel-icon-btn"
          data-tone="ok"
          data-variant="solid"
          aria-label="allow once"
          title="allow once"
          @click="emit('allow', active.id)"
        >
          <FaIcon :icon="faCheck" class="permission-panel-icon" aria-hidden="true" />
        </button>
        <button
          type="button"
          class="permission-panel-icon-btn"
          data-tone="ok"
          aria-label="allow all"
          title="allow all (always allow this pattern)"
          @click="emit('allow', active.id)"
        >
          <FaIcon :icon="faCheckDouble" class="permission-panel-icon" aria-hidden="true" />
        </button>
        <button
          type="button"
          class="permission-panel-icon-btn"
          data-tone="err"
          aria-label="deny once"
          title="deny once"
          @click="emit('deny', active.id)"
        >
          <FaIcon :icon="faXmark" class="permission-panel-icon" aria-hidden="true" />
        </button>
        <button
          type="button"
          class="permission-panel-icon-btn"
          data-tone="err"
          aria-label="deny all"
          title="deny all (always deny this pattern)"
          @click="emit('deny', active.id)"
        >
          <FaIcon :icon="faXmark" class="permission-panel-icon permission-panel-icon-double" aria-hidden="true" />
        </button>
      </div>
    </header>
    <div class="permission-panel-body">
      <ToolSpecSheet
        :command="active.args"
        :flags="parsedFlags"
      />
    </div>
  </section>
</template>

<style scoped>
@reference '../../assets/styles.css';

/* permission-bg fill, top border 2px warn, max-height 45vh,
 * scrolls internally on overflow. */
.permission-panel {
  @apply flex flex-col overflow-y-auto;
  background-color: var(--theme-permission-bg);
  border-top: 2px solid var(--theme-status-warn);
  max-height: 45vh;
}

.permission-panel-header {
  @apply sticky top-0 z-10 flex items-center gap-[10px] text-[0.7rem];
  background-color: var(--theme-permission-bg);
  border-bottom: 1px solid var(--theme-border-soft);
  font-family: var(--theme-font-mono);
  padding: 6px 14px 6px 4px;
}

.permission-panel-tool {
  @apply inline-flex shrink-0 items-center gap-[5px] text-[0.62rem];
  background-color: var(--theme-status-warn);
  color: var(--theme-fg-on-tone);
  padding: 2px 7px;
  border-radius: 3px;
  font-weight: 700;
}

.permission-panel-tool-icon {
  width: 9px;
  height: 9px;
}

.permission-panel-tool-kind {
  font-weight: 700;
}

.permission-panel-tool-sep {
  opacity: 0.7;
  font-weight: 400;
}

.permission-panel-tool-name {
  font-weight: 600;
}

/* `1 of N` counter pill. Mirrors `+N queued` from the prior layout
 * but expresses position-in-queue (current / total) so the captain
 * knows how many decisions are ahead. Updates live as new prompts
 * land or the active one resolves. */
.permission-panel-counter {
  @apply inline-flex shrink-0 items-center font-bold text-[0.6rem];
  background-color: var(--theme-surface-bg);
  border: 1px solid var(--theme-border-soft);
  color: var(--theme-fg);
  padding: 1px 7px;
  border-radius: 3px;
  letter-spacing: 0.4px;
}


.permission-panel-spacer {
  flex: 1;
}

.permission-panel-actions {
  @apply flex shrink-0 items-center gap-1;
}

/* wireframe iconBtn: 22x22 ghost square; tone drives border + ink. */
.permission-panel-icon-btn {
  @apply inline-flex items-center justify-center;
  width: 22px;
  height: 22px;
  padding: 0;
  border-radius: 3px;
  background-color: transparent;
  cursor: pointer;
}

.permission-panel-icon-btn[data-tone='ok'] {
  border: 1px solid var(--theme-status-ok);
  color: var(--theme-status-ok);
}

.permission-panel-icon-btn[data-tone='err'] {
  border: 1px solid var(--theme-status-err);
  color: var(--theme-status-err);
}

.permission-panel-icon-btn[data-variant='solid'][data-tone='ok'] {
  background-color: var(--theme-status-ok);
  color: var(--theme-fg-on-tone);
}

.permission-panel-icon {
  width: 11px;
  height: 11px;
}

/* Visual hint for "deny all" — slightly heavier ink so the double-x
 * variant reads stronger than the single. */
.permission-panel-icon-double {
  width: 13px;
  height: 13px;
}

.permission-panel-icon-btn:hover {
  filter: brightness(1.15);
}

/* Body — wraps `ToolSpecSheet` (the same component the tool pill's
 * expanded body uses, so vocabulary stays consistent). Padding +
 * scroll live here; the spec sheet's own output section caps
 * independently so the captain can scroll the panel as a whole AND
 * the output `<pre>` independently when it's a long stream. */
.permission-panel-body {
  @apply flex flex-col;
  padding: 8px 10px;
}

</style>

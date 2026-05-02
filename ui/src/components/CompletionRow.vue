<script setup lang="ts">
import { faFile, faFileCode, faFileLines, faFolder, faMagnifyingGlass, faStar, faTerminal } from '@fortawesome/free-solid-svg-icons'
import { computed } from 'vue'

import { type CompletionItem, CompletionKind } from '@ipc'

/**
 * One row in the completion popover. Layout (left-to-right):
 *   [kind icon] label (detail in dim parens)        [source tag]
 *
 * - `label` is the primary text (skill title, command name, etc.).
 * - `detail`, when present, renders in dim parens after the label —
 *   reads as "title (description)". Path entries use this for the
 *   `dir` tag; ripgrep for the hit count or path.
 * - Source tag is derived from `kind` and right-aligned, dim, in
 *   square brackets so the row reads like an editor completion.
 *
 * Active row gets a tone-bg + accent-fg highlight via `[data-active]`.
 */
const props = defineProps<{
  item: CompletionItem
  active: boolean
}>()

const emit = defineEmits<{
  hover: []
  click: []
}>()

const icon = computed(() => {
  switch (props.item.kind) {
    case CompletionKind.Skill:
      return faStar

    case CompletionKind.Path: {
      const detail = props.item.detail ?? ''

      if (detail === 'dir') {
        return faFolder
      }
      const label = props.item.label

      if (label.endsWith('.md') || label.endsWith('.txt')) {
        return faFileLines
      }

      if (label.match(/\.(ts|tsx|js|jsx|rs|py|go|java|rb)$/)) {
        return faFileCode
      }

      return faFile
    }

    case CompletionKind.Word:
      return faMagnifyingGlass

    case CompletionKind.Command:
      return faTerminal
  }

  return faFile
})

const sourceLabel = computed<string>(() => {
  switch (props.item.kind) {
    case CompletionKind.Skill:
      return 'skill'

    case CompletionKind.Path:
      return 'path'

    case CompletionKind.Word:
      return 'ripgrep'

    case CompletionKind.Command:
      return 'command'
  }

  return ''
})
</script>

<template>
  <button type="button" class="completion-row" :data-active="active" @mouseenter="emit('hover')" @click.prevent="emit('click')">
    <FaIcon :icon="icon" class="completion-row-icon" aria-hidden="true" />
    <span class="completion-row-label">{{ item.label }}</span>
    <span v-if="item.detail" class="completion-row-detail">({{ item.detail }})</span>
    <span class="completion-row-source">[{{ sourceLabel }}]</span>
  </button>
</template>

<style scoped>
@reference '../assets/styles.css';

.completion-row {
  @apply flex w-full items-center gap-2 border-0 bg-transparent px-3 py-1 text-left;
  font-family: var(--theme-font-mono);
  font-size: 0.78rem;
  color: var(--theme-fg-ink-2);
  cursor: pointer;
}

.completion-row:hover,
.completion-row[data-active='true'] {
  background-color: var(--theme-surface-alt);
  color: var(--theme-fg);
}

.completion-row-icon {
  width: 12px;
  height: 12px;
  color: var(--theme-fg-dim);
  flex-shrink: 0;
}

.completion-row[data-active='true'] .completion-row-icon {
  color: var(--theme-accent);
}

.completion-row-label {
  @apply truncate;
  flex: 0 0 auto;
  max-width: 50%;
}

/* Inline parenthesised description, dim and same-line as the label.
 * `flex: 1 1 auto; min-width: 0` lets it absorb the slack between
 * label and source tag while truncating cleanly. */
.completion-row-detail {
  @apply truncate;
  flex: 1 1 auto;
  min-width: 0;
  color: var(--theme-fg-faint);
}

/* Right-aligned source tag (`[skill]`, `[path]`, `[ripgrep]`,
 * `[command]`). Dim + small + uppercase to read as an editor
 * affordance, not as part of the label. */
.completion-row-source {
  flex: 0 0 auto;
  margin-left: auto;
  padding-left: 8px;
  color: var(--theme-fg-faint);
  font-size: 0.66rem;
  letter-spacing: 0.3px;
  text-transform: uppercase;
}
</style>

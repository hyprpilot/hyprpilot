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
  <!-- Commit on `mousedown.prevent`, not `click`. The composer
       textarea's `@blur="completion.close()"` fires the instant a
       button anywhere else in the DOM takes focus — and click only
       fires AFTER mousedown completes. Without `.prevent` the
       sequence is: mousedown → button receives focus → textarea
       blurs → completion.close() → popover unmounts → click never
       fires (the row is gone). `.prevent` on mousedown blocks the
       button-focus default action, so the textarea keeps focus and
       the commit fires before any blur. Mouse + touch + pen all
       route through this path; pointer-event capture (`@pointerdown`)
       would catch them too, but `mousedown` is the broader-supported
       event and matches what the wider Vue ecosystem uses. -->
  <button type="button" class="completion-row" :data-active="active" @mouseenter="emit('hover')" @mousedown.prevent="emit('click')">
    <FaIcon :icon="icon" class="completion-row-icon" aria-hidden="true" />
    <span class="completion-row-label">{{ item.label }}</span>
    <span v-if="item.detail" class="completion-row-detail">({{ item.detail }})</span>
    <span class="completion-row-source">[{{ sourceLabel }}]</span>
  </button>
</template>

<style scoped>
@reference '../assets/styles.css';

.completion-row {
  @apply flex w-full items-center gap-2 overflow-hidden border-0 bg-transparent px-3 py-1 text-left;
  font-family: var(--theme-font-mono);
  font-size: 0.78rem;
  color: var(--theme-fg-subtle);
  cursor: pointer;
}

.completion-row:hover,
.completion-row[data-active='true'] {
  background-color: var(--theme-surface-alt);
  color: var(--theme-fg);
}

.completion-row-icon {
  width: 0.75rem;
  height: 0.75rem;
  color: var(--theme-fg-dim);
  flex-shrink: 0;
}

.completion-row[data-active='true'] .completion-row-icon {
  color: var(--theme-accent);
}

/* Label is the primary identifier — render at natural width and
 * never yield space to the description. When the label alone
 * exceeds the row, the parent's `overflow: hidden` clips it. */
.completion-row-label {
  flex: 0 0 auto;
}

/* Inline parenthesised description — packs directly after the label
 * and truncates to whatever width remains before the right-aligned
 * source tag. `flex: 0 1 auto + min-width: 0 + truncate` is the
 * truncate-only-shrink-only recipe; basis stays at content size so
 * a short description doesn't get stretched into the slack between
 * label and source. */
.completion-row-detail {
  @apply truncate;
  flex: 0 1 auto;
  min-width: 0;
  color: var(--theme-fg-faint);
}

/* Right-aligned source tag (`[skill]`, `[path]`, `[ripgrep]`,
 * `[command]`). Dim + small + uppercase to read as an editor
 * affordance, not as part of the label. */
.completion-row-source {
  flex: 0 0 auto;
  margin-left: auto;
  padding-left: 0.5rem;
  color: var(--theme-fg-faint);
  font-size: 0.66rem;
  letter-spacing: 0.01875rem;
  text-transform: uppercase;
}
</style>

<script setup lang="ts">
/**
 * Recursive palette overlay. Renders the top of `usePalette().stack` as
 * a floating centered panel with a search input, fuzzy-filtered row list,
 * and a capture-phase keyboard dispatcher. Port of the Python pilot's
 * `CommandPalette` (see `~/.dotfiles/wayland/.config/wayland/scripts/lib/
 * overlay.py`) — multi-select ticking, active-row pinning, and the
 * `Ctrl+D` delete hook are all preserved.
 *
 * Filter semantics: two-stage. (1) subsequence gate — every query char
 * must appear in the entry name in order, case-insensitive. (2) fuse.js
 * scores the survivors. Fuse alone is greedy enough that "gst" pulls in
 * haystacks that share only a partial substring; the gate cuts those.
 *
 * Intra-palette shortcuts are hardcoded on purpose (not driven by
 * `[keymaps.palette]`); the open shortcut lives on the parent (Chat.vue)
 * and reads from the config tree.
 */
import { FocusTrap } from 'focus-trap-vue'
import Fuse from 'fuse.js'
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'

import { type PaletteEntry, PaletteMode, type PaletteSpec, usePalette } from '@composables'

const { stack, close } = usePalette()

const top = computed<PaletteSpec | undefined>(() => stack.value[stack.value.length - 1])

const query = ref('')
const highlighted = ref(0)
const tickedIds = ref<Set<string>>(new Set())

const inputRef = ref<HTMLInputElement>()
// focus-trap-vue activates synchronously on prop flip; if we bind it
// directly to `top` the activation fires before the slot's input makes
// it into the DOM. Gate behind a nextTick so the child tree commits
// first, then arm the trap.
const trapActive = ref(false)

watch(
  top,
  (spec) => {
    tickedIds.value = new Set(spec?.preseedActive?.map((e) => e.id) ?? [])
    query.value = ''
    highlighted.value = 0
    if (spec) {
      void nextTick(() => {
        inputRef.value?.focus()
        trapActive.value = true
      })
    } else {
      trapActive.value = false
    }
  },
  { immediate: true }
)

function subsequenceMatch(q: string, name: string): boolean {
  if (!q) {
    return true
  }
  const needle = q.toLowerCase()
  const hay = name.toLowerCase()
  let pos = 0
  for (const ch of needle) {
    const next = hay.indexOf(ch, pos)
    if (next < 0) {
      return false
    }
    pos = next + 1
  }

  return true
}

const visibleEntries = computed<PaletteEntry[]>(() => {
  const spec = top.value
  if (!spec) {
    return []
  }
  const q = query.value.trim()
  const tickedSet = tickedIds.value
  const ticked = spec.entries.filter((e) => tickedSet.has(e.id))
  const rest = spec.entries.filter((e) => !tickedSet.has(e.id))
  const gated = rest.filter((e) => subsequenceMatch(q, e.name))

  let ordered: PaletteEntry[]
  if (!q) {
    ordered = gated
  } else {
    const fuse = new Fuse(gated, { keys: ['name'], threshold: 0.5, ignoreLocation: true })
    ordered = fuse.search(q).map((r) => r.item)
  }

  // Multi-select pins every ticked row to the top (matched or not); the
  // gated filter above already excluded them so they only appear once.
  if (spec.mode === PaletteMode.MultiSelect) {
    return [...ticked, ...ordered]
  }

  return ordered
})

watch(visibleEntries, (rows) => {
  if (rows.length === 0) {
    highlighted.value = 0

    return
  }
  if (highlighted.value >= rows.length) {
    highlighted.value = rows.length - 1
  }
  if (highlighted.value < 0) {
    highlighted.value = 0
  }
})

function onDocumentKeyDown(e: KeyboardEvent): void {
  const spec = top.value
  if (!spec) {
    return
  }

  // IME composition: swallow the synthetic keydown the browser fires for
  // each compose step so the palette doesn't treat an in-progress candidate
  // selection as navigation / commit input.
  if (e.isComposing || e.keyCode === 229) {
    return
  }

  const rows = visibleEntries.value
  const current = rows[highlighted.value]

  const key = e.key
  const ctrl = e.ctrlKey

  if (key === 'Escape') {
    e.preventDefault()
    e.stopPropagation()
    close()

    return
  }

  if (key === 'ArrowUp' || (ctrl && key.toLowerCase() === 'p')) {
    e.preventDefault()
    e.stopPropagation()
    if (rows.length > 0) {
      highlighted.value = (highlighted.value - 1 + rows.length) % rows.length
    }

    return
  }

  if (key === 'ArrowDown' || (ctrl && key.toLowerCase() === 'n')) {
    e.preventDefault()
    e.stopPropagation()
    if (rows.length > 0) {
      highlighted.value = (highlighted.value + 1) % rows.length
    }

    return
  }

  if (key === 'Tab' && spec.mode === PaletteMode.MultiSelect) {
    e.preventDefault()
    e.stopPropagation()
    if (current) {
      const next = new Set(tickedIds.value)
      if (next.has(current.id)) {
        next.delete(current.id)
      } else {
        next.add(current.id)
      }
      tickedIds.value = next
    }

    return
  }

  if (ctrl && key.toLowerCase() === 'd') {
    e.preventDefault()
    e.stopPropagation()
    if (current && spec.onDelete) {
      void spec.onDelete(current)
    }

    return
  }

  if (key === 'Enter') {
    e.preventDefault()
    e.stopPropagation()
    commit()
  }
}

function commit(): void {
  const spec = top.value
  if (!spec) {
    return
  }
  const rows = visibleEntries.value
  const current = rows[highlighted.value]

  let picks: PaletteEntry[]
  if (spec.mode === PaletteMode.MultiSelect) {
    const ticked = spec.entries.filter((e) => tickedIds.value.has(e.id))
    if (ticked.length > 0) {
      picks = ticked
    } else if (current) {
      picks = [current]
    } else {
      picks = []
    }
  } else {
    picks = current ? [current] : []
  }

  // Close before dispatching onCommit so a recursive `open()` in the
  // callback pushes onto a clean stack rather than stacking under the
  // just-committed spec.
  close()
  void spec.onCommit(picks)
}

function onRowClick(entry: PaletteEntry): void {
  const spec = top.value
  if (!spec) {
    return
  }
  const rows = visibleEntries.value
  const idx = rows.findIndex((r) => r.id === entry.id)
  if (idx < 0) {
    return
  }
  highlighted.value = idx
  // Close before dispatching onCommit so a recursive `open()` in the
  // callback pushes onto a clean stack rather than stacking under the
  // just-committed spec.
  close()
  void spec.onCommit([entry])
}

onMounted(() => {
  document.addEventListener('keydown', onDocumentKeyDown, { capture: true })
})

onUnmounted(() => {
  document.removeEventListener('keydown', onDocumentKeyDown, { capture: true })
})
</script>

<template>
  <FocusTrap v-if="top" :active="trapActive" :escape-deactivates="false" :allow-outside-click="true">
    <div class="palette-overlay" data-testid="palette-overlay">
      <div class="palette-frame" role="dialog" aria-modal="true" tabindex="0" :aria-label="top.title ?? 'palette'" data-testid="palette-frame">
        <header v-if="top.title" class="palette-title">{{ top.title }}</header>

        <div class="palette-query">
          <input
            ref="inputRef"
            v-model="query"
            type="text"
            class="palette-input"
            placeholder="search"
            spellcheck="false"
            autocomplete="off"
            autocapitalize="off"
            data-testid="palette-input"
          />
        </div>

        <ul class="palette-list" data-testid="palette-list">
          <li
            v-for="(entry, idx) in visibleEntries"
            :key="entry.id"
            class="palette-row"
            :data-selected="idx === highlighted"
            :data-ticked="tickedIds.has(entry.id)"
            :data-testid="`palette-row-${entry.id}`"
            @mouseenter="highlighted = idx"
            @click="onRowClick(entry)"
          >
            <span v-if="top.mode === PaletteMode.MultiSelect" class="palette-tick" aria-hidden="true">{{ tickedIds.has(entry.id) ? '✓' : '·' }}</span>
            <span class="palette-name">{{ entry.name }}</span>
            <span v-if="entry.kind" class="palette-kind">({{ entry.kind }})</span>
            <span v-if="entry.description" class="palette-description">{{ entry.description }}</span>
          </li>
          <li v-if="visibleEntries.length === 0" class="palette-empty">no matches</li>
        </ul>
      </div>
    </div>
  </FocusTrap>
</template>

<style scoped>
@reference '../../assets/styles.css';

.palette-overlay {
  @apply fixed inset-0 z-50 flex items-center justify-center;
  background-color: color-mix(in srgb, var(--theme-surface-bg) 60%, transparent);
}

.palette-frame {
  @apply flex w-full max-w-[38rem] flex-col border;
  max-height: 50vh;
  border-color: var(--theme-border-soft);
  background-color: var(--theme-surface-alt);
  color: var(--theme-fg);
  box-shadow: 0 12px 40px color-mix(in srgb, var(--theme-surface-bg) 70%, transparent);
}

.palette-title {
  @apply border-b px-3 py-[6px] text-[0.7rem] uppercase tracking-wider;
  border-color: var(--theme-border-soft);
  color: var(--theme-fg-dim);
  font-family: var(--theme-font-mono);
}

.palette-query {
  @apply border-b px-2 py-2;
  border-color: var(--theme-border-soft);
}

.palette-input {
  @apply w-full bg-transparent outline-none text-[0.8rem];
  color: var(--theme-fg);
  font-family: var(--theme-font-mono);
}

.palette-list {
  @apply m-0 flex min-h-0 flex-1 list-none flex-col overflow-y-auto p-0;
}

.palette-row {
  @apply flex items-baseline gap-2 px-3 py-[6px] text-[0.75rem];
  cursor: pointer;
  color: var(--theme-fg);
  font-family: var(--theme-font-mono);
}

.palette-row[data-selected='true'] {
  background-color: var(--theme-accent);
  color: var(--theme-fg);
}

.palette-tick {
  @apply shrink-0;
  color: var(--theme-fg-dim);
}

.palette-row[data-ticked='true'] .palette-tick {
  color: var(--theme-accent-user);
}

.palette-name {
  @apply shrink-0 font-bold;
}

.palette-kind {
  @apply shrink-0 text-[0.7rem];
  color: var(--theme-fg-dim);
}

.palette-description {
  @apply flex-1 truncate text-[0.7rem];
  color: var(--theme-fg-dim);
}

.palette-empty {
  @apply px-3 py-2 text-[0.72rem];
  color: var(--theme-fg-dim);
  font-family: var(--theme-font-mono);
}
</style>

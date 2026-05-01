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
import { faSquare as farSquare } from '@fortawesome/free-regular-svg-icons'
import {
  faArrowRightToBracket,
  faArrowTurnDown,
  faSquareCheck,
  faUpDown
} from '@fortawesome/free-solid-svg-icons'
import { FocusTrap } from 'focus-trap-vue'
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'

import { Loading } from '@components'
import {
  type PaletteEntry,
  PaletteMode,
  type PaletteSpec,
  useMultiSelect,
  usePalette,
  usePaletteFilter
} from '@composables'

const { stack, close } = usePalette()

const top = computed<PaletteSpec | undefined>(() => stack.value[stack.value.length - 1])

const query = ref('')
const highlighted = ref(0)
const { ticked: tickedIds, toggle: toggleTick } = useMultiSelect()
const { visible: visibleEntries } = usePaletteFilter(top, query, tickedIds)

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

const highlightedEntry = computed<PaletteEntry | undefined>(() => visibleEntries.value[highlighted.value])

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
      toggleTick(current.id)
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
  const liveQuery = query.value
  close()
  void spec.onCommit(picks, liveQuery)
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
  const liveQuery = query.value
  close()
  void spec.onCommit([entry], liveQuery)
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
      <div
        class="palette-frame"
        :data-wide="Boolean(top.preview)"
        :data-mode="top.mode"
        role="dialog"
        aria-modal="true"
        tabindex="0"
        :aria-label="top.title ?? 'palette'"
        data-testid="palette-frame"
      >
        <header class="palette-header">
          <span v-if="top.title" class="palette-title">{{ top.title }}</span>
          <span v-if="top.title" class="palette-arrow" aria-hidden="true">›</span>
          <input
            ref="inputRef"
            v-model="query"
            type="text"
            class="palette-input"
            placeholder=""
            spellcheck="false"
            autocomplete="off"
            autocapitalize="off"
            data-testid="palette-input"
          />
          <span class="palette-result-count">{{ visibleEntries.length }} result{{ visibleEntries.length === 1 ? '' : 's' }}</span>
        </header>

        <div class="palette-content">
          <ul class="palette-list" data-testid="palette-list">
            <li
              v-for="(entry, idx) in visibleEntries"
              :key="entry.id"
              class="palette-row"
              :data-selected="idx === highlighted"
              :data-ticked="tickedIds.has(entry.id)"
              :data-multi="top.mode === PaletteMode.MultiSelect"
              :data-testid="`palette-row-${entry.id}`"
              @mouseenter="highlighted = idx"
              @click="onRowClick(entry)"
            >
              <FaIcon
                v-if="top.mode === PaletteMode.MultiSelect"
                :icon="tickedIds.has(entry.id) ? faSquareCheck : farSquare"
                class="palette-tick"
                aria-hidden="true"
              />
              <span class="palette-name">{{ entry.name }}</span>
              <span v-if="entry.description" class="palette-description">{{ entry.description }}</span>
              <span v-if="entry.kind" class="palette-kind">{{ entry.kind }}</span>
            </li>
            <li v-if="visibleEntries.length === 0 && top.loading" class="palette-empty palette-empty-loading">
              <Loading mode="inline" :status="top.loadingStatus" />
            </li>
            <li v-else-if="visibleEntries.length === 0" class="palette-empty">no matches</li>
          </ul>

          <aside v-if="top.preview" class="palette-preview" data-testid="palette-preview">
            <component :is="top.preview.component" :entry="highlightedEntry" v-bind="top.preview.props ?? {}" />
          </aside>
        </div>

        <footer class="palette-footer">
          <span class="palette-kbd-hint">
            <kbd class="palette-kbd">
              <FaIcon :icon="faUpDown" class="palette-kbd-icon" aria-hidden="true" />
            </kbd>
            <span class="palette-kbd-label">navigate</span>
          </span>
          <span v-if="top.mode === PaletteMode.MultiSelect" class="palette-kbd-hint">
            <kbd class="palette-kbd">
              <FaIcon :icon="faArrowRightToBracket" class="palette-kbd-icon" aria-hidden="true" />
            </kbd>
            <span class="palette-kbd-label">toggle</span>
          </span>
          <span class="palette-kbd-hint">
            <kbd class="palette-kbd">
              <FaIcon :icon="faArrowTurnDown" class="palette-kbd-icon" aria-hidden="true" />
            </kbd>
            <span class="palette-kbd-label">confirm</span>
          </span>
          <span class="palette-kbd-hint">
            <kbd class="palette-kbd palette-kbd-text">Esc</kbd>
            <span class="palette-kbd-label">close</span>
          </span>
        </footer>
      </div>
    </div>
  </FocusTrap>
</template>

<style scoped>
@reference '../../assets/styles.css';

/* palette overlay: dimmed scrim, palette centered on the chat surface
 * (vertically + horizontally). Frame width is driven by `data-mode` /
 * `data-wide` below — the overlay just provides breathing room. */
.palette-overlay {
  @apply fixed inset-0 z-50 flex items-center justify-center;
  background-color: color-mix(in srgb, var(--theme-surface-bg) 60%, transparent);
  padding: 24px;
}

/* palette frame: surface bg + line2 border, 8px radius, big shadow.
 * Width is explicit per palette type so single / multi / preview
 * states render consistently rather than each filling the available
 * viewport. `max-width` clamps gracefully on narrow anchors. */
.palette-frame {
  @apply flex flex-col;
  max-height: 70vh;
  width: 32rem;
  max-width: calc(100vw - 48px);
  border: 1px solid var(--theme-border-soft);
  border-radius: 8px;
  background-color: var(--theme-surface);
  color: var(--theme-fg);
  box-shadow: 0 16px 48px rgba(0, 0, 0, 0.6);
  overflow: hidden;
}

.palette-frame[data-mode='multi-select'] {
  width: 36rem;
}

.palette-frame[data-wide='true'] {
  width: 56rem;
}

/* wireframe header: title › query (caret) ... result count. */
.palette-header {
  @apply flex items-center gap-2;
  padding: 10px 14px;
  border-bottom: 1px solid var(--theme-border);
  font-family: var(--theme-font-mono);
}

.palette-title {
  color: var(--theme-fg-dim);
  font-size: 0.7rem;
}

.palette-arrow {
  color: var(--theme-fg-dim);
  font-size: 0.75rem;
}

.palette-input {
  @apply flex-1 bg-transparent outline-none border-0 text-[0.7rem];
  color: var(--theme-fg);
  font-family: var(--theme-font-mono);
}

.palette-input::placeholder {
  color: var(--theme-fg-faint);
}

.palette-result-count {
  @apply shrink-0 text-[0.56rem];
  color: var(--theme-fg-dim);
}

.palette-content {
  @apply flex min-h-0 flex-1;
  overflow: hidden;
}

.palette-list {
  @apply m-0 flex min-h-0 flex-1 list-none flex-col overflow-y-auto p-[6px];
  min-width: 0;
}

.palette-frame[data-wide='true'] .palette-list {
  flex: 0 0 42%;
  border-right: 1px solid var(--theme-border);
}

/* wireframe row: 3px transparent left border (yellow on selected),
 * surface2 bg on selected, mono ink2 → fg on selected. */
.palette-row {
  @apply flex items-center gap-[10px] text-[0.7rem];
  cursor: pointer;
  padding: 6px 10px;
  border-radius: 4px;
  border-left: 3px solid transparent;
  color: var(--theme-fg-ink-2);
  font-family: var(--theme-font-mono);
  margin-bottom: 1px;
}

.palette-row[data-selected='true'] {
  background-color: var(--theme-surface-alt);
  border-left-color: var(--theme-accent);
  color: var(--theme-fg);
}

.palette-tick {
  @apply inline-flex shrink-0 items-center justify-center text-[0.7rem];
  width: 18px;
  text-align: center;
  color: var(--theme-fg-dim);
}

.palette-row[data-ticked='true'] .palette-tick {
  color: var(--theme-accent);
}

.palette-name {
  @apply shrink-0 font-bold;
}

.palette-row[data-selected='true'] .palette-name {
  color: var(--theme-fg);
}

.palette-description {
  @apply min-w-0 flex-1 truncate text-[0.62rem];
  color: var(--theme-fg-dim);
}

.palette-kind {
  @apply shrink-0 text-[0.56rem];
  color: var(--theme-fg-dim);
  font-family: var(--theme-font-mono);
}

.palette-empty {
  @apply text-[0.7rem];
  padding: 12px 16px;
  color: var(--theme-fg-dim);
  font-family: var(--theme-font-mono);
}

/* Loading variant — drop the inner padding so the <Loading
 * mode="inline"> component owns its own vertical spacing
 * (24px 16px). Without this, the wrapper's 12px padding stacks
 * on top of the component's, leaving the spinner floating in a
 * lopsided box. */
.palette-empty-loading {
  padding: 0;
}

.palette-preview {
  @apply flex min-h-0 flex-1 flex-col overflow-y-auto;
  padding: 12px 14px;
}

@media (max-width: 560px) {
  .palette-preview {
    display: none;
  }
}

/* wireframe footer: keyboard hints, mono dim, centered. */
.palette-footer {
  @apply flex items-center justify-center;
  padding: 8px 14px;
  border-top: 1px solid var(--theme-border);
  gap: 18px;
  font-family: var(--theme-font-mono);
}

.palette-kbd-hint {
  @apply inline-flex items-center;
  gap: 5px;
}

.palette-kbd {
  @apply inline-flex items-center justify-center text-[0.56rem];
  min-width: 18px;
  padding: 1px 6px;
  background-color: var(--theme-surface-bg);
  border: 1px solid var(--theme-border-soft);
  border-radius: 3px;
  color: var(--theme-fg);
  font-family: var(--theme-font-mono);
}

.palette-kbd-icon {
  width: 8px;
  height: 8px;
}

.palette-kbd-label {
  color: var(--theme-fg-dim);
  font-size: 0.56rem;
}
</style>

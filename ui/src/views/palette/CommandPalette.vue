<script setup lang="ts">
/**
 * Recursive palette overlay. Renders the top of `usePalette().stack` as
 * a floating centered panel with a search input, fuzzy-filtered row list,
 * and a capture-phase keyboard dispatcher. Port of the Python pilot's
 * `CommandPalette` (see `~/.dotfiles/wayland/.config/wayland/scripts/lib/
 * overlay.py`) — multi-select ticking, active-row pinning, and the
 * `Ctrl+D` delete hook are all preserved.
 *
 * Filter semantics: every keystroke debounces (60ms) into a daemon-side
 * `completion/rank` RPC. nucleo-matcher does the fuzzy ranking; the
 * UI just renders the order it gets back. Same matcher every other
 * surface uses (cwd recents, future Neovim plugin), so ranking stays
 * consistent across frontends.
 *
 * Intra-palette shortcuts are hardcoded on purpose (not driven by
 * `[keymaps.palette]`); the open shortcut lives on the parent (Chat.vue)
 * and reads from the config tree.
 */
import { faSquare as farSquare } from '@fortawesome/free-regular-svg-icons'
import { faArrowRightToBracket, faArrowTurnDown, faSquareCheck, faUpDown, faXmark } from '@fortawesome/free-solid-svg-icons'
import { FocusTrap } from 'focus-trap-vue'
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'

import { KbdHint, Loading } from '@components'
import { type PaletteEntry, PaletteMode, type PaletteSpec, useMultiSelect, usePalette, usePaletteFilter } from '@composables'

const { stack, close, registerFocusCallback } = usePalette()

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
        // On touch devices, skip auto-focus so the virtual keyboard
        // doesn't pop up on every palette open. The captain taps the
        // input explicitly when they want to type. Desktop / mouse
        // users keep the existing focus-on-open behaviour.
        if (typeof matchMedia !== 'undefined' && matchMedia('(pointer: fine)').matches) {
          inputRef.value?.focus()
        }
        trapActive.value = true
      })
    } else {
      trapActive.value = false
    }
  },
  { immediate: true }
)

// Live-query hook: leaves with dynamic entries (cwd path
// autocomplete, future ripgrep search, …) wire `onQueryChange` on
// their spec; we forward every keystroke alongside an `update()`
// callback that swaps the spec's `entries` ref so
// `usePaletteFilter` re-renders without a re-open.
watch(query, (q) => {
  const spec = top.value

  if (!spec?.onQueryChange) {
    return
  }
  spec.onQueryChange(q, (next) => {
    spec.entries = next
  })
})

const highlightedEntry = computed<PaletteEntry | undefined>(() => visibleEntries.value[highlighted.value])

// Per-row template refs — `:ref` callback in the v-for slot pushes
// each <li> into this map, keyed by entry id. Used by the watcher
// below to scroll the highlighted row into view as the captain
// arrows through the list. Cleared on every render via the callback's
// `null` branch.
const rowRefs = new Map<string, HTMLLIElement>()

function bindRow(entry: PaletteEntry): (_el: unknown) => void {
  return (el) => {
    if (el === null) {
      rowRefs.delete(entry.id)
    } else {
      rowRefs.set(entry.id, el as HTMLLIElement)
    }
  }
}

// Keep the highlighted row in view as the captain arrows up/down.
// `block: 'nearest'` only scrolls when the row is actually offscreen;
// ignores the row when it's already visible so mouse-driven hover
// doesn't shake the scroll position.
watch(highlighted, () => {
  void nextTick(() => {
    const entry = highlightedEntry.value

    if (!entry) {
      return
    }
    // jsdom doesn't implement `scrollIntoView` — feature-check the
    // method (not just the element) so vitest doesn't surface six
    // "scrollIntoView is not a function" unhandled errors per run.
    rowRefs.get(entry.id)?.scrollIntoView?.({ block: 'nearest' })
  })
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

/// Tab in Input mode: pull the highlighted row's text into the query
/// so the captain can keep typing past the suggestion (descend into a
/// dir, keep refining). The row's `description` carries the canonical
/// text the leaf wants in the buffer (cwd palette → path). Commit
/// stays on Enter.
///
/// `nextTick` + explicit `inputRef.focus()` defends against the
/// `<FocusTrap>` race — the trap sometimes shifts focus to the
/// palette frame on Tab even with `preventDefault` + `stopPropagation`
/// at capture phase. Refocusing after Vue's reactive re-render is the
/// safety net so the captain can keep typing into the input.
function autocompleteIntoQuery(current: PaletteEntry | undefined): void {
  if (current?.description !== undefined && current.description.length > 0) {
    query.value = current.description
    void nextTick(() => {
      const el = inputRef.value

      if (!el) {
        return
      }
      el.focus()
      const len = query.value.length

      el.setSelectionRange(len, len)
    })
  }
}

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
    onTickHighlighted()

    return
  }

  if (key === 'Tab' && spec.mode === PaletteMode.Input) {
    e.preventDefault()
    e.stopPropagation()
    onAutocompleteHighlighted()

    return
  }

  if (ctrl && key.toLowerCase() === 'd') {
    e.preventDefault()
    e.stopPropagation()
    onDeleteHighlighted()

    return
  }

  if (key === 'Enter') {
    e.preventDefault()
    e.stopPropagation()
    commit()
  }
}

function onTickHighlighted(): void {
  const current = visibleEntries.value[highlighted.value]

  if (current) {
    toggleTick(current.id)
  }
}

function onAutocompleteHighlighted(): void {
  autocompleteIntoQuery(visibleEntries.value[highlighted.value])
}

function onDeleteHighlighted(): void {
  const spec = top.value
  const current = visibleEntries.value[highlighted.value]

  if (!spec || !current || !spec.onDelete) {
    return
  }
  // `update` is the only path that surfaces entry mutations through
  // the reactive proxy on `top.value`. Assigning to `.entries` on the
  // raw spec literal skips the proxy and leaves rows stale.
  void spec.onDelete(current, (next) => {
    spec.entries = next
  })
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

  // MultiSelect: a row click toggles its tick (mirrors the Tab
  // keybind). Closing on click would force the captain to commit
  // every individual change to the trust store one round-trip at a
  // time — the whole point of multi-select is batching.
  if (spec.mode === PaletteMode.MultiSelect) {
    toggleTick(entry.id)

    return
  }
  // Close before dispatching onCommit so a recursive `open()` in the
  // callback pushes onto a clean stack rather than stacking under the
  // just-committed spec.
  const liveQuery = query.value

  close()
  void spec.onCommit([entry], liveQuery)
}

onMounted(() => {
  document.addEventListener('keydown', onDocumentKeyDown, { capture: true })
  // Hook the global Ctrl+F handler into our search input. The
  // composable holds a single callback because the palette is a
  // singleton — only one CommandPalette mounts at a time.
  registerFocusCallback(() => {
    inputRef.value?.focus()
    inputRef.value?.select()
  })
})

onUnmounted(() => {
  document.removeEventListener('keydown', onDocumentKeyDown, { capture: true })
  registerFocusCallback(undefined)
})
</script>

<template>
  <FocusTrap v-if="top" :active="trapActive" :escape-deactivates="false" :allow-outside-click="true">
    <div class="palette-overlay" data-testid="palette-overlay" @click.self="close">
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
            :placeholder="top.placeholder ?? ''"
            spellcheck="false"
            autocomplete="off"
            autocapitalize="off"
            autocorrect="off"
            inputmode="search"
            enterkeyhint="go"
            data-testid="palette-input"
          />
          <span class="palette-result-count">{{ visibleEntries.length }} result{{ visibleEntries.length === 1 ? '' : 's' }}</span>
          <!-- Touch-only close button. Desktop hits Esc; mobile users
               can't reach the keyboard's Esc without losing the
               textarea focus, and the dim outside-scrim doesn't read
               as a tap target on a full-screen palette. -->
          <button type="button" class="palette-close" aria-label="close" data-testid="palette-close" @click="close">
            <FaIcon :icon="faXmark" class="palette-close-icon" />
          </button>
        </header>

        <div class="palette-content">
          <ul class="palette-list" data-testid="palette-list">
            <li
              v-for="(entry, idx) in visibleEntries"
              :ref="bindRow(entry)"
              :key="entry.id"
              class="palette-row"
              :data-selected="idx === highlighted"
              :data-active="entry.active === true || entry.kind === 'active'"
              :data-ticked="tickedIds.has(entry.id)"
              :data-multi="top.mode === PaletteMode.MultiSelect"
              :data-testid="`palette-row-${entry.id}`"
              @mouseenter="highlighted = idx"
              @click="onRowClick(entry)"
            >
              <FaIcon v-if="top.mode === PaletteMode.MultiSelect" :icon="tickedIds.has(entry.id) ? faSquareCheck : farSquare" class="palette-tick" aria-hidden="true" />
              <span class="palette-name">{{ entry.name }}</span>
              <span v-if="entry.description" class="palette-description">{{ entry.description }}</span>
              <span v-if="entry.kind" class="palette-kind">{{ entry.kind }}</span>
            </li>
            <li v-if="visibleEntries.length === 0 && top.loading" class="palette-empty palette-empty-loading">
              <Loading mode="inline" :status="top.status" />
            </li>
            <!-- Input mode suppresses the "no matches" empty-state:
                 captain types and an empty list just means "no
                 autocomplete suggestion yet, Enter still commits the
                 typed value". Other modes treat empty as a real
                 zero-result signal. -->
            <li v-else-if="visibleEntries.length === 0 && top.mode !== PaletteMode.Input" class="palette-empty">no matches</li>
          </ul>

          <aside v-if="top.preview" class="palette-preview" data-testid="palette-preview">
            <component :is="top.preview.component" :entry="highlightedEntry" v-bind="top.preview.props ?? {}" />
          </aside>
        </div>

        <footer class="palette-footer">
          <KbdHint :keys="[faUpDown]" label="navigate" />
          <KbdHint v-if="top.mode === PaletteMode.MultiSelect" :keys="[faArrowRightToBracket]" label="toggle" :on-activate="onTickHighlighted" />
          <KbdHint v-if="top.mode === PaletteMode.Input" :keys="[faArrowRightToBracket]" label="autocomplete" :on-activate="onAutocompleteHighlighted" />
          <KbdHint :keys="[faArrowTurnDown]" label="confirm" :on-activate="commit" />
          <KbdHint v-if="top.onDelete" :keys="['Ctrl+D']" label="delete" :on-activate="onDeleteHighlighted" />
          <KbdHint class="palette-footer-esc" :keys="['Esc']" label="close" :on-activate="close" />
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
  padding: 1.5rem;
}

/* palette frame: surface bg + line2 border, 8px radius, big shadow.
 * Width is explicit per palette type so single / multi / preview
 * states render consistently rather than each filling the available
 * viewport. `max-width` clamps gracefully on narrow anchors. */
.palette-frame {
  @apply flex flex-col;
  /* Fixed height (not max-height) so the palette doesn't resize
   * as rows of varying heights or preview content scroll in/out
   * — moving frame chrome is jarring while you're scanning rows. */
  height: 70dvh;
  max-height: 32rem;
  width: 32rem;
  max-width: calc(100vw - 3rem);
  border: 1px solid var(--theme-border-soft);
  border-radius: 0.5rem;
  background-color: var(--theme-surface);
  color: var(--theme-fg);
  box-shadow: 0 1rem 3rem rgba(0, 0, 0, 0.6);
  overflow: hidden;
}

.palette-frame[data-mode='multi-select'] {
  width: 36rem;
}

.palette-frame[data-wide='true'] {
  width: 56rem;
}

/* Header: title › query (caret) ... result count. */
.palette-header {
  @apply flex items-center gap-2;
  padding: 0.625rem 0.875rem;
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
  @apply min-w-0 flex-1 bg-transparent outline-none border-0 text-[0.7rem];
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
  @apply m-0 flex min-h-0 flex-1 list-none flex-col overflow-y-auto p-[0.375rem];
  min-width: 0;
}

.palette-frame[data-wide='true'] .palette-list {
  flex: 0 0 42%;
  border-right: 1px solid var(--theme-border);
}

/* Two independent accent indicators on opposite sides, painted via
 * inset `box-shadow` rather than `border-left` / `border-right`:
 *   - LEFT shadow: navigation cursor (`data-selected`, moves with
 *     arrows / hover).
 *   - RIGHT shadow: persistent active row (`data-active`, the
 *     captain's currently-picked profile / model / cwd / instance).
 * When a row is both, both shadows stack.
 *
 * Why box-shadow over `border-*`: a colored border with the row's
 * `border-radius` leaks a tiny triangle of the border color at the
 * rounded corners — the border tapers to follow the radius, but the
 * tapered region paints in the border color, leaving 1-2 px of
 * accent peeking through where the rounded `background-color` ends.
 * Inset box-shadows respect `border-radius` cleanly, so the accent
 * stripes start AND end flush with the rounded edge. */
.palette-row {
  @apply flex items-center gap-[0.625rem] text-[0.7rem];
  cursor: pointer;
  /* Gutter reserved for the L/R accent stripes — keeps row content
   * from shifting horizontally when an accent lights up. */
  padding: 0.375rem calc(0.625rem + 0.1875rem);
  border-radius: 0.25rem;
  color: var(--theme-fg-subtle);
  font-family: var(--theme-font-mono);
  margin-bottom: 1px;
}

.palette-row[data-active='true'] {
  box-shadow: inset -0.1875rem 0 0 0 var(--theme-accent);
}

.palette-row[data-selected='true'] {
  background-color: var(--theme-surface-alt);
  box-shadow: inset 0.1875rem 0 0 0 var(--theme-accent);
  color: var(--theme-fg);
}

/* Both states at once — stack the shadows so each edge lights up
 * independently (otherwise the later selector overrides the earlier
 * `box-shadow` entirely). */
.palette-row[data-selected='true'][data-active='true'] {
  box-shadow:
    inset 0.1875rem 0 0 0 var(--theme-accent),
    inset -0.1875rem 0 0 0 var(--theme-accent);
}

.palette-tick {
  @apply inline-flex shrink-0 items-center justify-center text-[0.7rem];
  width: 1.125rem;
  text-align: center;
  color: var(--theme-fg-dim);
}

.palette-row[data-ticked='true'] .palette-tick {
  color: var(--theme-accent);
}

/* `palette-name` shrinks before `palette-description` because the
 * name is the row's identity and stays meaningful when ellipsised;
 * description is secondary. Both yield before the trailing
 * `palette-kind` chip, which is `shrink-0`. */
.palette-name {
  @apply min-w-0 truncate font-bold;
  flex: 0 1 auto;
  max-width: 60%;
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
  padding: 0.75rem 1rem;
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
  padding: 0.75rem 0.875rem;
}

/* Mobile / narrow palettes: stack the preview UNDER the results
 * (instead of hiding it) so detail-rich palettes — sessions,
 * MCPs, instances — keep their preview content reachable. The
 * `data-wide='true'` palette grows from 32rem to 56rem on
 * desktop; on phones the frame is viewport-bounded, so we just
 * flip `.palette-content` from row to column and let both panes
 * share the height. */
@media (max-width: 560px) {
  .palette-content {
    flex-direction: column;
  }

  .palette-frame[data-wide='true'] .palette-list {
    flex: 0 0 50%;
    border-right: none;
    border-bottom: 1px solid var(--theme-border);
  }

  .palette-preview {
    flex: 1 1 50%;
  }
}

/* Footer: keyboard hints, mono dim, centered. Wraps on narrow widths
 * so 5-6 hints don't punch out of phone viewports. */
.palette-footer {
  @apply flex flex-wrap items-center justify-center;
  padding: 0.5rem 0.875rem;
  border-top: 1px solid var(--theme-border);
  gap: 0.5rem 1.125rem;
  font-family: var(--theme-font-mono);
}

/* Esc is keyboard-only — touch users tap the backdrop / scrim to
 * dismiss the palette. The KbdHint chip would clutter the footer
 * without adding capability. */
@media (pointer: coarse) {
  .palette-footer-esc {
    display: none;
  }
}

/* Header close button (the X chip). Hidden on desktop — Esc is the
 * keyboard shortcut + the dim outside-scrim is the click target. On
 * touch the close button is the only ergonomic dismiss path (the
 * full-screen mobile palette below has no scrim to tap, and a phone
 * keyboard rarely surfaces Esc). */
.palette-close {
  @apply hidden shrink-0 items-center justify-center border-0 bg-transparent leading-none;
  color: var(--theme-fg-dim);
  cursor: pointer;
  padding: 0;
  font-family: var(--theme-font-mono);
}

.palette-close:hover {
  color: var(--theme-status-err);
}

.palette-close-icon {
  width: 0.75rem;
  height: 0.75rem;
}

@media (pointer: coarse) {
  .palette-close {
    @apply inline-flex;
    min-width: 2.25rem;
    min-height: 2.25rem;
  }

  .palette-close-icon {
    width: 0.875rem;
    height: 0.875rem;
  }

  /* Compact keyboard hints on touch. Desktop kept them visible /
   * readable; on mobile the captain reaches for taps over keyboard
   * shortcuts, so each hint's label costs more vertical space than
   * it pays back. Drop the label text on coarse pointers, keep the
   * icon-only keycap as a tap target. Combined with the header
   * close button + scroll-friendly full-screen frame, the captain
   * sees more content. The full label still reads aloud for
   * screen-readers via the underlying button's `aria-label`
   * substitute (label still in the DOM, hidden visually only). */
  .palette-footer {
    padding: 0.25rem 0.5rem;
    gap: 0.25rem 0.5rem;
  }
}

/* Phone / very-narrow viewports: drop the centered card and fill
 * the viewport. The captain reads the palette like a primary view
 * (sessions, instances, mcps) and a half-screen modal forces
 * pinching to see the content. Scrim collapses to zero so the
 * frame anchors flush; the new in-header close button replaces the
 * scrim-tap dismiss path. 32rem (~512px) matches the same
 * phone-portrait breakpoint the Frame chrome uses. */
@media (max-width: 32rem) {
  .palette-overlay {
    padding: 0;
    background-color: var(--theme-surface-bg);
  }

  .palette-frame,
  .palette-frame[data-mode='multi-select'],
  .palette-frame[data-wide='true'] {
    height: 100dvh;
    max-height: none;
    width: 100vw;
    max-width: none;
    border: none;
    border-radius: 0;
    box-shadow: none;
  }
}
</style>

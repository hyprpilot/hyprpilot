<script setup lang="ts">
import { faArrowTurnDown, faSortUp, faXmark } from '@fortawesome/free-solid-svg-icons'
import { computed, nextTick, ref, watch } from 'vue'

import { CompletionDocs, CompletionRow } from '@components'
import { useCompletion } from '@composables'

/**
 * Composer autocomplete popover — caret-anchored, palette-styled,
 * VS Code row layout (icon + label + detail). Two-column when the
 * active item has resolved documentation.
 *
 * Position is computed by the host (`Composer.vue`) via the
 * `caret-position` helper and passed in as `top` / `left` props.
 * Popover renders at `position: fixed` (z-index 40) so it escapes
 * any composer-stacking-context concerns.
 */
const props = defineProps<{
  /** Pixel offset from the viewport top. `null` when anchoring via `bottom`. */
  top: number | null
  /** Pixel offset from the viewport bottom. `null` when anchoring via `top`. */
  bottom: number | null
  left: number
}>()

const emit = defineEmits<{
  commit: []
}>()

const completion = useCompletion()
const state = completion.state

// Anchor from top OR bottom — never both. When the popover flips
// above the caret (no room below), `bottom` is set so the rendered
// box's bottom edge lines up with the caret regardless of row count.
const popoverStyle = computed<Record<string, string>>(() => {
  const style: Record<string, string> = { left: `${props.left}px` }

  if (props.top !== null) {
    style.top = `${props.top}px`
  }

  if (props.bottom !== null) {
    style.bottom = `${props.bottom}px`
  }

  return style
})

const showDocs = computed<boolean>(() => Boolean(state.value.documentation && state.value.documentation.trim().length > 0))

const listRef = ref<HTMLUListElement>()

// Keep the highlighted row visible inside the scrollable list. Pure
// browser-native scrollIntoView with `nearest` block — no smooth
// animation since the popover is a fast-keypress surface; smooth
// scrolling lags the cursor.
watch(
  () => state.value.selectedIndex,
  async(idx) => {
    await nextTick()
    const list = listRef.value

    if (!list) {
      return
    }
    const row = list.children.item(idx) as HTMLElement | null

    row?.scrollIntoView({ block: 'nearest' })
  }
)
</script>

<template>
  <Teleport to="body">
    <div v-if="state.open && state.items.length > 0" class="completion-popover-wrap" :class="{ 'completion-popover-wrap-flipped': props.bottom !== null }" :style="popoverStyle">
      <div class="completion-popover">
        <ul ref="listRef" class="completion-list" role="listbox" aria-label="Completion suggestions">
          <li v-for="(item, idx) in state.items" :key="`${item.kind}-${item.label}-${idx}`">
            <CompletionRow :item="item" :active="idx === state.selectedIndex" @hover="state.selectedIndex = idx" @click="emit('commit')" />
          </li>
        </ul>
        <footer class="completion-footer">
          <span class="completion-footer-hint">
            <FaIcon :icon="faSortUp" class="completion-footer-icon-up" aria-hidden="true" />
            <FaIcon :icon="faSortUp" class="completion-footer-icon-down" aria-hidden="true" />
          </span>
          <span class="completion-footer-label">navigate</span>
          <span class="completion-footer-hint">
            <FaIcon :icon="faArrowTurnDown" aria-hidden="true" />
          </span>
          <span class="completion-footer-label">commit</span>
          <span class="completion-footer-hint">
            <FaIcon :icon="faXmark" aria-hidden="true" />
          </span>
          <span class="completion-footer-label">close</span>
        </footer>
      </div>
      <CompletionDocs v-if="showDocs && state.documentation" :documentation="state.documentation" />
    </div>
  </Teleport>
</template>

<style scoped>
@reference '../../assets/styles.css';

.completion-popover-wrap {
  @apply flex items-start gap-2;
  position: fixed;
  z-index: 40;
}

/* Flipped above the caret: align children to the wrap's bottom so
 * the popover (often shorter than the docs panel) still sits flush
 * against the caret line, instead of floating high while the
 * taller docs panel hangs down. */
.completion-popover-wrap-flipped {
  @apply items-end;
}

.completion-popover {
  @apply flex flex-col;
  width: 360px;
  max-height: 240px;
  background-color: var(--theme-surface);
  border: 1px solid var(--theme-border);
  border-radius: 4px;
  box-shadow: 0 4px 12px rgb(0 0 0 / 0.4);
  overflow: hidden;
}

.completion-list {
  @apply flex flex-col overflow-y-auto;
  margin: 0;
  padding: 4px 0;
  list-style: none;
  flex: 1 1 auto;
}

.completion-footer {
  @apply flex items-center gap-1 border-t px-3 py-1;
  border-color: var(--theme-border);
  background-color: var(--theme-surface-bg);
  font-family: var(--theme-font-mono);
  font-size: 0.62rem;
  color: var(--theme-fg-dim);
  letter-spacing: 0.5px;
}

.completion-footer-hint {
  @apply inline-flex items-center justify-center gap-px;
  min-width: 16px;
  padding: 0 4px;
  color: var(--theme-fg);
  background-color: var(--theme-surface-alt);
  border-radius: 3px;
  font-weight: 600;
  font-size: 0.55rem;
}

/* Stack the two faSortUp glyphs as a paired up-arrow + down-arrow
 * cluster — FontAwesome free-solid has no native two-direction
 * vertical-arrow icon, and stacking faSortUp + a rotated copy reads
 * as the navigation affordance without a custom SVG. */
.completion-footer-icon-down {
  transform: rotate(180deg);
}

.completion-footer-label {
  margin-right: 8px;
}
</style>

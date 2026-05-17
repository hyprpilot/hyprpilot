<script setup lang="ts">
import { isFaIcon, type KeyLabel } from '@components'

/**
 * Keyboard-hint chip. Keycaps are either plain text (Ctrl, Esc,
 * Ctrl+K) or directly-imported FontAwesome `IconDefinition`s rendered
 * as glyphs — the latter catches keys where system fonts give
 * inconsistent unicode (↑↓ ⏎ ⎋ ⇥).
 *
 * `size = 'sm'` (default) is the palette-footer chip size; `'md'` is
 * the focal-point variant for hero hints (idle-screen "Ctrl+K command
 * palette." prompt). Both consume the same theme tokens — the only
 * difference is type scale + keycap padding.
 *
 * When `onActivate` is provided, the hint renders as a `<button>` so
 * touch / mobile users can tap it as a real action — keyboard users
 * still trigger the same handler via the underlying keybind. Without
 * it the hint is purely informational.
 */
withDefaults(
  defineProps<{
    keys: KeyLabel[]
    label: string
    size?: 'sm' | 'md'
    /** Tap handler — when set, the hint becomes a tap target. */
    onActivate?: () => void
  }>(),
  {
    size: 'sm',
    onActivate: undefined
  }
)
</script>

<template>
  <button v-if="onActivate" type="button" class="kbd-hint kbd-hint-clickable" :data-size="size" @click="onActivate">
    <kbd v-for="(k, i) in keys" :key="i" class="kbd-hint-key">
      <FaIcon v-if="isFaIcon(k)" :icon="k" class="kbd-hint-key-icon" />
      <template v-else>{{ k }}</template>
    </kbd>
    <span class="kbd-hint-label">{{ label }}</span>
  </button>
  <span v-else class="kbd-hint" :data-size="size">
    <kbd v-for="(k, i) in keys" :key="i" class="kbd-hint-key">
      <FaIcon v-if="isFaIcon(k)" :icon="k" class="kbd-hint-key-icon" />
      <template v-else>{{ k }}</template>
    </kbd>
    <span class="kbd-hint-label">{{ label }}</span>
  </span>
</template>

<style scoped>
@reference '../assets/styles.css';

.kbd-hint {
  @apply inline-flex items-center gap-1 leading-tight;
  color: var(--theme-fg-dim);
  font-family: var(--theme-font-mono);
}

.kbd-hint-clickable {
  @apply cursor-pointer border-0 bg-transparent;
  padding: 0.25rem 0.375rem;
  border-radius: 0.1875rem;
  transition: background-color 0.12s ease-out;
}

.kbd-hint-clickable:hover,
.kbd-hint-clickable:focus-visible {
  background-color: var(--theme-surface-alt);
  outline: none;
}

.kbd-hint-clickable:active {
  filter: brightness(0.85);
}

.kbd-hint[data-size='sm'] {
  @apply text-[0.7rem];
}

.kbd-hint[data-size='md'] {
  @apply gap-2 text-[0.85rem];
}

.kbd-hint-key {
  @apply inline-flex min-w-4 items-center justify-center border;
  color: var(--theme-fg-subtle);
  background-color: var(--theme-surface-alt);
  border-color: var(--theme-border);
}

.kbd-hint[data-size='sm'] .kbd-hint-key {
  @apply px-[4px] py-[1px] text-[0.68rem];
}

.kbd-hint[data-size='md'] .kbd-hint-key {
  @apply px-[6px] py-[2px] text-[0.82rem];
}

/* Touch-friendly hit target on coarse pointers (phones / tablets).
 * Desktop keeps the compact chrome — keyboard users don't need a
 * tappable row; the hint is just a label there.
 *
 * The original sizing (min-height 2.75rem + 0.5rem padding) hit the
 * WCAG 2.5.5 enhanced target, but the captain reported it dominated
 * the palette footer on phones — three or four hint chips ate the
 * better part of the screen and pushed the result list above the
 * fold. Dropping to 2rem min + 0.375rem padding keeps the chip well
 * inside the WCAG 2.5.5 minimum (24x24 CSS px ≈ 1.5rem) while
 * recovering ~30% of the footer's vertical real-estate. The chip's
 * keycap glyph is still the dominant visual; the label trails as
 * a faint mono caption. */
@media (pointer: coarse) {
  .kbd-hint-clickable {
    min-height: 2rem;
    padding: 0.25rem 0.5rem;
  }
}

.kbd-hint-key-icon {
  width: 0.5625rem;
  height: 0.5625rem;
}

.kbd-hint[data-size='md'] .kbd-hint-key-icon {
  width: 0.6875rem;
  height: 0.6875rem;
}

.kbd-hint-label {
  color: var(--theme-fg-dim);
}
</style>

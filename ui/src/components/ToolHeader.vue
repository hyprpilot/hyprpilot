<script setup lang="ts">
import type { IconDefinition } from '@fortawesome/fontawesome-svg-core'
import { computed } from 'vue'

import { ToastTone } from '@components'
import { toneBg } from '@constants/ui'

/**
 * Shared header chrome for the three tool surfaces (ToolPill in chat,
 * PermissionRow in the composer, PermissionModal via `Modal.vue`).
 * Renders an icon + title; when `tone` is set the icon+title pair
 * sits inside a tone-colored tag (the warm-brown "permission" pill).
 * Trailing slot accepts the surface-specific accessory (the pill's
 * caret, the row's `<PermissionActions>`).
 *
 * Pinning the chrome here means a future reskin of the warn tag
 * propagates to every consumer in one edit instead of three.
 */

const props = withDefaults(
  defineProps<{
    icon?: IconDefinition
    title: string
    /// Tag tone — `undefined` renders plain icon + text (pill default);
    /// any `ToastTone` value wraps the icon+title in a tone-colored
    /// tag (row + modal).
    tone?: ToastTone
  }>(),
  {
    tone: undefined,
    icon: undefined
  }
)

const tagBg = computed(() => (props.tone === undefined ? undefined : toneBg(props.tone)))
</script>

<template>
  <header class="tool-header" :data-toned="tone !== undefined">
    <span v-if="tone !== undefined" class="tool-header-tag" :style="{ backgroundColor: tagBg }">
      <FaIcon v-if="icon" :icon="icon" class="tool-header-icon" aria-hidden="true" />
      <span class="tool-header-title">{{ title }}</span>
    </span>
    <template v-else>
      <FaIcon v-if="icon" :icon="icon" class="tool-header-icon" aria-hidden="true" />
      <span class="tool-header-title">{{ title }}</span>
    </template>
    <slot name="trailing" />
  </header>
</template>

<style scoped>
@reference '../assets/styles.css';

.tool-header {
  @apply flex items-center;
  gap: 0.5rem;
  min-width: 0;
  font-family: var(--theme-font-mono);
}

/* Title column grows to fill available space and yields when the
 * trailing slot's controls (caret, action buttons, dismiss) need
 * room. Without `min-width: 0` on the flex item, a long unbroken
 * tool path refuses to shrink and pushes the slot offscreen. */
.tool-header-tag {
  @apply inline-flex items-center;
  flex: 1 1 auto;
  min-width: 0;
  gap: 0.3125rem;
  padding: 0.125rem 0.4375rem;
  color: var(--theme-fg-on-tone);
  border-radius: 0.1875rem;
  font-size: 0.62rem;
  font-weight: 700;
  letter-spacing: 0.01875rem;
}

.tool-header-icon {
  flex-shrink: 0;
  width: 0.5625rem;
  height: 0.5625rem;
}

.tool-header-title {
  flex: 1 1 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-weight: 700;
}

.tool-header[data-toned='false'] .tool-header-title {
  font-weight: 600;
}

.tool-header :slotted(*) {
  flex-shrink: 0;
}
</style>

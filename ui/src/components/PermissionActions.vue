<script setup lang="ts">
import { Button, ButtonTone, ButtonVariant } from '@components'
import type { PermissionOptionView } from '@interfaces/wire/transcript'

/**
 * Action button row shared by `PermissionRow` and `PermissionModal`.
 * Buttons share allotted width via `flex: 1 1 0; min-width: 0` so a
 * long agent-supplied label can't crowd a short one — they all expand
 * equally and ellipsis-truncate at their bounds. Single-button case
 * keeps intrinsic width (a lone "Cancel" stretching across the whole
 * modal looks oversized).
 *
 * Tone: `allow_*` → ok, `reject_*` → err, anything else → neutral.
 * Variant: `allow_once` is solid (the agent's default + most common
 * pick), every other kind renders ghost so the primary action is
 * visually unambiguous.
 *
 * Emits `reply` with the real `optionId` from the offered set.
 */
defineProps<{
  options: PermissionOptionView[]
}>()

const emit = defineEmits<{
  reply: [optionId: string]
}>()

function toneFor(opt: PermissionOptionView): ButtonTone {
  if (opt.kind.startsWith('allow')) {
    return ButtonTone.Ok
  }

  if (opt.kind.startsWith('reject')) {
    return ButtonTone.Err
  }

  return ButtonTone.Neutral
}

function variantFor(opt: PermissionOptionView): ButtonVariant {
  return opt.kind === 'allow_once' ? ButtonVariant.Solid : ButtonVariant.Ghost
}
</script>

<template>
  <div class="permission-actions" :data-single="options.length === 1">
    <Button
      v-for="opt in options"
      :key="opt.optionId"
      class="permission-actions-btn"
      :tone="toneFor(opt)"
      :variant="variantFor(opt)"
      :title="opt.name"
      :aria-label="opt.name"
      @click="emit('reply', opt.optionId)"
    ><span class="permission-actions-label">{{ opt.name }}</span></Button>
  </div>
</template>

<style scoped>
@reference '../assets/styles.css';

.permission-actions {
  /* Take the full width of the parent slot so children's `flex: 1`
   * shares of `width` resolve against a non-zero number. Without an
   * explicit width here, an `inline-flex` parent (or one whose width
   * collapses to children) would give each child its intrinsic
   * width and the row drifts back to the long-label-wins shape. */
  @apply flex w-full min-w-0 items-center;
  flex: 1 1 100%;
  gap: 6px;
}

.permission-actions-btn {
  @apply justify-start;
  flex: 1 1 0;
  min-width: 0;
  max-width: 100%;
  /* Constrain the button to its flex share + clip overflowing label
   * text. The actual ellipsis happens on the inner label span (an
   * `<inline-flex>` button doesn't apply `text-overflow` to its
   * direct text node reliably across engines). */
  overflow: hidden;
}

.permission-actions-label {
  display: inline-block;
  min-width: 0;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.permission-actions[data-single='true'] .permission-actions-btn {
  flex: 0 0 auto;
}
</style>

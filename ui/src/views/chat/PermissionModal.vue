<script setup lang="ts">
import { computed } from 'vue'

import ToolBody from './ToolBody.vue'
import { Button, ButtonTone, ButtonVariant, Modal, ToastTone } from '@components'
import type { PermissionView } from '@components'

/**
 * Modal-class permission UI — pop-up dialog the chat surface opens
 * for permissions whose formatter declared `permissionUi: Modal`
 * (Edit / Delete / Move per ACP `kind`, plan-exit, future
 * heavy-confirm flows).
 *
 * Body composes the formatter's `description` (markdown — diff
 * payload for edits, plan body for plan-exit) above the structured
 * fields / output. Action row renders a button per agent-offered
 * `PermissionOption`. No icons; labels are the agent's `name` field
 * verbatim (vendors stuff rule context here — claude-code ships
 * `Always allow Bash(curl -sSo /dev/null ...)` so the captain sees
 * the literal pattern the persistent rule will store; re-casing
 * strips that detail). Tone derived from the option's typed `kind`:
 *
 * - `allow_*` → ok tone (`allow_once` solid, others ghost).
 * - `reject_*` → err tone.
 * - anything else (forward-compat) → neutral.
 *
 * Long names are capped via `max-width` + ellipsis; the full string
 * survives on the button's `title` for hover.
 *
 * Emits `reply` with the real `optionId` from the offered set.
 */
const props = defineProps<{
  view: PermissionView
}>()

const emit = defineEmits<{
  reply: [optionId: string]
  dismiss: []
}>()

interface ButtonView {
  optionId: string
  label: string
  tone: ButtonTone
  variant: ButtonVariant
}

const buttons = computed<ButtonView[]>(() =>
  props.view.options.map((opt) => {
    const tone = opt.kind.startsWith('allow') ? ButtonTone.Ok : opt.kind.startsWith('reject') ? ButtonTone.Err : ButtonTone.Neutral
    // Solid fill on `allow_once` (the agent-default + most common
    // pick); every other variant renders ghost so the primary action
    // is unambiguous.
    const variant = opt.kind === 'allow_once' ? ButtonVariant.Solid : ButtonVariant.Ghost

    return {
      optionId: opt.optionId,
      label: opt.name,
      tone,
      variant
    }
  })
)
</script>

<template>
  <Modal :title="view.call.title" :tone="ToastTone.Warn" :icon="view.call.icon" :dismissable="false" @dismiss="emit('dismiss')">
    <template #actions>
      <Button
        v-for="b in buttons"
        :key="b.optionId"
        class="permission-modal-btn"
        :tone="b.tone"
        :variant="b.variant"
        :title="b.label"
        :aria-label="b.label"
        @click="emit('reply', b.optionId)"
      >{{ b.label }}</Button>
    </template>
    <ToolBody :view="view.call" />
  </Modal>
</template>

<style scoped>
@reference '../../assets/styles.css';

/* Vendors stuff rule-context into the option's `name`; cap the
 * button's visible width so a long pattern doesn't blow out the
 * modal's action row. The full string lives on `title` for hover. */
.permission-modal-btn {
  max-width: 28ch;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>

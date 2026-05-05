<script setup lang="ts">
import ToolBody from './ToolBody.vue'
import { Modal, PermissionActions, ToastTone } from '@components'
import type { PermissionView } from '@components'

/**
 * Modal-class permission UI — pop-up dialog the chat surface opens
 * for permissions whose formatter declared `permissionUi: Modal`
 * (Edit / Delete / Move per ACP `kind`, plan-exit, future
 * heavy-confirm flows).
 *
 * Body is the formatter's `<ToolBody>` (description + fields + output).
 * Action row delegates to the shared `<PermissionActions>` so the
 * modal + the inline `PermissionRow` button rows look + behave
 * identically.
 *
 * Emits `reply` with the real `optionId` from the offered set.
 */
defineProps<{
  view: PermissionView
}>()

const emit = defineEmits<{
  reply: [optionId: string]
  dismiss: []
}>()
</script>

<template>
  <Modal :title="view.call.title" :tone="ToastTone.Warn" :icon="view.call.icon" :dismissable="false" @dismiss="emit('dismiss')">
    <template #actions>
      <PermissionActions class="permission-modal-actions" :options="view.options" @reply="(id) => emit('reply', id)" />
    </template>
    <ToolBody :view="view.call" />
  </Modal>
</template>

<style scoped>
@reference '../../assets/styles.css';

/* Modal's `.modal-actions` slot is `inline-flex` by default; widen
 * it so the shared `<PermissionActions>` can flex-share the
 * available footer space. */
.permission-modal-actions {
  flex: 1 1 100%;
  width: 100%;
}
</style>

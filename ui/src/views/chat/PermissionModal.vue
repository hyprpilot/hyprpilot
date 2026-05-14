<script setup lang="ts">
import { computed, ref } from 'vue'

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
 * identically. A reject-feedback textarea sits between body + actions
 * — captains supply a "why" the daemon dispatches as a synthetic
 * follow-up turn so the agent reads the rejection reason on its next
 * turn. Optional; empty / whitespace-only commits the same way as
 * before (no feedback, no follow-up).
 *
 * Emits `reply` with the real `optionId` from the offered set, plus
 * the current feedback string when set. Parent forwards both to
 * `permissions/respond` / `permission_reply`.
 */
const props = defineProps<{
  view: PermissionView
}>()

const emit = defineEmits<{
  reply: [optionId: string, feedback?: string]
  dismiss: []
}>()

const feedback = ref('')

/// Only forward the feedback when it's non-empty after trim AND the
/// picked option is reject-kind. Daemon's reject-only gate would
/// drop it on allow anyway; the UI mirrors that so the
/// `acp:transcript` we generate for the follow-up turn doesn't get
/// confused with the captain's allow path.
function onReply(optionId: string): void {
  const trimmed = feedback.value.trim()
  const picked = props.view.options.find((o) => o.optionId === optionId)
  const isReject = picked?.kind.startsWith('reject') ?? false

  if (trimmed.length > 0 && isReject) {
    emit('reply', optionId, trimmed)
  } else {
    emit('reply', optionId)
  }
  feedback.value = ''
}

const hasRejectOption = computed(() => props.view.options.some((o) => o.kind.startsWith('reject')))
</script>

<template>
  <Modal :title="view.call.title" :tone="ToastTone.Warn" :icon="view.call.icon" :dismissable="false" @dismiss="emit('dismiss')">
    <template #actions>
      <PermissionActions class="permission-modal-actions" :options="view.options" :default-option-id="view.defaultOptionId" @reply="onReply" />
    </template>
    <ToolBody :view="view.call" />
    <div v-if="hasRejectOption" class="permission-modal-feedback">
      <label class="permission-modal-feedback-label" for="permission-modal-feedback-input">
        feedback on reject <span class="permission-modal-feedback-hint">(optional — sent to agent as a follow-up turn)</span>
      </label>
      <textarea
        id="permission-modal-feedback-input"
        v-model="feedback"
        class="permission-modal-feedback-input"
        rows="2"
        placeholder="explain why — agent reads this if you reject"
      />
    </div>
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

.permission-modal-feedback {
  @apply flex flex-col;
  gap: 0.25rem;
  margin-top: 0.625rem;
  padding-top: 0.5rem;
  border-top: 1px dashed var(--theme-border-soft);
}

.permission-modal-feedback-label {
  @apply leading-tight;
  font-size: 0.65rem;
  color: var(--theme-fg-dim);
  font-family: var(--theme-font-mono);
}

.permission-modal-feedback-hint {
  color: var(--theme-fg-subtle);
}

.permission-modal-feedback-input {
  @apply w-full resize-none rounded-sm;
  font-size: 0.7rem;
  font-family: var(--theme-font-sans);
  padding: 0.375rem 0.5rem;
  color: var(--theme-fg);
  background-color: var(--theme-surface-bg);
  border: 1px solid var(--theme-border-soft);
}

.permission-modal-feedback-input:focus {
  outline: none;
  border-color: var(--theme-accent);
}
</style>

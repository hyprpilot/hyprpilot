<script setup lang="ts">
import { computed } from 'vue'

import ToolSpecSheet from './ToolSpecSheet.vue'
import { Button, ButtonTone, ButtonVariant, MarkdownBody, Modal, ToastTone } from '@components'
import type { PermissionView } from '@components'

/**
 * Modal-class permission UI — pop-up dialog the chat surface opens
 * for permissions whose formatter declared
 * `permissionUi: Modal` (plan-exit, future heavy-confirm flows).
 *
 * Body composes the formatter's `description` (markdown — the plan
 * body for plan-exit) above the structured fields / output. Action
 * row carries accept / reject buttons; emit `reply` with the
 * `optionId` the wire option array carries. Today only synthetic
 * `'allow'` / `'deny'` shortcuts are surfaced; future ACP option
 * sets (named choices) slot in by listing `view.options` here.
 */
const props = defineProps<{
  view: PermissionView
}>()

const emit = defineEmits<{
  reply: [optionId: 'allow' | 'deny']
  dismiss: []
}>()

const description = computed(() => props.view.call.description)
const hasSpec = computed(() => Boolean(props.view.call.output) || (props.view.call.fields !== undefined && props.view.call.fields.length > 0))
</script>

<template>
  <Modal :title="view.call.title" :tone="ToastTone.Warn" :icon="view.call.icon" :dismissable="false" @dismiss="emit('dismiss')">
    <template #actions>
      <Button :tone="ButtonTone.Err" @click="emit('reply', 'deny')">reject</Button>
      <Button :tone="ButtonTone.Ok" :variant="ButtonVariant.Solid" @click="emit('reply', 'allow')">accept</Button>
    </template>
    <MarkdownBody v-if="description" :source="description" />
    <ToolSpecSheet v-if="hasSpec" :output="view.call.output" :fields="view.call.fields" />
  </Modal>
</template>

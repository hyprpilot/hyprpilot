<script setup lang="ts">
import { faMobileScreenButton } from '@fortawesome/free-solid-svg-icons'
import { computed, ref, watch } from 'vue'

import { Button, ButtonTone, ButtonVariant, Modal, ModalDescription, ModalInput, ToastTone } from '@components'
import { type RemotePairState, useRemotePair } from '@composables'

/**
 * Remote pair-confirm modal. Auto-opens on every `remote:pair-request`
 * Tauri event the daemon emits — captain reads the 4-word BIP39 code
 * off the connecting phone's screen and types it here. On match the
 * daemon upgrades the pending WS to authenticated; on mismatch the
 * input flags the error and the WS stays pending until expiry / reject.
 *
 * Camera-scan input is intentionally deferred — typed-code is the
 * minimum viable path. A `@zxing/browser` integration slots in via
 * a "scan" button in the actions row when the captain wants it.
 */
defineProps<{
  state: RemotePairState
}>()

const emit = defineEmits<{
  /** Captain typed the right code → daemon upgraded the WS. */
  confirmed: []
  /** Captain dismissed → daemon-side reject. */
  rejected: []
}>()

const draft = ref('')
const submitting = ref(false)
const lastError = ref<string | undefined>(undefined)

watch(
  () => draft.value,
  () => {
    lastError.value = undefined
  }
)

const validate = computed<(_raw: string) => string | null>(() => (raw) => {
  if (raw.trim().length === 0) {
    return null
  }
  const words = raw.trim().split(/\s+/u).filter(Boolean)

  if (words.length !== 4) {
    return 'expecting 4 words'
  }

  return null
})

async function onAccept(): Promise<void> {
  if (submitting.value) {
    return
  }
  const err = validate.value(draft.value)

  if (err !== null) {
    lastError.value = err

    return
  }
  submitting.value = true

  try {
    const ok = await useRemotePair().confirm(draft.value)

    if (ok) {
      emit('confirmed')
    } else {
      lastError.value = 'code does not match'
    }
  } catch(err) {
    lastError.value = String(err)
  } finally {
    submitting.value = false
  }
}

async function onReject(): Promise<void> {
  await useRemotePair().reject()
  emit('rejected')
}
</script>

<template>
  <Modal title="remote · pair request" :tone="ToastTone.Warn" :icon="faMobileScreenButton" :dismissable="false" @dismiss="onReject">
    <template #actions>
      <Button :tone="ButtonTone.Err" @click="onReject">reject</Button>
      <Button :tone="ButtonTone.Ok" :variant="ButtonVariant.Solid" :disabled="submitting" @click="onAccept">confirm</Button>
    </template>

    <ModalDescription>
      A device at <code>{{ state.remoteAddr }}</code> is requesting access. Read the 4-word code off
      the connecting device and type it below to confirm. Re-pair on every reconnect; no tokens persist.
    </ModalDescription>

    <div class="pair-words">
      <span v-for="(word, i) in state.words" :key="`w-${i}-${word}`" class="pair-word">{{ word }}</span>
    </div>

    <ModalInput v-model:value="draft" placeholder="four words separated by spaces" :validate="validate.value" @submit="onAccept" />

    <p v-if="lastError" class="pair-error">{{ lastError }}</p>
  </Modal>
</template>

<style scoped>
@reference '../assets/styles.css';

.pair-words {
  @apply flex flex-wrap items-center;
  gap: 6px;
  margin: 10px 0 12px;
  padding: 8px 10px;
  background-color: var(--theme-surface-alt);
  border: 1px dashed var(--theme-border-soft);
  border-radius: 3px;
  font-family: var(--theme-font-mono);
}

.pair-word {
  padding: 3px 8px;
  background-color: var(--theme-surface);
  color: var(--theme-fg);
  border: 1px solid var(--theme-border);
  border-radius: 3px;
  font-size: 0.82rem;
  font-weight: 600;
  letter-spacing: 0.4px;
}

.pair-error {
  margin-top: 6px;
  color: var(--theme-status-err);
  font-family: var(--theme-font-mono);
  font-size: 0.7rem;
}
</style>

<script setup lang="ts">
import { onMounted, ref } from 'vue'

import PermissionPrompt from '@components/PermissionPrompt.vue'
import { useAcpAgent } from '@composables'
import { Button } from '@ui/button'

const { transcript, state, lastPermission, bind, submit, cancel } = useAcpAgent()

const draft = ref('')
const sending = ref(false)
const lastErr = ref<string>()

onMounted(() => {
  bind().catch((err) => {
    lastErr.value = `bind failed: ${String(err)}`
  })
})

async function onSubmit() {
  if (!draft.value.trim() || sending.value) return
  sending.value = true
  lastErr.value = undefined
  try {
    await submit({ text: draft.value })
    draft.value = ''
  } catch (err) {
    lastErr.value = String(err)
  } finally {
    sending.value = false
  }
}

async function onCancel() {
  try {
    await cancel()
  } catch (err) {
    lastErr.value = String(err)
  }
}
</script>

<template>
  <section class="overlay-card placeholder" data-testid="placeholder">
    <header class="placeholder-header">
      <span class="placeholder-dot" :class="`placeholder-dot-${state?.state ?? 'idle'}`" aria-hidden="true" />
      <h1 class="placeholder-title">hyprpilot</h1>
      <span v-if="state" class="placeholder-state">{{ state.state }}</span>
    </header>

    <form class="placeholder-form" @submit.prevent="onSubmit">
      <textarea
        v-model="draft"
        class="placeholder-textarea"
        rows="3"
        placeholder="type a prompt…"
        data-testid="placeholder-textarea"
      />

      <div class="placeholder-actions">
        <Button
          type="submit"
          variant="accent"
          :disabled="sending || draft.trim().length === 0"
          data-testid="placeholder-submit"
        >
          {{ sending ? 'sending…' : 'submit' }}
        </Button>
        <Button type="button" variant="muted" data-testid="placeholder-cancel" @click="onCancel"> cancel </Button>
      </div>
    </form>

    <p v-if="lastErr" class="placeholder-err">{{ lastErr }}</p>

    <section v-if="transcript.length > 0" class="placeholder-transcript" data-testid="placeholder-transcript">
      <h2 class="placeholder-transcript-title">transcript</h2>
      <pre
        v-for="(chunk, idx) in transcript"
        :key="idx"
        class="placeholder-transcript-chunk"
      >{{ JSON.stringify(chunk.update, null, 2) }}</pre>
    </section>

    <PermissionPrompt :request="lastPermission" />
  </section>
</template>

<style scoped>
@reference "../assets/styles.css";

.placeholder {
  @apply mx-auto my-12 flex max-w-[34rem] flex-col gap-3 border px-6 py-5;
  background-color: var(--theme-surface-card-user);
  color: var(--theme-fg);
  border-color: var(--theme-border-soft);
  border-left: 2px solid var(--theme-window-edge);
}

.placeholder-header {
  @apply flex items-center gap-2;
}

.placeholder-dot {
  @apply h-2 w-2 rounded-full;
  background-color: var(--theme-state-idle);
}

.placeholder-dot-starting,
.placeholder-dot-running {
  background-color: var(--theme-state-stream);
}

.placeholder-dot-error {
  background-color: var(--theme-accent);
}

.placeholder-title {
  @apply flex-1 text-[1.05rem] font-bold tracking-wider;
  font-family: var(--theme-font-family);
  color: var(--theme-fg);
}

.placeholder-state {
  @apply text-[0.8rem];
  font-family: var(--theme-font-family);
  color: var(--theme-fg-muted);
}

.placeholder-form {
  @apply flex flex-col gap-2;
}

.placeholder-textarea {
  @apply w-full resize-y border px-3 py-2 text-[0.9rem];
  background-color: var(--theme-surface-compose);
  color: var(--theme-fg);
  border-color: var(--theme-border-soft);
  font-family: var(--theme-font-family);

  &:focus {
    outline: none;
    border-color: var(--theme-border-focus);
  }
}

.placeholder-actions {
  @apply flex gap-2;
}

.placeholder-err {
  @apply text-[0.85rem];
  color: var(--theme-accent);
}

.placeholder-transcript {
  @apply flex flex-col gap-1 border px-3 py-2;
  background-color: var(--theme-surface-card-assistant);
  border-color: var(--theme-border-soft);
}

.placeholder-transcript-title {
  @apply text-[0.85rem] font-bold tracking-wider;
  color: var(--theme-fg-dim);
}

.placeholder-transcript-chunk {
  @apply m-0 whitespace-pre-wrap text-[0.75rem];
  font-family: var(--theme-font-family);
  color: var(--theme-fg-muted);
}
</style>

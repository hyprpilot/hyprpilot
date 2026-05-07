<script setup lang="ts">
/**
 * Mobile / remote-host pair landing. Renders before the normal overlay
 * boots when the SPA is loaded over the daemon's HTTPS bridge — the
 * phone's first WS upgrade is `pending` until the captain confirms on
 * the desktop. Shows the 4-word BIP39 code as text (read-aloud path)
 * AND as a QR (scan-from-desktop path).
 *
 * Once the daemon sends `{type:"authenticated"}`, the parent flips off
 * `pending` and the regular boot continues. This screen is consumer-side
 * only — it doesn't drive any IPC; just displays state from the
 * pair-frame subscription.
 */
import { faLock, faRotate } from '@fortawesome/free-solid-svg-icons'
import QRCode from 'qrcode'
import { computed, onMounted, ref, watch } from 'vue'

const props = defineProps<{
  code: string
  expiresInSeconds: number
}>()

const qrDataUrl = ref<string | undefined>(undefined)
const qrError = ref<string | undefined>(undefined)

const words = computed<string[]>(() => props.code.trim().split(/\s+/u).filter(Boolean))

async function regenerate(code: string): Promise<void> {
  qrError.value = undefined

  try {
    qrDataUrl.value = await QRCode.toDataURL(code, {
      errorCorrectionLevel: 'M',
      margin: 1,
      scale: 8
    })
  } catch(err) {
    qrError.value = String(err)
  }
}

onMounted(() => {
  void regenerate(props.code)
})

watch(
  () => props.code,
  (next) => {
    void regenerate(next)
  }
)
</script>

<template>
  <main class="pair-screen">
    <header class="pair-screen-header">
      <FaIcon :icon="faLock" class="pair-screen-header-icon" aria-hidden="true" />
      <span class="pair-screen-header-label">hyprpilot · pair</span>
    </header>

    <section class="pair-screen-body">
      <p class="pair-screen-prompt">Show this screen to the desktop. Either scan the QR or read out the four words.</p>

      <div v-if="qrDataUrl" class="pair-qr-frame">
        <img :src="qrDataUrl" class="pair-qr" alt="pair-code QR" />
      </div>
      <p v-else-if="qrError" class="pair-qr-error">QR render failed: {{ qrError }}</p>

      <ul class="pair-words" aria-label="pair code">
        <li v-for="(word, i) in words" :key="`w-${i}-${word}`" class="pair-word">{{ word }}</li>
      </ul>

      <p class="pair-screen-status">
        <FaIcon :icon="faRotate" class="pair-screen-status-icon" aria-hidden="true" spin />
        waiting for desktop · expires in {{ expiresInSeconds }}s
      </p>
    </section>
  </main>
</template>

<style scoped>
@reference '../assets/styles.css';

.pair-screen {
  display: flex;
  flex-direction: column;
  align-items: stretch;
  justify-content: flex-start;
  gap: 18px;
  height: 100%;
  padding: 28px 22px 32px;
  background-color: var(--theme-surface-bg);
  color: var(--theme-fg);
  font-family: var(--theme-font-mono);
}

.pair-screen-header {
  @apply flex items-center;
  gap: 8px;
  padding-bottom: 6px;
  border-bottom: 1px solid var(--theme-border-soft);
  font-size: 0.85rem;
  letter-spacing: 0.4px;
  color: var(--theme-fg-dim);
}

.pair-screen-header-icon {
  width: 12px;
  height: 12px;
  color: var(--theme-status-warn);
}

.pair-screen-body {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 16px;
}

.pair-screen-prompt {
  text-align: center;
  font-size: 0.85rem;
  color: var(--theme-fg-subtle);
  max-width: 32ch;
  line-height: 1.4;
}

.pair-qr-frame {
  padding: 12px;
  background-color: #ffffff;
  border-radius: 6px;
  border: 1px solid var(--theme-border);
}

.pair-qr {
  display: block;
  width: 220px;
  height: 220px;
  image-rendering: pixelated;
}

.pair-qr-error {
  color: var(--theme-status-err);
  font-size: 0.75rem;
}

.pair-words {
  @apply flex flex-wrap items-center justify-center;
  gap: 8px;
  margin: 0;
  padding: 0;
  list-style: none;
}

.pair-word {
  padding: 5px 10px;
  background-color: var(--theme-surface);
  color: var(--theme-fg);
  border: 1px solid var(--theme-border);
  border-radius: 3px;
  font-size: 0.95rem;
  font-weight: 600;
  letter-spacing: 0.4px;
}

.pair-screen-status {
  @apply flex items-center;
  gap: 8px;
  margin-top: 4px;
  color: var(--theme-fg-dim);
  font-size: 0.75rem;
}

.pair-screen-status-icon {
  width: 10px;
  height: 10px;
}
</style>

<script setup lang="ts">
import { faCamera, faMobileScreenButton, faXmark } from '@fortawesome/free-solid-svg-icons'
import QrScanner from 'qr-scanner'
import QRCode from 'qrcode'
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'

import { Button, ButtonTone, ButtonVariant, Modal, ModalDescription, ModalInput, ToastTone } from '@components'
import { type RemotePairState, useRemotePair } from '@composables'
import { log } from '@lib'

/**
 * Remote pair-confirm modal. Auto-opens on every `remote:pair-request`
 * Tauri event the daemon emits — captain reads the 4-word BIP39 code
 * off the connecting phone's screen and types it here. On match the
 * daemon upgrades the pending WS to authenticated; on mismatch the
 * input flags the error and the WS stays pending until expiry / reject.
 *
 * Two confirm paths:
 *   1. Type the four words manually.
 *   2. Click `scan` → the laptop webcam reads the QR off the phone's
 *      pair screen and the decoded code auto-fills the input. The
 *      captain still hits confirm to commit; the scan only fills.
 */
const props = defineProps<{
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

const scanning = ref(false)
const scanError = ref<string | undefined>(undefined)
const videoEl = ref<HTMLVideoElement | undefined>(undefined)
let scanner: QrScanner | undefined

// Render the same QR the connecting device shows. Side-by-side
// visual confirmation: if the two QRs (or the two word lists)
// don't match, the captain rejects without thinking.
const qrDataUrl = ref<string | undefined>(undefined)
const qrError = ref<string | undefined>(undefined)

async function regenerateQr(code: string): Promise<void> {
  qrError.value = undefined

  try {
    qrDataUrl.value = await QRCode.toDataURL(code, {
      errorCorrectionLevel: 'M',
      margin: 1,
      scale: 6
    })
  } catch(err) {
    qrError.value = String(err)
  }
}

onMounted(() => {
  void regenerateQr(props.state.code)
})

watch(
  () => props.state.code,
  (next) => {
    void regenerateQr(next)
  }
)

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
  stopScan()
  await useRemotePair().reject()
  emit('rejected')
}

async function startScan(): Promise<void> {
  scanError.value = undefined

  // Browser API check — `getUserMedia` requires HTTPS or localhost.
  // In dev preview / non-secure contexts the API is undefined; guard
  // to surface a readable error instead of throwing.
  if (typeof navigator === 'undefined' || !navigator.mediaDevices?.getUserMedia) {
    scanError.value = 'camera unavailable in this context'

    return
  }

  scanning.value = true
  // Wait one tick for the <video> element to mount.
  await new Promise<void>((resolve) => setTimeout(resolve, 0))

  const target = videoEl.value

  if (!target) {
    scanError.value = 'video element failed to mount'
    scanning.value = false

    return
  }

  try {
    scanner = new QrScanner(target, (result) => {
      const decoded = result.data.trim()

      log.info('remote: QR scanned', { length: decoded.length })
      draft.value = decoded
      stopScan()
    }, {
      preferredCamera: 'environment',
      highlightScanRegion: true,
      highlightCodeOutline: true,
      maxScansPerSecond: 5
    })
    await scanner.start()
  } catch(err) {
    scanError.value = String(err)
    stopScan()
  }
}

function stopScan(): void {
  if (scanner) {
    scanner.stop()
    scanner.destroy()
    scanner = undefined
  }
  scanning.value = false
}

onBeforeUnmount(() => {
  stopScan()
})
</script>

<template>
  <Modal title="remote · pair request" :tone="ToastTone.Warn" :icon="faMobileScreenButton" :dismissable="false" @dismiss="onReject">
    <template #actions>
      <Button v-if="!scanning" :tone="ButtonTone.Neutral" @click="startScan">
        <FaIcon :icon="faCamera" /> scan
      </Button>
      <Button v-else :tone="ButtonTone.Neutral" @click="stopScan">
        <FaIcon :icon="faXmark" /> stop
      </Button>
      <Button :tone="ButtonTone.Err" @click="onReject">reject</Button>
      <Button :tone="ButtonTone.Ok" :variant="ButtonVariant.Solid" :disabled="submitting" @click="onAccept">confirm</Button>
    </template>

    <ModalDescription>
      A device at <code>{{ state.remoteAddr }}</code> is requesting access. Compare the QR / words below
      against the connecting device — if they match, type the code or click <strong>scan</strong> to
      read it from the webcam. Re-pair on every reconnect; no tokens persist.
    </ModalDescription>

    <div v-if="scanning" class="pair-scan-frame">
      <video ref="videoEl" class="pair-scan-video" muted playsinline autoplay></video>
    </div>
    <p v-if="scanError" class="pair-error">{{ scanError }}</p>

    <div v-if="!scanning" class="pair-display">
      <div v-if="qrDataUrl" class="pair-qr-frame">
        <img :src="qrDataUrl" class="pair-qr" alt="pair-code QR" />
      </div>
      <p v-else-if="qrError" class="pair-qr-error">QR render failed: {{ qrError }}</p>

      <div class="pair-words">
        <span v-for="(word, i) in state.words" :key="`w-${i}-${word}`" class="pair-word">{{ word }}</span>
      </div>
    </div>

    <ModalInput v-model:value="draft" placeholder="four words separated by spaces" :validate="validate.value" @submit="onAccept" />

    <p v-if="lastError" class="pair-error">{{ lastError }}</p>
  </Modal>
</template>

<style scoped>
@reference '../assets/styles.css';

.pair-display {
  @apply flex items-center;
  gap: 14px;
  margin: 10px 0 12px;
  padding: 10px 12px;
  background-color: var(--theme-surface-alt);
  border: 1px dashed var(--theme-border-soft);
  border-radius: 3px;
}

.pair-qr-frame {
  flex-shrink: 0;
  padding: 8px;
  background-color: #ffffff;
  border-radius: 4px;
  border: 1px solid var(--theme-border);
}

.pair-qr {
  display: block;
  width: 132px;
  height: 132px;
  image-rendering: pixelated;
}

.pair-qr-error {
  flex-shrink: 0;
  color: var(--theme-status-err);
  font-family: var(--theme-font-mono);
  font-size: 0.7rem;
}

.pair-words {
  @apply flex flex-wrap items-center;
  gap: 6px;
  flex: 1 1 auto;
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

.pair-scan-frame {
  margin: 10px 0;
  padding: 6px;
  background-color: var(--theme-surface-alt);
  border: 1px solid var(--theme-border);
  border-radius: 3px;
  display: flex;
  justify-content: center;
}

.pair-scan-video {
  width: 100%;
  max-width: 320px;
  aspect-ratio: 1 / 1;
  background-color: #000;
  object-fit: cover;
  border-radius: 3px;
}

.pair-error {
  margin-top: 6px;
  color: var(--theme-status-err);
  font-family: var(--theme-font-mono);
  font-size: 0.7rem;
}
</style>

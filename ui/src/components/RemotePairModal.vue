<script setup lang="ts">
import { faCamera, faMobileScreenButton, faXmark } from '@fortawesome/free-solid-svg-icons'
import QrScanner from 'qr-scanner'
import QRCode from 'qrcode'
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'

import { Button, ButtonTone, ButtonVariant, Modal, ModalDescription, ModalInput, ToastTone } from '@components'
import { type KeymapEntry, type RemotePairState, useKeymap, useKeymaps, useRemotePair } from '@composables'
import { log } from '@lib'

/**
 * Remote pair-confirm modal. Auto-opens on every `remote:pair-request`
 * Tauri event the daemon emits. Asymmetric pair codes — this side
 * renders `desktopCode` (its identity) as QR + readable words and
 * expects to receive `deviceCode` (only visible on the connecting
 * device) typed or scanned to confirm.
 *
 * Two confirm paths:
 *   1. Type the four words from the connecting device's screen.
 *   2. Click `scan` → laptop webcam reads the device's QR (which
 *      encodes `deviceCode`); on a successful decode the modal
 *      *auto-confirms* — no extra click. Captain pointing the
 *      camera at the device IS the act of trust.
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

// Render this side's own code (`desktopCode`) as a QR for the
// connecting device to scan. The captain compares the four words
// and the QR against what's shown on the device — if either
// doesn't match, reject.
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
  void regenerateQr(props.state.desktopCode)
})

watch(
  () => props.state.desktopCode,
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

async function commitConfirm(code: string): Promise<void> {
  if (submitting.value) {
    return
  }
  submitting.value = true

  try {
    const ok = await useRemotePair().confirm(code)

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

async function onAccept(): Promise<void> {
  const err = validate.value(draft.value)

  if (err !== null) {
    lastError.value = err

    return
  }
  await commitConfirm(draft.value)
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

      log.info('remote: QR scanned (desktop) → auto-confirming', { length: decoded.length })
      draft.value = decoded
      stopScan()
      // Scanning the device's QR IS the act of confirmation —
      // no extra click. Captain pointed the camera at the device,
      // that's the proof-of-presence the pairing is checking.
      void commitConfirm(decoded)
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

// Approval keybinds — same chord the captain uses for tool-permission
// prompts (`approvals.allow` = Ctrl+G, `approvals.deny` = Ctrl+R by
// default). Pair confirm + reject are the same shape of "act on this
// pending request"; binding the same chord keeps the muscle memory
// consistent. The Overlay-level permission handler also listens on
// these chords but bails when no permission is queued, so the two
// paths don't fight in the common case (pair modal up, no pending
// permissions).
const { keymaps } = useKeymaps()

useKeymap(
  () => document,
  (): KeymapEntry[] => {
    if (!keymaps.value) {
      return []
    }

    return [
      {
        binding: keymaps.value.approvals.allow,
        handler: () => {
          // Skip when the draft is empty — confirming an empty code
          // is just a guaranteed mismatch round-trip with no captain
          // intent. Useful flow: scan auto-fills, captain hits the
          // chord to commit.
          if (draft.value.trim().length === 0) {
            return false
          }
          void onAccept()

          return true
        }
      },
      {
        binding: keymaps.value.approvals.deny,
        handler: () => {
          void onReject()

          return true
        }
      }
    ]
  }
)
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
      A device at <code>{{ state.remoteAddr }}</code> is requesting access. Show the QR / words below to
      that device (it can also scan this QR with its camera), or click <strong>scan</strong> to read
      the device's QR from this webcam — confirms automatically. Re-pair on every reconnect; no tokens persist.
    </ModalDescription>

    <div v-if="scanning" class="pair-scan-frame">
      <video ref="videoEl" class="pair-scan-video" muted playsinline autoplay></video>
    </div>
    <p v-if="scanError" class="pair-error">{{ scanError }}</p>

    <div v-if="!scanning" class="pair-display">
      <div v-if="qrDataUrl" class="pair-qr-frame">
        <img :src="qrDataUrl" class="pair-qr" alt="desktop pair-code QR" />
      </div>
      <p v-else-if="qrError" class="pair-qr-error">QR render failed: {{ qrError }}</p>

      <div class="pair-words">
        <span v-for="(word, i) in state.desktopWords" :key="`w-${i}-${word}`" class="pair-word">{{ word }}</span>
      </div>
    </div>

    <p class="pair-input-hint">type the code shown on the connecting device:</p>
    <ModalInput v-model:value="draft" placeholder="four words separated by spaces" :validate="validate" @submit="onAccept" />

    <p v-if="lastError" class="pair-error">{{ lastError }}</p>
  </Modal>
</template>

<style scoped>
@reference '../assets/styles.css';

.pair-display {
  @apply flex items-center;
  gap: 0.875rem;
  margin: 0.625rem 0 0.75rem;
  padding: 0.625rem 0.75rem;
  background-color: var(--theme-surface-alt);
  border: 1px dashed var(--theme-border-soft);
  border-radius: 0.1875rem;
}

.pair-qr-frame {
  flex-shrink: 0;
  padding: 0.5rem;
  background-color: #ffffff;
  border-radius: 0.25rem;
  border: 1px solid var(--theme-border);
}

.pair-qr {
  display: block;
  width: 8.25rem;
  height: 8.25rem;
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
  gap: 0.375rem;
  flex: 1 1 auto;
  font-family: var(--theme-font-mono);
}

.pair-word {
  padding: 0.1875rem 0.5rem;
  background-color: var(--theme-surface);
  color: var(--theme-fg);
  border: 1px solid var(--theme-border);
  border-radius: 0.1875rem;
  font-size: 0.82rem;
  font-weight: 600;
  letter-spacing: 0.025rem;
}

.pair-scan-frame {
  margin: 0.625rem 0;
  padding: 0.375rem;
  background-color: var(--theme-surface-alt);
  border: 1px solid var(--theme-border);
  border-radius: 0.1875rem;
  display: flex;
  justify-content: center;
}

.pair-scan-video {
  width: 100%;
  max-width: 20rem;
  aspect-ratio: 1 / 1;
  background-color: #000;
  object-fit: cover;
  border-radius: 0.1875rem;
}

.pair-error {
  margin-top: 0.375rem;
  color: var(--theme-status-err);
  font-family: var(--theme-font-mono);
  font-size: 0.7rem;
}

.pair-input-hint {
  margin: 0 0 0.25rem;
  color: var(--theme-fg-dim);
  font-family: var(--theme-font-mono);
  font-size: 0.7rem;
}
</style>

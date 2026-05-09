<script setup lang="ts">
/**
 * Mobile / remote-host pair landing. Renders before the normal overlay
 * boots when the SPA is loaded over the daemon's HTTPS bridge — the
 * phone's first WS upgrade is `pending` until the captain confirms on
 * the desktop. Two distinct codes ride per connection: this side shows
 * `deviceCode` (its identity) as both QR and readable words; to confirm
 * from this side, the captain scans the **desktop's** QR (which encodes
 * the desktop's own `desktopCode`), and the decoded value pushes back
 * over the WS as `{type:"confirm", code}`. The daemon checks against
 * its `desktopCode` — match → authenticated. The same code on both
 * screens would defeat the pairing entirely (anyone with eyes on
 * either screen could fake the proof).
 *
 * Two confirm paths from this side:
 *  1. Wait — captain types / scans this device's code on the desktop.
 *  2. Tap "scan" → device camera reads the desktop's QR → decoded code
 *     pushes back as `{type:"confirm", code}` → daemon authenticates.
 *
 * Once the daemon sends `{type:"authenticated"}`, the parent flips off
 * `pending` and the regular boot continues.
 */
import { faCamera, faLock, faRotate, faXmark } from '@fortawesome/free-solid-svg-icons'
import QrScanner from 'qr-scanner'
import QRCode from 'qrcode'
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'

import { confirmFromBrowser } from '@ipc/remote-bridge'
import { log } from '@lib'

const props = defineProps<{
  /** Code shown on this device — both as readable words and QR. */
  deviceCode: string
  expiresInSeconds: number
  /** Last `confirm-rejected` reason from the daemon, if any. */
  rejectReason?: string
}>()

const qrDataUrl = ref<string | undefined>(undefined)
const qrError = ref<string | undefined>(undefined)

const scanning = ref(false)
const scanError = ref<string | undefined>(undefined)
const videoEl = ref<HTMLVideoElement | undefined>(undefined)
let scanner: QrScanner | undefined

const words = computed<string[]>(() => props.deviceCode.trim().split(/\s+/u).filter(Boolean))

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

async function startScan(): Promise<void> {
  scanError.value = undefined

  // `getUserMedia` is gated on a secure context (HTTPS or localhost).
  // The daemon's bridge is HTTPS so the phone path is fine — guard
  // anyway to surface a readable error if the captain ever serves
  // the SPA from plain HTTP for testing.
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
    scanner = new QrScanner(
      target,
      (result) => {
        const decoded = result.data.trim()

        log.info('remote: QR scanned (mobile)', { length: decoded.length })
        stopScan()

        try {
          confirmFromBrowser(decoded)
        } catch(err) {
          scanError.value = String(err)
        }
      },
      {
        preferredCamera: 'environment',
        highlightScanRegion: true,
        highlightCodeOutline: true,
        maxScansPerSecond: 5
      }
    )
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

onMounted(() => {
  void regenerate(props.deviceCode)
})

onBeforeUnmount(() => {
  stopScan()
})

watch(
  () => props.deviceCode,
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
      <p class="pair-screen-prompt">Show this screen to the desktop, or tap <strong>scan</strong> to read the desktop's QR with this device's camera.</p>

      <div v-if="scanning" class="pair-scan-frame">
        <video ref="videoEl" class="pair-scan-video" muted playsinline autoplay></video>
      </div>

      <template v-else>
        <div v-if="qrDataUrl" class="pair-qr-frame">
          <img :src="qrDataUrl" class="pair-qr" alt="pair-code QR" />
        </div>
        <p v-else-if="qrError" class="pair-qr-error">QR render failed: {{ qrError }}</p>

        <ul class="pair-words" aria-label="pair code">
          <li v-for="(word, i) in words" :key="`w-${i}-${word}`" class="pair-word">{{ word }}</li>
        </ul>
      </template>

      <button v-if="!scanning" type="button" class="pair-scan-btn" @click="startScan"><FaIcon :icon="faCamera" /> scan desktop QR</button>
      <button v-else type="button" class="pair-scan-btn pair-scan-btn-stop" @click="stopScan"><FaIcon :icon="faXmark" /> stop scanning</button>

      <p v-if="scanError" class="pair-error">{{ scanError }}</p>
      <p v-if="rejectReason" class="pair-error">{{ rejectReason }}</p>

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
  gap: 1.125rem;
  height: 100%;
  padding: 1.75rem 1.375rem 2rem;
  background-color: var(--theme-surface-bg);
  color: var(--theme-fg);
  font-family: var(--theme-font-mono);
}

.pair-screen-header {
  @apply flex items-center;
  gap: 0.5rem;
  padding-bottom: 0.375rem;
  border-bottom: 1px solid var(--theme-border-soft);
  font-size: 0.85rem;
  letter-spacing: 0.025rem;
  color: var(--theme-fg-dim);
}

.pair-screen-header-icon {
  width: 0.75rem;
  height: 0.75rem;
  color: var(--theme-status-warn);
}

.pair-screen-body {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 1rem;
}

.pair-screen-prompt {
  text-align: center;
  font-size: 0.85rem;
  color: var(--theme-fg-subtle);
  max-width: 32ch;
  line-height: 1.4;
}

.pair-qr-frame {
  padding: 0.75rem;
  background-color: #ffffff;
  border-radius: 0.375rem;
  border: 1px solid var(--theme-border);
}

.pair-qr {
  display: block;
  width: 13.75rem;
  height: 13.75rem;
  image-rendering: pixelated;
}

.pair-qr-error {
  color: var(--theme-status-err);
  font-size: 0.75rem;
}

.pair-words {
  @apply flex flex-wrap items-center justify-center;
  gap: 0.5rem;
  margin: 0;
  padding: 0;
  list-style: none;
}

.pair-word {
  padding: 0.3125rem 0.625rem;
  background-color: var(--theme-surface);
  color: var(--theme-fg);
  border: 1px solid var(--theme-border);
  border-radius: 0.1875rem;
  font-size: 0.95rem;
  font-weight: 600;
  letter-spacing: 0.025rem;
}

.pair-scan-frame {
  width: 100%;
  max-width: 20rem;
  padding: 0.375rem;
  background-color: var(--theme-surface-alt);
  border: 1px solid var(--theme-border);
  border-radius: 0.25rem;
}

.pair-scan-video {
  display: block;
  width: 100%;
  aspect-ratio: 1 / 1;
  background-color: #000;
  object-fit: cover;
  border-radius: 0.1875rem;
}

.pair-scan-btn {
  @apply inline-flex items-center;
  gap: 0.5rem;
  padding: 0.5rem 1rem;
  background-color: var(--theme-accent);
  color: var(--theme-fg-on-tone);
  border: none;
  border-radius: 0.25rem;
  font-family: var(--theme-font-mono);
  font-size: 0.9rem;
  font-weight: 600;
  cursor: pointer;
  /* Touch-friendly tap target — phone first. */
  min-height: 2.75rem;
}

.pair-scan-btn:active {
  filter: brightness(0.85);
}

.pair-scan-btn-stop {
  background-color: var(--theme-status-err);
}

.pair-error {
  margin: 0;
  color: var(--theme-status-err);
  font-family: var(--theme-font-mono);
  font-size: 0.75rem;
  text-align: center;
}

.pair-screen-status {
  @apply flex items-center;
  gap: 0.5rem;
  margin-top: 0.25rem;
  color: var(--theme-fg-dim);
  font-size: 0.75rem;
}

.pair-screen-status-icon {
  width: 0.625rem;
  height: 0.625rem;
}
</style>

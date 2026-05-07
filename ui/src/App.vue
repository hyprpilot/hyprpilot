<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'

import Loading from '@components/Loading.vue'
import { useBootLoading } from '@composables'
import { isRemoteHost, type PairView, retryRemotePair, subscribePair } from '@ipc/remote-bridge'
import Overlay from '@views/Overlay.vue'
import RemotePairScreen from '@views/RemotePairScreen.vue'

const { status, done } = useBootLoading()

// Remote-host only: the daemon's HTTPS bridge serves this SPA to
// any browser on the LAN. The first WS upgrade lands in `pending`
// state until the captain confirms a pair code on the desktop —
// the regular boot can't run yet because every `invoke()` gates on
// authentication. Render `<RemotePairScreen>` until the daemon
// emits `authenticated`; then fall through to the normal overlay.
const remoteHost = isRemoteHost()
const pair = ref<PairView | undefined>(undefined)
let stopPair: (() => void) | undefined

onMounted(() => {
  if (remoteHost) {
    stopPair = subscribePair((view) => {
      pair.value = view
    })
  }
})

onUnmounted(() => {
  stopPair?.()
  stopPair = undefined
})

const showPairScreen = computed(() => remoteHost && (pair.value === undefined || !pair.value.authenticated))
const showRejectedScreen = computed(() => showPairScreen.value && pair.value?.terminalReason !== undefined)
</script>

<template>
  <main class="overlay-root">
    <RemotePairScreen
      v-if="showPairScreen && pair?.pending"
      :code="pair.pending.code"
      :expires-in-seconds="pair.pending.expiresInSeconds"
      :reject-reason="pair?.lastConfirmRejection"
    />
    <Overlay v-else-if="!showPairScreen" />

    <!-- Rejected / terminal-state screen — replaces the generic
         "connecting…" loader after the daemon sent `{type:"rejected"}`
         (captain rejected, pair expired, attempt cap hit) or the WS
         dropped. Survives the close so the captain doesn't bounce
         back to a meaningless loader. Retry reloads the SPA — fresh
         WS, fresh pending code. -->
    <div v-if="showRejectedScreen" class="pair-rejected">
      <h1 class="pair-rejected-title">pair rejected</h1>
      <p class="pair-rejected-reason">{{ pair?.terminalReason }}</p>
      <button type="button" class="pair-rejected-btn" @click="retryRemotePair">try again</button>
    </div>

    <!-- Connecting indicator for the remote-host before the first
         pending frame lands (WS still opening). Renders as a
         fullscreen loader so the captain isn't staring at a blank
         body. -->
    <Loading v-else-if="showPairScreen && !pair?.pending" mode="fullscreen" status="connecting to daemon…" />

    <!-- Fullscreen boot overlay paints over Overlay until main.ts's
         `boot()` resolves. `markBootDone()` flips `done` and the
         overlay disappears, leaving the populated UI behind. -->
    <Loading v-if="!showPairScreen && !done" mode="fullscreen" :status="status" />
  </main>
</template>

<style scoped>
@reference './assets/styles.css';

.pair-rejected {
  position: fixed;
  inset: 0;
  z-index: 60;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 16px;
  padding: 28px 22px;
  background-color: var(--theme-surface-bg);
  color: var(--theme-fg);
  font-family: var(--theme-font-mono);
  text-align: center;
}

.pair-rejected-title {
  margin: 0;
  font-size: 1.2rem;
  font-weight: 700;
  color: var(--theme-status-err);
  letter-spacing: 0.4px;
}

.pair-rejected-reason {
  margin: 0;
  max-width: 32ch;
  font-size: 0.9rem;
  color: var(--theme-fg-subtle);
  line-height: 1.4;
}

.pair-rejected-btn {
  margin-top: 8px;
  padding: 10px 20px;
  background-color: var(--theme-accent);
  color: var(--theme-fg-on-tone);
  border: none;
  border-radius: 4px;
  font-family: inherit;
  font-size: 0.9rem;
  font-weight: 600;
  cursor: pointer;
  /* Touch-friendly tap target — same min-height as the scan button
   * on the pair screen so this view feels native to mobile. */
  min-height: 44px;
}

.pair-rejected-btn:active {
  filter: brightness(0.85);
}
</style>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'

import Loading from '@components/Loading.vue'
import { useBootLoading } from '@composables'
import { isRemoteHost, type PairView, subscribePair } from '@ipc/remote-bridge'
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

    <!-- Connecting indicator for the remote-host before the first
         pending frame lands (WS still opening). Renders as a
         fullscreen loader so the captain isn't staring at a blank
         body. -->
    <Loading v-if="showPairScreen && !pair?.pending" mode="fullscreen" status="connecting to daemon…" />

    <!-- Fullscreen boot overlay paints over Overlay until main.ts's
         `boot()` resolves. `markBootDone()` flips `done` and the
         overlay disappears, leaving the populated UI behind. -->
    <Loading v-if="!showPairScreen && !done" mode="fullscreen" :status="status" />
  </main>
</template>

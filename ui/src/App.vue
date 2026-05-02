<script setup lang="ts">
import Loading from '@components/Loading.vue'
import { useBootLoading } from '@composables'
import Overlay from '@views/Overlay.vue'

const { status, done } = useBootLoading()
</script>

<template>
  <main class="overlay-root">
    <Overlay />
    <!-- Fullscreen boot overlay paints over Overlay until main.ts's
         `boot()` resolves. `markBootDone()` flips `done` and the
         overlay disappears, leaving the populated UI behind. -->
    <Loading v-if="!done" mode="fullscreen" :status="status" />
  </main>
</template>

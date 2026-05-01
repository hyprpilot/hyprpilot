<script setup lang="ts">
import RoleTag from './RoleTag.vue'
import { Role } from '@components'

/**
 * turn lane: 2px colored left stripe wraps the whole turn (role tag
 * + optional elapsed chip + body). Stripe color is the visual law #2
 * — every captain or pilot turn is a vertical lane the eye follows
 * through deep tool nests. Captain (`Role.User`) → green; Pilot
 * (`Role.Assistant`) → red.
 */
const props = withDefaults(
  defineProps<{
    role: Role
    elapsed?: string
    live?: boolean
  }>(),
  { live: false }
)

const ROLE_LABELS: Record<Role, string> = {
  [Role.User]: 'captain',
  [Role.Assistant]: 'pilot'
}
const roleLabel = ROLE_LABELS[props.role]
</script>

<template>
  <article class="turn" :data-role="role" :data-live="live">
    <header class="turn-header">
      <RoleTag :role="role" :label="roleLabel" />
      <span v-if="elapsed && role === Role.Assistant" class="turn-elapsed">
        <span v-if="live" class="turn-live-dot" aria-hidden="true" />
        <span>{{ elapsed }}</span>
      </span>
    </header>
    <div class="turn-body">
      <slot />
    </div>
  </article>
</template>

<style scoped>
@reference '../../assets/styles.css';

/* turn lane: 2px stripe, padding-left 4px (per wireframe spec). */
.turn {
  @apply flex flex-col py-1;
  padding-left: 4px;
  border-left: 2px solid var(--theme-accent-user);
  position: relative;
}

.turn[data-role='assistant'] {
  border-left-color: var(--theme-accent-assistant);
}

.turn-header {
  @apply flex items-center gap-2;
  margin-bottom: 4px;
}

/* Live elapsed chip: dim by default, glows yellow + pulses when live. */
.turn-elapsed {
  @apply ml-auto inline-flex shrink-0 items-center gap-1 rounded-sm border px-[5px] py-0 text-[0.56rem];
  color: var(--theme-fg-dim);
  background-color: var(--theme-surface);
  border-color: var(--theme-border);
  font-family: var(--theme-font-mono);
  letter-spacing: 0.3px;
}

.turn[data-live='true'] .turn-elapsed {
  color: var(--theme-state-stream);
}

.turn-live-dot {
  @apply inline-block h-[4px] w-[4px] animate-pulse rounded-full;
  background-color: var(--theme-state-stream);
}

/* wireframe: 4px gap between turn body children — chunks, tool
 * chips, stream cards stack tightly. */
.turn-body {
  @apply flex flex-col;
  gap: 4px;
}
</style>

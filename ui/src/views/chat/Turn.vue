<script setup lang="ts">
import RoleTag from './RoleTag.vue'
import { Role, StatPill } from '@components'

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
      <StatPill v-if="elapsed && role === Role.Assistant" class="turn-elapsed" :label="elapsed" :live="live" />
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

/* Push the elapsed chip to the right edge of the header. */
.turn-elapsed {
  margin-left: auto;
}

/* wireframe: 4px gap between turn body children — chunks, tool
 * chips, stream cards stack tightly. */
.turn-body {
  @apply flex flex-col;
  gap: 4px;
}
</style>

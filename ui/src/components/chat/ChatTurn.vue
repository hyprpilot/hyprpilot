<script setup lang="ts">
import RoleTag from './ChatRoleTag.vue'
import { Role } from '../types'

/**
 * Turn lane: colored left border wraps the whole turn (role tag +
 * elapsed badge + body). Border color follows role: user/yellow or
 * assistant/green. Port of D5's `D5Turn`. Stateless.
 */
const props = withDefaults(
  defineProps<{
    role: Role
    elapsed?: string
    live?: boolean
  }>(),
  { live: false }
)

// Presentation-only mapping: `Role` is the domain discriminator,
// `label` is what the role-tag renders. Keep the map inline — RoleTag
// is the only consumer.
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
      <span v-if="elapsed" class="turn-elapsed">
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

.turn {
  @apply flex flex-col gap-1 py-1 pl-2;
  border-left: 2px solid var(--theme-accent-user);
  position: relative;
}

.turn[data-role='assistant'] {
  border-left-color: var(--theme-accent-assistant);
}

.turn-header {
  @apply flex items-center gap-2;
}

.turn-elapsed {
  @apply ml-auto inline-flex items-center gap-1 rounded-sm border px-[5px] py-0 text-[0.68rem];
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
  @apply inline-block h-[4px] w-[4px] animate-pulse-slow rounded-full;
  background-color: var(--theme-state-stream);
}

.turn-body {
  @apply flex flex-col gap-1;
}
</style>

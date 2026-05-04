<script setup lang="ts">
import { computed } from 'vue'

import ToolBody from '../chat/ToolBody.vue'
import type { PermissionView } from '@components'

/**
 * Single permission row. Renders the unified `ToolCallView` chrome
 * (icon + composed title + structured fields) plus a button per
 * agent-offered `PermissionOption`. No icons on the action row —
 * each button is a text label.
 *
 * The label is the agent's `name` field passed through verbatim —
 * vendors carry meaningful information here (claude-code ships
 * `Always allow Bash(curl -sSo /dev/null ...)` so the captain sees
 * the literal command pattern that the persistent rule will store).
 * Re-casing strips that detail; we keep the wire shape intact.
 *
 * Long names are constrained via `max-width` + `text-overflow:
 * ellipsis` so the row doesn't wrap; hover surfaces the full string
 * via the `title` attribute. Tone comes from the option's typed
 * `kind`:
 *
 * - `allow_*` → ok tone.
 * - `reject_*` → err tone.
 * - anything else (forward-compat for new ACP variants) → neutral.
 *
 * Emits `reply` with the real `optionId` from the offered set. The
 * captain's "remember this" intent rides on the option's `kind`
 * (the daemon-side controller writes the trust store atomically for
 * `_always` variants); UI doesn't carry a separate `remember` flag.
 */
const props = defineProps<{
  view: PermissionView
}>()

const emit = defineEmits<{
  reply: [optionId: string]
  dismiss: []
}>()

interface ButtonView {
  optionId: string
  label: string
  tone: 'ok' | 'err' | 'neutral'
  variant: 'solid' | 'outline'
}

const buttons = computed<ButtonView[]>(() =>
  props.view.options.map((opt) => {
    const tone = opt.kind.startsWith('allow') ? 'ok' : opt.kind.startsWith('reject') ? 'err' : 'neutral'
    // `allow_once` is the agent-default + the most common pick, so
    // it gets the solid fill; every other variant renders as an
    // outline to keep the row's primary action obvious.
    const variant = opt.kind === 'allow_once' ? 'solid' : 'outline'

    return {
      optionId: opt.optionId,
      label: opt.name,
      tone,
      variant
    }
  })
)
</script>

<template>
  <article class="permission-row" data-testid="permission-row">
    <header class="permission-row-header">
      <span class="permission-row-tool" :aria-label="view.call.title">
        <FaIcon :icon="view.call.icon" class="permission-row-icon" aria-hidden="true" />
        <span class="permission-row-title">{{ view.call.title }}</span>
      </span>
      <span class="permission-row-spacer" />
      <div class="permission-row-actions">
        <button
          v-for="b in buttons"
          :key="b.optionId"
          type="button"
          class="permission-row-btn"
          :data-tone="b.tone"
          :data-variant="b.variant"
          :aria-label="b.label"
          :title="b.label"
          @click="emit('reply', b.optionId)"
        >
          {{ b.label }}
        </button>
      </div>
    </header>
    <div class="permission-row-body">
      <ToolBody :view="view.call" />
    </div>
  </article>
</template>

<style scoped>
@reference '../../assets/styles.css';

.permission-row {
  @apply flex flex-col;
  background-color: var(--theme-permission-bg);
  border-top: 1px solid var(--theme-border-soft);
}

.permission-row-header {
  @apply sticky top-0 z-10 flex items-center gap-[10px] text-[0.7rem];
  background-color: var(--theme-permission-bg);
  border-bottom: 1px solid var(--theme-border-soft);
  font-family: var(--theme-font-mono);
  padding: 6px 14px 6px 4px;
}

.permission-row-tool {
  @apply inline-flex shrink-0 items-center gap-[5px] text-[0.62rem];
  background-color: var(--theme-status-warn);
  color: var(--theme-fg-on-tone);
  padding: 2px 7px;
  border-radius: 3px;
  font-weight: 700;
}

.permission-row-icon {
  width: 9px;
  height: 9px;
}

.permission-row-title {
  font-weight: 700;
}

.permission-row-spacer {
  flex: 1;
}

.permission-row-actions {
  /* Buttons can flex-grow to share remaining row width up to the
   * cap below; prior `shrink-0` kept them tight against the actions
   * gutter regardless of available space. Captain wanted them to
   * expand into that space and only truncate once they hit the
   * shared per-button cap. */
  @apply flex min-w-0 flex-1 items-center gap-1;
  justify-content: flex-end;
}

.permission-row-btn {
  /* Left-aligned label so a long captain-prefix ("always allow") +
   * a vendor-supplied rule context read as one phrase running
   * left-to-right. Centred labels collapsed the visual into a
   * disconnected glyph block. */
  @apply inline-flex flex-1 cursor-pointer items-center justify-start text-[0.62rem];
  padding: 3px 8px;
  border-radius: 3px;
  background-color: transparent;
  border: 1px solid var(--theme-border);
  color: var(--theme-fg-dim);
  font-family: var(--theme-font-mono);
  font-weight: 600;
  letter-spacing: 0.2px;
  white-space: nowrap;
  /* Vendors stuff rule-context into the option's `name` (claude-code
   * ships the full bash command on `allow_always`). Cap the visible
   * width + ellipsise; the full string survives on `title` for
   * hover. Min-width 0 lets the button shrink below intrinsic
   * content size so flex truncation works. */
  min-width: 0;
  max-width: 32ch;
  overflow: hidden;
  text-overflow: ellipsis;
}

.permission-row-btn[data-tone='ok'] {
  border-color: var(--theme-status-ok);
  color: var(--theme-status-ok);
}

.permission-row-btn[data-tone='err'] {
  border-color: var(--theme-status-err);
  color: var(--theme-status-err);
}

.permission-row-btn[data-tone='neutral'] {
  border-color: var(--theme-accent);
  color: var(--theme-accent);
}

.permission-row-btn[data-variant='solid'][data-tone='ok'] {
  background-color: var(--theme-status-ok);
  color: var(--theme-fg-on-tone);
}

.permission-row-btn[data-variant='solid'][data-tone='err'] {
  background-color: var(--theme-status-err);
  color: var(--theme-fg-on-tone);
}

.permission-row-btn[data-variant='solid'][data-tone='neutral'] {
  background-color: var(--theme-accent);
  color: var(--theme-fg-on-tone);
}

.permission-row-btn:hover {
  filter: brightness(1.15);
}

.permission-row-body {
  @apply flex flex-col;
  padding: 8px 10px;
}
</style>

<script setup lang="ts">
import { faSquare as farSquare } from '@fortawesome/free-regular-svg-icons'
import { faChevronDown, faChevronRight, faCircleHalfStroke, faSquareCheck } from '@fortawesome/free-solid-svg-icons'
import { computed, ref, useSlots, watch } from 'vue'

import { MarkdownBody, PlanStatus, StatPill, StreamKind, type PlanItem } from '@components'

/**
 * stream card — thinking / planning. Two states:
 *   active=true  → expanded card, glow dot, full body (checklist for
 *                  planning items, prose for thinking text).
 *   active=false → collapsed one-line summary, italic Inter recap to
 *                  differentiate from the structured mono header.
 *
 * Header: chevron caret + glow dot (active) + uppercase mono label +
 *         `· elapsed` + (collapsed only) summary in italic.
 *
 * Thinking content is the agent's free-form reasoning prose — the
 * agent commonly emits it as markdown (lists, **bold**, fences). We
 * route it through the same `renderMarkdown` pipeline `ChatBody`
 * uses so thoughts read like agent prose, not like raw `<pre>`. The
 * legacy `<slot>` path stays as a typed-text escape hatch when no
 * `text` is passed.
 *
 * Planning checklist icons (FontAwesome — never unicode):
 *   todo       far square             gray    not yet
 *   active     fas circle-half-stroke orange  currently doing
 *   done       fas square-check       green   done
 */
const props = defineProps<{
  kind: StreamKind
  active: boolean
  label: string
  elapsed?: string
  summary?: string
  items?: PlanItem[]
  /// Free-form prose body. Routed through `renderMarkdown` for the
  /// thinking kind; falls through to `<slot>` when omitted.
  text?: string
  /// `done / total` running stat for checklist-shaped cards (plans
  /// today). Rendered as a `N/M` chip in the header — mirrors the
  /// elapsed chip alongside. `undefined` hides the chip entirely.
  stats?: { done: number; total: number }
}>()

const statsLabel = computed<string | undefined>(() => {
  const s = props.stats

  if (!s || s.total === 0) {
    return undefined
  }

  return `${s.done}/${s.total}`
})

const slots = useSlots()

// planning → agent (purple); thinking → think (muted slate).
const tone = computed(() => (props.kind === StreamKind.Planning ? 'var(--theme-kind-agent)' : 'var(--theme-kind-think)'))

const hasItems = computed(() => (props.items?.length ?? 0) > 0)
const hasSlot = computed(() => Boolean(slots.default))
// Treat whitespace-only text as no text at all. Vendors (notably
// Opus) emit thought chunks whose `text` is `"\n"` / `"\n\n"` as
// filler between real content — concatenated they become visible
// blank rows that look like a layout bug. The header chrome
// (label / elapsed / stats pills) still renders, so the captain
// still sees "thought · 12s" even when the body is empty.
const hasNonWhitespaceText = computed(() => typeof props.text === 'string' && props.text.trim() !== '')
const useMarkdown = computed(() => props.kind === StreamKind.Thinking && hasNonWhitespaceText.value)
/// No expandable content → header is the whole card (used by the
/// thinking-time-only fallback path: agent reasoned silently for N
/// seconds, no prose to render). Hides the chevron + drops the
/// click affordance so the row reads as a static badge, not a
/// fooling-the-captain "click me to expand into nothing".
const hasBody = computed(() => hasItems.value || useMarkdown.value || hasSlot.value)

// Local expanded state — seeded from the `active` prop so live cards
// open by default. Clicking the header toggles regardless. Once the
// user has manually collapsed/expanded, we stop tracking the prop so
// their click intent isn't overridden by a stream update.
const expanded = ref(props.active)
let manuallyToggled = false

watch(
  () => props.active,
  (next) => {
    if (!manuallyToggled) {
      expanded.value = next
    }
  }
)

function toggle(): void {
  manuallyToggled = true
  expanded.value = !expanded.value
}

function planIconFor(status: PlanStatus) {
  switch (status) {
    case PlanStatus.Completed:
      return faSquareCheck

    case PlanStatus.InProgress:
      return faCircleHalfStroke

    case PlanStatus.Pending:
    default:
      return farSquare
  }
}
</script>

<template>
  <article class="stream-card" :data-kind="kind" :data-active="expanded" :data-has-body="hasBody" :style="{ '--tone': tone }">
    <header
      class="stream-card-header"
      :role="hasBody ? 'button' : undefined"
      :tabindex="hasBody ? 0 : undefined"
      :aria-expanded="hasBody ? expanded : undefined"
      @click="hasBody && toggle()"
      @keydown.enter.prevent="hasBody && toggle()"
      @keydown.space.prevent="hasBody && toggle()"
    >
      <FaIcon v-if="hasBody" :icon="expanded ? faChevronDown : faChevronRight" class="stream-card-caret" aria-hidden="true" />
      <span class="stream-card-label">{{ label }}</span>
      <span v-if="!expanded && summary" class="stream-card-summary-inline">{{ summary }}</span>
      <StatPill v-if="statsLabel" class="stream-card-stat" :label="statsLabel" :live="active" />
      <StatPill v-if="elapsed" class="stream-card-elapsed" :label="elapsed" :live="active" />
    </header>

    <div v-if="expanded && hasItems" class="stream-card-body">
      <ul class="stream-card-list">
        <li v-for="(item, idx) in items" :key="idx" class="stream-card-item" :data-status="item.status">
          <FaIcon :icon="planIconFor(item.status)" class="stream-card-glyph" aria-hidden="true" />
          <span class="stream-card-text">{{ item.text }}</span>
        </li>
      </ul>
    </div>
    <div v-else-if="expanded && useMarkdown && text" class="stream-card-body stream-card-prose">
      <MarkdownBody :source="text" />
    </div>
    <div v-else-if="expanded && hasSlot" class="stream-card-body stream-card-plain">
      <slot />
    </div>
  </article>
</template>

<style scoped>
@reference '../../assets/styles.css';

/* Active: filled surface bg + line2 border; collapsed: transparent +
 * line border. 3px tone stripe on the left in both states. */
.stream-card {
  @apply flex flex-col text-[0.78rem] leading-snug;
  color: var(--theme-fg);
  border-left: 0.1875rem solid var(--tone);
  border-top: 1px solid var(--theme-border-soft);
  border-right: 1px solid var(--theme-border-soft);
  border-bottom: 1px solid var(--theme-border-soft);
  border-radius: 0.25rem;
  background-color: var(--theme-surface);
  font-family: var(--theme-font-sans);
  padding: 0.375rem 0.625rem;
}

.stream-card[data-active='false'] {
  background-color: transparent;
  border-top-color: var(--theme-border);
  border-right-color: var(--theme-border);
  border-bottom-color: var(--theme-border);
  padding: 0.25rem 0.625rem;
}

.stream-card-header {
  @apply flex min-w-0 items-center gap-2 text-[0.62rem] uppercase;
  color: var(--theme-fg);
  font-family: var(--theme-font-mono);
  letter-spacing: 0.025rem;
}

.stream-card-caret,
.stream-card-label,
.stream-card-elapsed {
  flex-shrink: 0;
}

/* Cursor only on rows that actually expand into a body. */
.stream-card[data-has-body='true'] .stream-card-header {
  cursor: pointer;
}

.stream-card-caret {
  width: 0.5625rem;
  height: 0.5625rem;
  color: var(--theme-fg-dim);
}

.stream-card[data-active='true'] .stream-card-caret {
  color: var(--tone);
}

.stream-card-label {
  @apply font-bold;
  color: var(--tone);
}

/* Push the elapsed chip to the right edge of the header so it
 * aligns with the Turn footer's elapsed chip — same visual law. */
.stream-card-elapsed {
  margin-left: auto;
}

/* Italic Inter recap — visually distinct from the mono header tag.
 * `flex: 1 1 auto` + `min-w-0` so the recap is the only column that
 * yields when a long summary would push the elapsed chip off the
 * right edge. */
.stream-card-summary-inline {
  @apply ml-1 min-w-0 truncate text-[0.7rem] italic normal-case;
  flex: 1 1 auto;
  color: var(--theme-fg-subtle);
  font-family: var(--theme-font-sans);
  letter-spacing: normal;
  font-weight: normal;
}

/* Body separator: dashed line above + 6px top padding. */
.stream-card-body {
  margin-top: 0.375rem;
  padding-top: 0.375rem;
  border-top: 1px dashed var(--theme-border);
}

.stream-card-list {
  @apply m-0 flex list-none flex-col p-0;
  gap: 0.25rem;
}

.stream-card-item {
  @apply flex items-start gap-2 text-[0.7rem];
  font-family: var(--theme-font-mono);
  line-height: 1.45;
}

.stream-card-glyph {
  @apply shrink-0;
  width: 0.6875rem;
  height: 0.6875rem;
  color: var(--theme-fg-dim);
}

.stream-card-item[data-status='completed'] .stream-card-glyph {
  color: var(--theme-status-ok);
}

.stream-card-item[data-status='in_progress'] .stream-card-glyph {
  color: var(--theme-state-awaiting);
}

/* done = dim text, NOT struck through. */
.stream-card-text {
  @apply flex-1;
  color: var(--theme-fg-subtle);
}

.stream-card-item[data-status='completed'] .stream-card-text {
  color: var(--theme-fg-dim);
}

/* Plain prose body — used as the fallback when markdown render fails
 * or when the legacy slot is passed. Inter font (matches the rest of
 * the chat prose); preserves newlines so line-broken thoughts read
 * correctly without a `<pre>` shape. */
.stream-card-plain {
  @apply text-[0.78rem] leading-relaxed;
  color: var(--theme-fg-subtle);
  font-family: var(--theme-font-sans);
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}

/* Wrap `<MarkdownBody>` — same prose vocabulary as `ChatBody`
 * (paragraphs / lists / blockquotes / tables / fenced code with
 * GitHub-spec margins all live there). Stream cards layer a dimmer
 * ink tone on top so thoughts read as "internal monologue" vs
 * assistant-spoken prose. */
.stream-card-prose {
  @apply text-[0.78rem] leading-normal;
  font-family: var(--theme-font-sans);
  overflow-wrap: anywhere;
}

.stream-card-prose :deep(.markdown-body) {
  font-size: inherit;
  line-height: inherit;
  color: var(--theme-fg-subtle);
}
</style>

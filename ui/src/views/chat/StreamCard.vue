<script setup lang="ts">
import { faSquare as farSquare } from '@fortawesome/free-regular-svg-icons'
import { faChevronDown, faChevronRight, faCircleHalfStroke, faSquareCheck } from '@fortawesome/free-solid-svg-icons'
import { computed, ref, useSlots, watch } from 'vue'

import { PlanStatus, StatPill, StreamKind, type PlanItem } from '@components'
import { log, renderMarkdown, renderMarkdownPlain } from '@lib'

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
}>()

const slots = useSlots()

// planning → agent (purple); thinking → think (muted slate).
const tone = computed(() => (props.kind === StreamKind.Planning ? 'var(--theme-kind-agent)' : 'var(--theme-kind-think)'))

const hasItems = computed(() => (props.items?.length ?? 0) > 0)
const hasSlot = computed(() => Boolean(slots.default))
const useMarkdown = computed(() => props.kind === StreamKind.Thinking && Boolean(props.text))
/// No expandable content → header is the whole card (used by the
/// thinking-time-only fallback path: agent reasoned silently for N
/// seconds, no prose to render). Hides the chevron + drops the
/// click affordance so the row reads as a static badge, not a
/// fooling-the-captain "click me to expand into nothing".
const hasBody = computed(() => hasItems.value || useMarkdown.value || hasSlot.value)

const renderedHtml = ref('')

// Two-pass render — same strategy as `<MarkdownBody>`. Plain
// markdown-it first (synchronous, no Shiki) so the prose lands
// instantly even on the cold path; then upgrade with the full
// Shiki pipeline asynchronously. Without this the first turn after
// boot renders the raw markdown source for ~200ms while Shiki's
// WASM engine warms up.
watch(
  [() => props.text, useMarkdown],
  async([raw, on]) => {
    if (!on || !raw) {
      renderedHtml.value = ''

      return
    }

    try {
      renderedHtml.value = renderMarkdownPlain(raw)
    } catch(err) {
      log.warn('stream-card: plain render failed', { err: String(err) })
      renderedHtml.value = ''
    }

    try {
      const out = await renderMarkdown(raw)

      renderedHtml.value = out.html
    } catch(err) {
      log.warn('stream-card: shiki upgrade failed; keeping plain pass', { err: String(err) })
    }
  },
  { immediate: true }
)

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
    <!-- eslint-disable-next-line vue/no-v-html -- HTML is sanitised by renderMarkdown -->
    <div v-else-if="expanded && useMarkdown && renderedHtml" class="stream-card-body stream-card-prose prose" v-html="renderedHtml" />
    <div v-else-if="expanded && useMarkdown && !renderedHtml && text" class="stream-card-body stream-card-plain">{{ text }}</div>
    <div v-else-if="expanded && hasSlot" class="stream-card-body stream-card-plain">
      <slot />
    </div>
  </article>
</template>

<style scoped>
@reference '../../assets/styles.css';

/* wireframe active: filled surface bg + line2 border; collapsed: transparent
 * + line border. 3px tone stripe on the left in both states. */
.stream-card {
  @apply flex flex-col text-[0.78rem] leading-snug;
  color: var(--theme-fg);
  border-left: 3px solid var(--tone);
  border-top: 1px solid var(--theme-border-soft);
  border-right: 1px solid var(--theme-border-soft);
  border-bottom: 1px solid var(--theme-border-soft);
  border-radius: 4px;
  background-color: var(--theme-surface);
  font-family: var(--theme-font-sans);
  padding: 6px 10px;
}

.stream-card[data-active='false'] {
  background-color: transparent;
  border-top-color: var(--theme-border);
  border-right-color: var(--theme-border);
  border-bottom-color: var(--theme-border);
  padding: 4px 10px;
}

.stream-card-header {
  @apply flex items-center gap-2 text-[0.62rem] uppercase;
  color: var(--theme-fg);
  font-family: var(--theme-font-mono);
  letter-spacing: 0.4px;
}

/* Cursor only on rows that actually expand into a body. */
.stream-card[data-has-body='true'] .stream-card-header {
  cursor: pointer;
}

.stream-card-caret {
  width: 9px;
  height: 9px;
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

/* Italic Inter recap — visually distinct from the mono header tag. */
.stream-card-summary-inline {
  @apply ml-1 truncate text-[0.7rem] italic normal-case;
  color: var(--theme-fg-subtle);
  font-family: var(--theme-font-sans);
  letter-spacing: normal;
  font-weight: normal;
}

/* Body separator: dashed line above + 6px top padding, per wireframe. */
.stream-card-body {
  margin-top: 6px;
  padding-top: 6px;
  border-top: 1px dashed var(--theme-border);
}

.stream-card-list {
  @apply m-0 flex list-none flex-col p-0;
  gap: 4px;
}

.stream-card-item {
  @apply flex items-start gap-2 text-[0.7rem];
  font-family: var(--theme-font-mono);
  line-height: 1.45;
}

.stream-card-glyph {
  @apply shrink-0;
  width: 11px;
  height: 11px;
  color: var(--theme-fg-dim);
}

.stream-card-item[data-status='completed'] .stream-card-glyph {
  color: var(--theme-status-ok);
}

.stream-card-item[data-status='in_progress'] .stream-card-glyph {
  color: var(--theme-state-awaiting);
}

/* wireframe fidelity: done = dim text, NOT struck through. */
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

/* Markdown-rendered body — same prose vocabulary as `ChatBody`'s
 * markdown render (paragraphs / lists / code blocks / etc.) but
 * dimmer ink to read as "internal monologue" vs assistant-spoken
 * prose. */
.stream-card-prose {
  @apply text-[0.78rem] leading-relaxed;
  color: var(--theme-fg-subtle);
  font-family: var(--theme-font-sans);
  overflow-wrap: anywhere;
}

.stream-card-prose :deep(p) {
  @apply my-1;
}

.stream-card-prose :deep(p:first-child) {
  @apply mt-0;
}

.stream-card-prose :deep(p:last-child) {
  @apply mb-0;
}

.stream-card-prose :deep(ul),
.stream-card-prose :deep(ol) {
  @apply my-1 pl-5;
}

.stream-card-prose :deep(code) {
  @apply rounded-sm px-1 py-[1px] text-[0.85em];
  font-family: var(--theme-font-mono);
  background-color: var(--theme-surface-alt);
  color: var(--theme-fg);
}
</style>

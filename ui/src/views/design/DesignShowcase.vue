<script setup lang="ts">
import Conversation from './stories/Conversation.vue'
import Idle from './stories/Idle.vue'
import PaletteModes from './stories/PaletteModes.vue'
import PalettePickers from './stories/PalettePickers.vue'
import PaletteRoot from './stories/PaletteRoot.vue'
import PaletteSessions from './stories/PaletteSessions.vue'
import Permission from './stories/Permission.vue'
import Queue from './stories/Queue.vue'
import ToolCalls from './stories/ToolCalls.vue'
import TweakPanel from './TweakPanel.vue'

interface Story {
  n: string
  name: string
  note?: string
  component: unknown
}

interface NarrowStory extends Story {
  width: number
}

const stories: Story[] = [
  { n: '01', name: 'Idle', note: 'no active turn; live-sessions grid visible', component: Idle },
  { n: '02', name: 'Conversation', note: 'user+assistant turns, thinking→message, active plan', component: Conversation },
  { n: '03', name: 'Tool calls', note: 'small-tool flex-wrap row, big-tool row, running terminal', component: ToolCalls },
  { n: '04', name: 'Permission', note: 'oldest-first active allow/deny + queued rows', component: Permission },
  { n: '05', name: 'Queue', note: 'FIFO queue strip with row actions', component: Queue },
  { n: '06', name: 'Palette — root', note: 'root menu (trust-store / wire-log / references dropped)', component: PaletteRoot },
  { n: '07', name: 'Palette — modes', note: 'ask / plan / accept-edits / bypass', component: PaletteModes },
  { n: '08', name: 'Palette — pickers', note: 'agents / profiles / skills columns', component: PalettePickers },
  { n: '09', name: 'Palette — sessions', note: 'list + preview pane (wide shell)', component: PaletteSessions }
]

// Narrow-width sibling row — renders representative stories clamped to a
// 360px frame so the @container queries in Frame.vue fire. Reviewers can
// eyeball that nothing horizontal-scrolls or clips on a worst-case
// right-anchor 40% width on a small monitor.
const narrowStories: NarrowStory[] = [
  { n: 'N1', name: 'Idle · narrow', note: '360px anchor; provider/model pill auto-hides', component: Idle, width: 360 },
  { n: 'N2', name: 'Conversation · narrow', note: '360px anchor; git chip stays, worktree pill hides <340', component: Conversation, width: 360 },
  { n: 'N3', name: 'Tool calls · narrow', note: '360px anchor; small-tool chips wrap', component: ToolCalls, width: 360 }
]
</script>

<template>
  <main class="design-showcase">
    <header class="design-showcase-header">
      <h1 class="design-showcase-title">hyprpilot · design fixtures (K-250)</h1>
      <p class="design-showcase-subtitle">
        every primitive ported from the D5 bundle, mounted against the fixture data called out in each story's
        <code>*.fixture.ts</code>. no IPC, no daemon. toggle the phase swatch in the bottom-right to preview phase tokens.
      </p>
    </header>

    <section v-for="s in stories" :key="s.n" class="design-showcase-section">
      <div class="design-showcase-label">
        <span class="design-showcase-label-n">{{ s.n }}</span>
        <span class="design-showcase-label-name">{{ s.name }}</span>
        <span v-if="s.note" class="design-showcase-label-note">— {{ s.note }}</span>
      </div>
      <div class="design-showcase-frame">
        <component :is="s.component" />
      </div>
    </section>

    <section v-for="s in narrowStories" :key="s.n" class="design-showcase-section">
      <div class="design-showcase-label">
        <span class="design-showcase-label-n">{{ s.n }}</span>
        <span class="design-showcase-label-name">{{ s.name }}</span>
        <span v-if="s.note" class="design-showcase-label-note">— {{ s.note }}</span>
      </div>
      <div class="design-showcase-frame design-showcase-frame-narrow" :style="{ width: `${s.width}px` }">
        <component :is="s.component" />
      </div>
    </section>

    <TweakPanel />
  </main>
</template>

<style scoped>
@reference '../../assets/styles.css';

.design-showcase {
  @apply flex min-h-screen flex-col gap-6 px-8 py-6;
  background-color: var(--theme-surface-bg);
  color: var(--theme-fg);
  font-family: var(--theme-font-sans);
}

.design-showcase-header {
  @apply flex flex-col gap-1;
}

.design-showcase-title {
  @apply m-0 text-[1.1rem];
  color: var(--theme-fg);
}

.design-showcase-subtitle {
  @apply m-0 max-w-3xl text-[0.8rem];
  color: var(--theme-fg-dim);
}

.design-showcase-subtitle code {
  font-family: var(--theme-font-mono);
  color: var(--theme-fg-ink-2);
}

.design-showcase-section {
  @apply flex flex-col gap-2;
}

.design-showcase-label {
  @apply flex items-baseline gap-2 text-[0.78rem];
  color: var(--theme-fg-dim);
  font-family: var(--theme-font-mono);
}

.design-showcase-label-n {
  @apply font-bold;
  color: var(--theme-accent);
}

.design-showcase-label-name {
  @apply font-bold;
  color: var(--theme-fg-ink-2);
}

.design-showcase-label-note {
  color: var(--theme-fg-dim);
}

.design-showcase-frame {
  @apply overflow-hidden border;
  width: 680px;
  height: 720px;
  border-color: var(--theme-border);
  background-color: var(--theme-surface-bg);
  display: flex;
  align-items: center;
  justify-content: center;
}

/* Narrow-mode preview frames reuse the standard 720px height so the
 * transcript body can breathe; only width shrinks per the per-story
 * `:style`. The overflow-hidden on the parent stops an incorrectly
 * sized child from breaking the reviewer's scroll timeline. */
.design-showcase-frame-narrow {
  width: auto;
}

/* Palette shells float on their own backdrop rather than filling the frame,
 * so center them inside the 680x720 card. Frame content can either fill the
 * whole box (frame-wrapped stories) or be centered (palette variants). */
.design-showcase-frame > :deep(.frame) {
  width: 100%;
  height: 100%;
}
</style>

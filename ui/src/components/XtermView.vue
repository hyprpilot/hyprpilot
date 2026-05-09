<script setup lang="ts">
/**
 * Lightweight xterm.js wrapper. Renders ANSI escape sequences (color
 * codes, cursor moves, clear-line, etc.) the way a real terminal
 * would — strictly better than `<pre>{{ stripAnsi(text) }}</pre>`,
 * which discards every escape and shows raw stdout in one tone.
 *
 * **Streaming model.** The component owns the xterm `Terminal`
 * instance and writes `text` deltas into it as the prop updates.
 * Append-only: every render compares the new prop length to what
 * we've already written and pumps the suffix through `term.write`.
 * On a wholesale rewrite (the prop got SHORTER, or the new text
 * doesn't extend the old) we `term.reset()` and rewrite from
 * scratch. The captain's selection survives append-only updates;
 * a wholesale rewrite is rare (snapshot replay, reset).
 *
 * **Sizing.** `@xterm/addon-fit` snaps the grid to the host
 * element's pixel size. We re-fit on container resize via a
 * `ResizeObserver` and on every text update (since auto-scroll
 * after a write needs the row count to be correct). A `rows`
 * prop caps the visible terminal height — the captain doesn't
 * want a 1000-line tail to push the rest of the chat off-screen.
 *
 * **No PTY input.** This is read-only: we don't bind keyboard
 * events. The captain reads but can't type into the agent's
 * shell. Selection + scrollback work normally.
 */
import '@xterm/xterm/css/xterm.css'
import { FitAddon } from '@xterm/addon-fit'
import { Terminal } from '@xterm/xterm'
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'

const props = withDefaults(
  defineProps<{
    /** Accumulated terminal text (ANSI included). Append-only is fast; wholesale rewrites trigger a reset. */
    text: string
    /** Visible height in terminal rows. Default sized to a reasonable chat-inline window. */
    rows?: number
    /** Whether the underlying tool is still running — drives the live cursor + auto-scroll behavior. */
    running?: boolean
  }>(),
  {
    rows: 16,
    running: false
  }
)

const hostEl = ref<HTMLElement>()
let term: Terminal | undefined
let fit: FitAddon | undefined
let resizeObs: ResizeObserver | undefined
let written = 0

function buildTerminal(): Terminal {
  const t = new Terminal({
    rows: props.rows,
    convertEol: true,
    cursorBlink: false,
    cursorStyle: 'underline',
    disableStdin: true,
    scrollback: 5000,
    fontFamily: getComputedStyle(document.documentElement).getPropertyValue('--theme-font-mono').trim() || 'monospace',
    fontSize: 12,
    theme: {
      background: getComputedStyle(document.documentElement).getPropertyValue('--theme-surface-bg').trim() || '#0e0e10',
      foreground: getComputedStyle(document.documentElement).getPropertyValue('--theme-fg-subtle').trim() || '#c5c8c6'
    }
  })

  return t
}

/// Apply the latest `text` prop to the terminal. Append-only when
/// the prop extends what we've already written; reset + replay
/// otherwise.
function syncText(): void {
  if (!term) {
    return
  }
  const next = props.text
  const cur = next.slice(0, written)
  const previous = props.text.length > 0 ? cur : ''

  if (next.startsWith(previous) && next.length >= written) {
    const delta = next.slice(written)

    if (delta.length === 0) {
      return
    }
    term.write(delta)
    written = next.length
  } else {
    term.reset()
    term.write(next)
    written = next.length
  }
}

onMounted(() => {
  if (!hostEl.value) {
    return
  }
  term = buildTerminal()
  fit = new FitAddon()
  term.loadAddon(fit)
  term.open(hostEl.value)

  try {
    fit.fit()
  } catch {
    // Initial fit can throw when the host has zero dimensions
    // (off-screen / display:none parent at mount). The next
    // ResizeObserver fire catches up.
  }
  syncText()

  if (typeof ResizeObserver !== 'undefined') {
    resizeObs = new ResizeObserver(() => {
      try {
        fit?.fit()
      } catch {
        /* host detached or zero-sized — next observer fire retries */
      }
    })
    resizeObs.observe(hostEl.value)
  }
})

onBeforeUnmount(() => {
  resizeObs?.disconnect()
  resizeObs = undefined
  term?.dispose()
  term = undefined
  fit = undefined
  written = 0
})

watch(() => props.text, syncText)
watch(() => props.rows, (next) => {
  if (term) {
    term.resize(term.cols, next)
  }
})
</script>

<template>
  <div ref="hostEl" class="xterm-host" :data-running="running" />
</template>

<style scoped>
@reference '../assets/styles.css';

.xterm-host {
  background-color: var(--theme-surface-bg);
  width: 100%;
  min-height: 6rem;
  padding: 0.25rem 0.375rem;
  overflow: hidden;
}

/* xterm injects its own elements via JS; constrain them here. */
.xterm-host :deep(.xterm) {
  height: 100% !important;
}

.xterm-host :deep(.xterm-viewport) {
  background-color: var(--theme-surface-bg) !important;
}
</style>

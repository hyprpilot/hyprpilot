/**
 * Capture the 9 docs screenshots against the Vite dev preview
 * (`pnpm --filter hyprpilot-ui run dev:preview` at localhost:1420).
 * Viewport mirrors the actual layer-shell anchor shape — 40% of a
 * 1920×1080 desktop, full-height — so the captured frames reflect
 * what the captain sees pinned to a monitor edge.
 *
 * Each shot has a seed function that calls into `window.__hyprpilot_dev.*`
 * to mimic real claude-code/haiku state without needing a live daemon
 * + Anthropic API.
 *
 * Usage (from `docs/`):
 *   pnpm run screenshots
 *
 * Prereqs:
 *   1. `pnpm --filter hyprpilot-ui run dev:preview` running on localhost:1420.
 *   2. `pnpm install` in `docs/` (drops Playwright).
 *   3. System Brave at /usr/bin/brave OR `npx playwright install chromium`.
 *
 * Outputs land at `docs/public/screenshots/<name>.png`.
 */

import { chromium, type Page } from '@playwright/test'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = path.dirname(fileURLToPath(import.meta.url))
const OUT_DIR = path.resolve(__dirname, '../public/screenshots')
const DEV_URL = process.env.HYPRPILOT_DEV_URL ?? 'http://localhost:1420/'

// 40% × 1920 horizontal, full 1080 height — the natural anchor shape.
const VIEWPORT = { width: 768, height: 1080 }

interface Shot {
  name: string
  seed: (page: Page) => Promise<void>
}

const shots: Shot[] = [
  {
    name: 'hero',
    seed: async (page) => {
      await page.evaluate(() => {
        const dev = (window as unknown as { __hyprpilot_dev: Record<string, (...args: unknown[]) => unknown> }).__hyprpilot_dev
        const id = 'demo'
        const sid = 's-1'
        const tid = 't-1'

        dev.useActiveInstance().set(id)
        dev.pushSessionInfoUpdate(id, {
          agent: 'claude-code',
          model: 'claude-haiku-4-5',
          cwd: '/home/dev/hyprpilot',
          mode: 'default'
        })
        dev.setInstanceCwd(id, '/home/dev/hyprpilot')
        dev.pushTurnStarted(id, { sessionId: sid, turnId: tid })

        dev.pushTranscriptChunk(id, sid, {
          sessionUpdate: 'user_message_chunk',
          content: { type: 'text', text: 'refactor the permission flow to land allow/deny rules in a runtime trust store. keep MCP globs as the second lane.' }
        })
        dev.pushThoughtChunk(id, sid, {
          sessionUpdate: 'agent_thought_chunk',
          content: { type: 'text', text: 'Looking at the current permission decision pipeline. The `PermissionController::decide` fn already has a two-lane shape — runtime trust store first, MCP globs second. The trust store today is in-memory and clears on shutdown.\n\nThe captain wants persistent trust decisions. Plan: add a SQLite-backed store keyed on `(instance_id, tool_name)`, populate via the UI\'s "always" buttons.' }
        })
        dev.pushToolCall(id, 'claude-code', sid, {
          sessionUpdate: 'tool_call',
          toolCallId: 'tc-1',
          kind: 'read',
          status: 'completed',
          formatted: { title: 'read src/adapters/permission.rs', stats: [], fields: [] }
        })
        dev.pushToolCall(id, 'claude-code', sid, {
          sessionUpdate: 'tool_call',
          toolCallId: 'tc-2',
          kind: 'execute',
          status: 'running',
          formatted: { title: 'bash: cargo check', stats: [], fields: [] }
        })
      })
    }
  },
  {
    name: 'idle-screen',
    seed: async () => {
      // Idle = no instance, no transcript. The dev preview boots into
      // this state by default; just let the page render.
    }
  },
  {
    name: 'palette-root',
    seed: async (page) => {
      await page.keyboard.press('Control+KeyK')
    }
  },
  {
    name: 'palette-sessions',
    seed: async (page) => {
      // Seed a profile so sessions has an addressable agent context.
      await page.evaluate(() => {
        const dev = (window as unknown as { __hyprpilot_dev: Record<string, (...args: unknown[]) => unknown> }).__hyprpilot_dev
        dev.useActiveInstance().set('demo')
        dev.pushSessionInfoUpdate('demo', { agent: 'claude-code', model: 'claude-haiku-4-5', cwd: '/home/dev/hyprpilot' })
      })
      await page.keyboard.press('Control+KeyK')
      await page.waitForTimeout(150)
      await page.keyboard.type('sessions')
      await page.waitForTimeout(150)
      await page.keyboard.press('Enter')
    }
  },
  {
    name: 'palette-models',
    seed: async (page) => {
      await page.evaluate(() => {
        const dev = (window as unknown as { __hyprpilot_dev: Record<string, (...args: unknown[]) => unknown> }).__hyprpilot_dev
        const id = 'demo'

        dev.useActiveInstance().set(id)
        dev.pushSessionInfoUpdate(id, { agent: 'claude-code', model: 'claude-haiku-4-5', cwd: '/home/dev/hyprpilot' })
        dev.pushInstanceModelState(id, {
          currentModelId: 'claude-haiku-4-5',
          availableModels: [
            { id: 'claude-opus-4-5', name: 'Claude Opus 4.5', description: 'Most capable; slower' },
            { id: 'claude-sonnet-4-5', name: 'Claude Sonnet 4.5', description: 'Balanced default' },
            { id: 'claude-haiku-4-5', name: 'Claude Haiku 4.5', description: 'Fast; cheaper' }
          ]
        })
      })
      await page.keyboard.press('Control+KeyK')
      await page.waitForTimeout(150)
      await page.keyboard.type('models')
      await page.waitForTimeout(150)
      await page.keyboard.press('Enter')
    }
  },
  {
    name: 'chat-tool-pills',
    seed: async (page) => {
      await page.evaluate(() => {
        const dev = (window as unknown as { __hyprpilot_dev: Record<string, (...args: unknown[]) => unknown> }).__hyprpilot_dev
        const id = 'demo'
        const sid = 's-1'
        const tid = 't-1'

        dev.useActiveInstance().set(id)
        dev.pushSessionInfoUpdate(id, { agent: 'claude-code', model: 'claude-haiku-4-5', cwd: '/home/dev/hyprpilot' })
        dev.pushTurnStarted(id, { sessionId: sid, turnId: tid })
        dev.pushTranscriptChunk(id, sid, {
          sessionUpdate: 'user_message_chunk',
          content: { type: 'text', text: 'audit the permission decision pipeline for race conditions' }
        })
        dev.pushTranscriptChunk(id, sid, {
          sessionUpdate: 'agent_message_chunk',
          content: { type: 'text', text: 'I\'ll trace the permission flow end-to-end.' }
        })
        dev.pushToolCall(id, 'claude-code', sid, {
          sessionUpdate: 'tool_call',
          toolCallId: 'tc-1',
          kind: 'read',
          status: 'completed',
          formatted: { title: 'read src/adapters/permission.rs', stats: [], fields: [] }
        })
        dev.pushToolCall(id, 'claude-code', sid, {
          sessionUpdate: 'tool_call',
          toolCallId: 'tc-2',
          kind: 'edit',
          status: 'completed',
          formatted: { title: 'edit src/adapters/permission.rs', stats: [{ kind: 'diff', added: 12, removed: 3 }], fields: [] }
        })
        dev.pushToolCall(id, 'claude-code', sid, {
          sessionUpdate: 'tool_call',
          toolCallId: 'tc-3',
          kind: 'execute',
          status: 'completed',
          formatted: { title: 'bash: cargo check', stats: [{ kind: 'duration', ms: 2400 }], fields: [] }
        })
      })
    }
  },
  {
    name: 'permission-modal',
    seed: async (page) => {
      await page.evaluate(() => {
        const dev = (window as unknown as { __hyprpilot_dev: Record<string, (...args: unknown[]) => unknown> }).__hyprpilot_dev
        const id = 'demo'
        const sid = 's-1'

        dev.useActiveInstance().set(id)
        dev.pushSessionInfoUpdate(id, { agent: 'claude-code', model: 'claude-haiku-4-5', cwd: '/home/dev/hyprpilot' })
        dev.pushTurnStarted(id, { sessionId: sid, turnId: 't-1' })
        dev.pushTranscriptChunk(id, sid, {
          sessionUpdate: 'user_message_chunk',
          content: { type: 'text', text: 'clean up the build artifacts' }
        })
        dev.pushPermissionRequest(id, sid, {
          requestId: 'r-1',
          tool: 'Bash',
          kind: 'execute',
          rawInput: { command: 'rm -rf target/debug' },
          options: [
            { optionId: 'allow_once', name: 'Allow once', kind: 'allow_once' },
            { optionId: 'allow_always', name: 'Allow always', kind: 'allow_always' },
            { optionId: 'reject', name: 'Deny', kind: 'reject_once' }
          ],
          formatted: { title: 'bash: rm -rf target/debug', stats: [], fields: [] }
        })
      })
    }
  },
  {
    name: 'composer-autocomplete',
    seed: async (page) => {
      await page.evaluate(() => {
        const dev = (window as unknown as { __hyprpilot_dev: Record<string, (...args: unknown[]) => unknown> }).__hyprpilot_dev
        const id = 'demo'
        const sid = 's-1'

        dev.useActiveInstance().set(id)
        dev.pushSessionInfoUpdate(id, { agent: 'claude-code', model: 'claude-haiku-4-5', cwd: '/home/dev/hyprpilot' })
        // Push a transcript so the chat surface (not idle) renders.
        dev.pushTurnStarted(id, { sessionId: sid, turnId: 't-1' })
        dev.pushTranscriptChunk(id, sid, {
          sessionUpdate: 'user_message_chunk',
          content: { type: 'text', text: 'how do I shape a good commit message?' }
        })
        dev.pushTranscriptChunk(id, sid, {
          sessionUpdate: 'agent_message_chunk',
          content: { type: 'text', text: 'I can attach the `git-commit` skill which captures the conventional-commit conventions. Type `#` to pick it.' }
        })
      })
      const composer = page.locator('textarea').first()

      await composer.click()
      // Type char-by-char so each keystroke fires the composer's
      // input handler — the sigil-triggered popover only opens on
      // a real `input` event with a `#` at word boundary.
      await composer.pressSequentially('look at #git', { delay: 80 })
      await page.waitForTimeout(800)
    }
  },
  {
    name: 'multi-instance-header',
    seed: async (page) => {
      await page.evaluate(() => {
        const dev = (window as unknown as { __hyprpilot_dev: Record<string, (...args: unknown[]) => unknown> }).__hyprpilot_dev
        const ids = [
          { id: 'inst-a', model: 'claude-opus-4-5', cwd: '/home/dev/hyprpilot' },
          { id: 'inst-b', model: 'claude-haiku-4-5', cwd: '/home/dev/blog' },
          { id: 'inst-c', model: 'claude-sonnet-4-5', cwd: '/home/dev/dotfiles' }
        ]

        ids.forEach((i) => {
          dev.pushSessionInfoUpdate(i.id, { agent: 'claude-code', model: i.model, cwd: i.cwd })
          dev.setInstanceCwd(i.id, i.cwd)
        })

        const focused = ids[0].id

        dev.useActiveInstance().set(focused)
        // Push a transcript on the focused instance so the chat
        // surface renders (idle screen otherwise hides the header
        // breadcrumbs).
        dev.pushTurnStarted(focused, { sessionId: 's-1', turnId: 't-1' })
        dev.pushTranscriptChunk(focused, 's-1', {
          sessionUpdate: 'user_message_chunk',
          content: { type: 'text', text: 'unify the dotfiles + blog deploys behind one taskfile' }
        })
        dev.pushThoughtChunk(focused, 's-1', {
          sessionUpdate: 'agent_thought_chunk',
          content: { type: 'text', text: 'Looking at how the three repos currently build…' }
        })
      })
    }
  }
]

async function main(): Promise<void> {
  const executablePath = process.env.HYPRPILOT_DOCS_BROWSER
    ?? (process.env.CI ? undefined : '/usr/bin/brave')

  const browser = await chromium.launch({ executablePath })
  const ctx = await browser.newContext({ viewport: VIEWPORT, deviceScaleFactor: 1 })
  const page = await ctx.newPage()

  for (const shot of shots) {
    await page.goto(DEV_URL, { waitUntil: 'networkidle' })
    await page.waitForTimeout(400)
    await shot.seed(page)
    await page.waitForTimeout(500)
    const out = path.join(OUT_DIR, `${shot.name}.png`)

    await page.screenshot({ path: out, fullPage: false })
    console.log(`captured ${shot.name} → ${out}`)
  }

  await ctx.close()
  await browser.close()
}

main().catch((err) => {
  console.error(err)
  process.exit(1)
})

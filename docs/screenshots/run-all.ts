/**
 * Capture the 9 docs screenshots at 2560×1440 against the Vite dev
 * preview (`pnpm --filter hyprpilot-ui dev` at localhost:1420). Each
 * shot has a seed function that calls into `window.__hyprpilot_dev.*`
 * to mimic real claude-code/haiku state without needing a live daemon
 * + Anthropic API.
 *
 * Usage (from `docs/`):
 *   pnpm run screenshots
 *
 * Prereqs:
 *   1. `pnpm --filter hyprpilot-ui dev` running on localhost:1420.
 *   2. `pnpm install` in `docs/` (drops Playwright + Chromium).
 *
 * Outputs land at `docs/public/screenshots/<name>.png`.
 */

import { chromium, type Page } from '@playwright/test'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = path.dirname(fileURLToPath(import.meta.url))
const OUT_DIR = path.resolve(__dirname, '../public/screenshots')
const DEV_URL = process.env.HYPRPILOT_DEV_URL ?? 'http://localhost:1420/'

const VIEWPORT = { width: 2560, height: 1440 }

interface Shot {
  name: string
  seed: (page: Page) => Promise<void>
}

const shots: Shot[] = [
  {
    name: 'hero',
    seed: async (page) => {
      await page.evaluate(() => {
        const dev = (window as any).__hyprpilot_dev
        const id = 'demo'
        dev.useActiveInstance().set(id)
        dev.pushSessionInfoUpdate(id, {
          agent: 'claude-code',
          model: 'claude-haiku-4-5',
          cwd: '~/dev/hyprpilot'
        })
        dev.pushTurnStarted(id, { sessionId: 's', turnId: 't1' })
        dev.pushTranscriptChunk(id, 's', {
          sessionUpdate: 'user_message_chunk',
          content: { type: 'text', text: 'refactor the permission flow to land allow/deny rules in a runtime trust store' }
        })
        dev.pushThoughtChunk(id, 's', {
          sessionUpdate: 'agent_thought_chunk',
          content: { type: 'text', text: 'Looking at the current permission decision pipeline...' }
        })
      })
    }
  },
  {
    name: 'idle-screen',
    seed: async (page) => {
      await page.evaluate(() => {
        const dev = (window as any).__hyprpilot_dev
        dev.useActiveInstance().clear?.()
      })
    }
  },
  {
    name: 'palette-root',
    seed: async (page) => {
      await page.keyboard.press('Control+K')
    }
  },
  {
    name: 'palette-sessions',
    seed: async (page) => {
      await page.keyboard.press('Control+K')
      await page.keyboard.type('sessions')
      await page.keyboard.press('Enter')
    }
  },
  {
    name: 'palette-models',
    seed: async (page) => {
      await page.keyboard.press('Control+K')
      await page.keyboard.type('models')
      await page.keyboard.press('Enter')
    }
  },
  {
    name: 'chat-tool-pills',
    seed: async (page) => {
      await page.evaluate(() => {
        const dev = (window as any).__hyprpilot_dev
        const id = 'demo'
        dev.useActiveInstance().set(id)
        dev.pushTurnStarted(id, { sessionId: 's', turnId: 't1' })
        dev.pushToolCall(id, 'claude-code', 's', {
          sessionUpdate: 'tool_call',
          toolCallId: 'tc1',
          kind: 'execute',
          status: 'completed',
          rawInput: { command: 'ls -la /tmp' },
          formatted: { title: 'bash', stats: [{ kind: 'duration', ms: 240 }], fields: [] }
        })
        dev.pushToolCall(id, 'claude-code', 's', {
          sessionUpdate: 'tool_call',
          toolCallId: 'tc2',
          kind: 'edit',
          status: 'completed',
          formatted: { title: 'edit src/main.rs', stats: [{ kind: 'diff', added: 12, removed: 3 }], fields: [] }
        })
        dev.pushToolCall(id, 'claude-code', 's', {
          sessionUpdate: 'tool_call',
          toolCallId: 'tc3',
          kind: 'read',
          status: 'completed',
          formatted: { title: 'read CLAUDE.md', stats: [], fields: [] }
        })
      })
    }
  },
  {
    name: 'permission-modal',
    seed: async (page) => {
      await page.evaluate(() => {
        const dev = (window as any).__hyprpilot_dev
        const id = 'demo'
        dev.useActiveInstance().set(id)
        dev.pushPermissionRequest(id, 's', {
          requestId: 'r1',
          tool: 'Bash',
          kind: 'execute',
          rawInput: { command: 'rm -rf node_modules' },
          options: [
            { optionId: 'allow_once', name: 'Allow once', kind: 'allow' },
            { optionId: 'allow_always', name: 'Allow always', kind: 'allow' },
            { optionId: 'reject', name: 'Deny', kind: 'reject' }
          ],
          formatted: { title: 'bash: rm -rf node_modules', stats: [], fields: [] }
        })
      })
    }
  },
  {
    name: 'composer-autocomplete',
    seed: async (page) => {
      const composer = page.locator('textarea').first()
      await composer.click()
      await composer.type('look at #git-c')
    }
  },
  {
    name: 'multi-instance-header',
    seed: async (page) => {
      await page.evaluate(() => {
        const dev = (window as any).__hyprpilot_dev
        const ids = ['inst-a', 'inst-b', 'inst-c']
        ids.forEach((id, i) => {
          dev.pushSessionInfoUpdate(id, {
            agent: 'claude-code',
            model: i === 0 ? 'claude-opus-4-5' : 'claude-haiku-4-5',
            cwd: ['~/dev/hyprpilot', '~/dev/blog', '~/dev/dotfiles'][i]
          })
        })
        dev.useActiveInstance().set(ids[0])
      })
    }
  }
]

async function main(): Promise<void> {
  const browser = await chromium.launch()
  const ctx = await browser.newContext({ viewport: VIEWPORT, deviceScaleFactor: 1 })
  const page = await ctx.newPage()

  for (const shot of shots) {
    await page.goto(DEV_URL, { waitUntil: 'networkidle' })
    await page.waitForTimeout(300)
    await shot.seed(page)
    await page.waitForTimeout(400)
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

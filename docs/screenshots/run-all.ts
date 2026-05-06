/**
 * Capture the 9 docs screenshots against the Vite dev preview
 * (`pnpm --filter hyprpilot-ui run dev:preview` at localhost:1420).
 * Viewport mirrors the actual layer-shell anchor shape — 40% of a
 * 1920×1080 desktop, full-height — so the captured frames reflect
 * what the captain sees pinned to a monitor edge.
 *
 * Each seed extends the dev preview's pre-seeded `preview-instance`
 * state (mode `plan`, model `claude-opus-4`, cwd `~/dev/hyprpilot`,
 * git status, title) and adds:
 *   - profile + mcps count via `setInstanceProfile` / `setInstanceMcpsCount`
 *   - completed turns via `pushTurnEnded` so elapsed chips render
 *   - per-tool stats (`diff`, `duration`) so mini-pills render
 *   - permission requests with proper `tool` + `kind` so PermissionUi
 *     resolution lands them on row vs modal correctly
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

const VIEWPORT = { width: 768, height: 1080 }

interface Shot {
  name: string
  seed: (page: Page) => Promise<void>
}

const PREVIEW_ID = 'preview-instance'
const PREVIEW_SESSION = 'preview-session'

async function enrichBase(page: Page): Promise<void> {
  await page.evaluate(({ id }) => {
    const dev = (window as unknown as { __hyprpilot_dev: Record<string, (...args: unknown[]) => unknown> }).__hyprpilot_dev

    dev.setInstanceProfile(id, 'captain')
    dev.setInstanceMcpsCount(id, 4)
  }, { id: PREVIEW_ID })
}

const PLAN_MARKDOWN = `## Plan

1. **Audit the permission decision pipeline** in \`PermissionController::decide\` — confirm the runtime trust store is checked before MCP globs.
2. **Persist trust decisions to disk** behind a SQLite store at \`$XDG_STATE_HOME/hyprpilot/trust.db\`. Keyed on \`(instance_id, tool_name)\`.
3. **Load trust on instance spawn** — block the actor's first \`session/prompt\` until the load completes.
4. **Migration:** existing in-memory entries vanish on restart (per captain's note that today's runtime store is intentional).

### Files

- \`src-tauri/src/adapters/permission.rs\` — wire the SQLite read at \`PermissionController::decide\`.
- \`src-tauri/src/tools/trust_store.rs\` — new module owning the schema + migrations.
- \`src-tauri/src/adapters/acp/instance.rs\` — block on trust load before first prompt.

I'll start with the new \`trust_store\` module + a unit test, then wire it into the controller.`

const shots: Shot[] = [
  {
    name: 'hero',
    seed: async (page) => {
      await enrichBase(page)
      await page.evaluate(({ id, sid }) => {
        const dev = (window as unknown as { __hyprpilot_dev: Record<string, (...args: unknown[]) => unknown> }).__hyprpilot_dev
        const tid = 't-hero'
        const turnStart = Date.now() - 32 * 1000

        dev.pushTurnStarted(id, { sessionId: sid, turnId: tid, startedAtMs: turnStart })
        dev.pushTranscriptChunk(id, sid, {
          sessionUpdate: 'user_message_chunk',
          content: { type: 'text', text: 'refactor the permission flow to land allow/deny rules in a runtime trust store. keep MCP globs as the second lane.' }
        })
        // Thought block paired with elapsed time + a follow-up agent
        // message — thoughts shouldn't render alone.
        dev.markThinkingStart(id, sid, turnStart + 600)
        dev.pushThoughtChunk(id, sid, {
          sessionUpdate: 'agent_thought_chunk',
          content: { type: 'text', text: 'Looking at the current permission decision pipeline. The `PermissionController::decide` fn already has a two-lane shape — runtime trust store first, MCP globs second.\n\nThe captain wants persistent trust decisions. Plan: add a SQLite-backed store keyed on `(instance_id, tool_name)`, populate via the UI\'s "always" buttons.' }
        })
        dev.markThinkingEnd(id, sid, turnStart + 4000)
        dev.pushTranscriptChunk(id, sid, {
          sessionUpdate: 'agent_message_chunk',
          content: { type: 'text', text: 'Got it — adding a SQLite-backed `TrustStore` under `tools/` and threading it through `PermissionController::decide` ahead of the MCP-glob lane.' }
        })
        dev.pushToolCall(id, 'claude-code', sid, {
          sessionUpdate: 'tool_call',
          toolCallId: 'tc-hero-1',
          kind: 'read',
          status: 'completed',
          startedAtMs: turnStart + 4500,
          completedAtMs: turnStart + 4680,
          formatted: { title: 'read src/adapters/permission.rs', stats: [{ kind: 'duration', ms: 180 }], fields: [] }
        })
        dev.pushToolCall(id, 'claude-code', sid, {
          sessionUpdate: 'tool_call',
          toolCallId: 'tc-hero-2',
          kind: 'execute',
          status: 'running',
          startedAtMs: turnStart + 6000,
          formatted: { title: 'bash: cargo check --tests', stats: [], fields: [] }
        })
      }, { id: PREVIEW_ID, sid: PREVIEW_SESSION })
    }
  },
  {
    name: 'idle-screen',
    seed: async (page) => {
      await page.evaluate(() => {
        const dev = (window as unknown as { __hyprpilot_dev: Record<string, (...args: unknown[]) => unknown> }).__hyprpilot_dev

        dev.useActiveInstance().clear?.()
      })
    }
  },
  {
    name: 'palette-root',
    seed: async (page) => {
      await enrichBase(page)
      await page.keyboard.press('Control+KeyK')
      await page.waitForTimeout(150)
    }
  },
  {
    name: 'palette-sessions',
    seed: async (page) => {
      await enrichBase(page)
      await page.keyboard.press('Control+KeyK')
      await page.waitForTimeout(120)
      await page.keyboard.type('sessions')
      await page.waitForTimeout(120)
      await page.keyboard.press('Enter')
      await page.waitForTimeout(500)
    }
  },
  {
    name: 'palette-models',
    seed: async (page) => {
      await enrichBase(page)
      await page.evaluate(({ id }) => {
        const dev = (window as unknown as { __hyprpilot_dev: Record<string, (...args: unknown[]) => unknown> }).__hyprpilot_dev

        dev.pushInstanceModelState(id, {
          currentModelId: 'claude-opus-4',
          availableModels: [
            { id: 'claude-opus-4-5', name: 'Claude Opus 4.5', description: 'Most capable; slower' },
            { id: 'claude-sonnet-4-5', name: 'Claude Sonnet 4.5', description: 'Balanced default' },
            { id: 'claude-haiku-4-5', name: 'Claude Haiku 4.5', description: 'Fast; cheaper' },
            { id: 'claude-opus-4', name: 'Claude Opus 4', description: 'Previous generation' }
          ]
        })
      }, { id: PREVIEW_ID })
      await page.keyboard.press('Control+KeyK')
      await page.waitForTimeout(120)
      await page.keyboard.type('models')
      await page.waitForTimeout(120)
      await page.keyboard.press('Enter')
      await page.waitForTimeout(500)
    }
  },
  {
    name: 'chat-tool-pills',
    seed: async (page) => {
      await enrichBase(page)
      await page.evaluate(({ id, sid }) => {
        const dev = (window as unknown as { __hyprpilot_dev: Record<string, (...args: unknown[]) => unknown> }).__hyprpilot_dev
        const tid = 't-tools'
        const turnStart = Date.now() - 18 * 1000
        const turnEnd = turnStart + 14_500

        dev.pushTurnStarted(id, { sessionId: sid, turnId: tid, startedAtMs: turnStart })
        dev.pushTranscriptChunk(id, sid, {
          sessionUpdate: 'user_message_chunk',
          content: { type: 'text', text: 'audit the permission decision pipeline for race conditions' }
        })
        dev.pushTranscriptChunk(id, sid, {
          sessionUpdate: 'agent_message_chunk',
          content: { type: 'text', text: 'I traced the permission flow end-to-end. The two-lane decide() is race-free; both lanes hold the same RwLock for the duration of a single decision.' }
        })
        dev.pushToolCall(id, 'claude-code', sid, {
          sessionUpdate: 'tool_call',
          toolCallId: 'tc-1',
          kind: 'read',
          status: 'completed',
          startedAtMs: turnStart + 1000,
          completedAtMs: turnStart + 1240,
          formatted: { title: 'read src/adapters/permission.rs', stats: [{ kind: 'duration', ms: 240 }], fields: [] }
        })
        dev.pushToolCall(id, 'claude-code', sid, {
          sessionUpdate: 'tool_call',
          toolCallId: 'tc-2',
          kind: 'edit',
          status: 'completed',
          startedAtMs: turnStart + 4000,
          completedAtMs: turnStart + 4090,
          formatted: { title: 'edit src/adapters/permission.rs', stats: [{ kind: 'diff', added: 12, removed: 3 }, { kind: 'duration', ms: 90 }], fields: [] }
        })
        dev.pushToolCall(id, 'claude-code', sid, {
          sessionUpdate: 'tool_call',
          toolCallId: 'tc-3',
          kind: 'execute',
          status: 'completed',
          startedAtMs: turnStart + 8000,
          completedAtMs: turnStart + 10_400,
          formatted: { title: 'bash: cargo check', stats: [{ kind: 'duration', ms: 2400 }], fields: [] }
        })
        // One mid-flight tool — `running` auto-expands the pill so the
        // body (fields + description + output) renders inline. Captures
        // the "live tool execution" shape captains see while a turn is
        // in progress.
        dev.pushToolCall(id, 'claude-code', sid, {
          sessionUpdate: 'tool_call',
          toolCallId: 'tc-4',
          kind: 'execute',
          status: 'running',
          startedAtMs: turnStart + 12_000,
          formatted: {
            title: 'bash: cargo nextest run --test-threads 4',
            stats: [],
            fields: [
              { label: 'cwd', value: '/home/dev/hyprpilot/src-tauri' },
              { label: 'command', value: 'cargo nextest run --test-threads 4' }
            ],
            description: 'Re-running the suite under reduced parallelism after the permission decision-pipeline change.',
            output: '   Compiling hyprpilot v0.1.3\n    Finished `test` profile [unoptimized + debuginfo] target(s)\n     Running unittests src/main.rs\n        PASS [   0.011s] adapters::permission::tests::trust_store_short_circuits_glob_lane'
          }
        })
        // No pushTurnEnded — the turn is still live (matches the
        // running tool above + the elapsed-clock chip).
      }, { id: PREVIEW_ID, sid: PREVIEW_SESSION })
    }
  },
  {
    name: 'permission-modal',
    seed: async (page) => {
      await enrichBase(page)
      await page.evaluate(({ id, sid, planMarkdown }) => {
        const dev = (window as unknown as { __hyprpilot_dev: Record<string, (...args: unknown[]) => unknown> }).__hyprpilot_dev
        const turnStart = Date.now() - 22 * 1000

        dev.pushTurnStarted(id, { sessionId: sid, turnId: 't-plan', startedAtMs: turnStart })
        dev.pushTranscriptChunk(id, sid, {
          sessionUpdate: 'user_message_chunk',
          content: { type: 'text', text: 'persist the trust store to disk so always-allow / always-deny survives a daemon restart' }
        })
        dev.pushThoughtChunk(id, sid, {
          sessionUpdate: 'agent_thought_chunk',
          content: { type: 'text', text: 'Today\'s in-memory store clears on instance shutdown. SQLite-backed file at $XDG_STATE_HOME/hyprpilot/trust.db is the right fit — small, single-writer, ships on every Linux distro.' }
        })
        // Modal-class permission: tool = `exit_plan_mode` → claude-code
        // override maps to PermissionUi.Modal. Description carries
        // markdown plan content rendered by ToolBody. `agentId` is
        // mandatory — the adapter override only applies when the
        // permission carries the agent so `adapterFor()` can resolve
        // the per-vendor presentation map.
        dev.pushPermissionRequest(id, sid, {
          agentId: 'claude-code',
          requestId: 'r-plan',
          tool: 'exit_plan_mode',
          kind: 'other',
          rawInput: { plan: planMarkdown },
          options: [
            { optionId: 'allow_once', name: 'Approve & exit plan mode', kind: 'allow_once' },
            { optionId: 'reject', name: 'Keep planning', kind: 'reject_once' }
          ],
          formatted: {
            title: 'exit plan mode',
            stats: [],
            fields: [],
            description: planMarkdown
          }
        })
      }, { id: PREVIEW_ID, sid: PREVIEW_SESSION, planMarkdown: PLAN_MARKDOWN })
    }
  },
  {
    name: 'composer-autocomplete',
    seed: async (page) => {
      await enrichBase(page)
      await page.evaluate(({ id, sid }) => {
        const dev = (window as unknown as { __hyprpilot_dev: Record<string, (...args: unknown[]) => unknown> }).__hyprpilot_dev
        const turnStart = Date.now() - 8000
        const turnEnd = turnStart + 4500

        dev.pushTurnStarted(id, { sessionId: sid, turnId: 't-comp', startedAtMs: turnStart })
        dev.pushTranscriptChunk(id, sid, {
          sessionUpdate: 'user_message_chunk',
          content: { type: 'text', text: 'how do I shape a good commit message?' }
        })
        dev.pushTranscriptChunk(id, sid, {
          sessionUpdate: 'agent_message_chunk',
          content: { type: 'text', text: 'Conventional commit format: `<type>(<scope>): <subject>`. Pick the type matching the change (feat / fix / refactor / chore / docs / ci). Subject is imperative mood, ≤72 chars. Attach the `git-commit` skill if you want the full convention pinned to the next prompt — type `#` to pick.' }
        })
        dev.pushTurnEnded(id, { sessionId: sid, turnId: 't-comp', endedAtMs: turnEnd, stopReason: 'end_turn' })
      }, { id: PREVIEW_ID, sid: PREVIEW_SESSION })
      const composer = page.locator('textarea').first()

      await composer.click()
      await composer.pressSequentially('cool, attach #git', { delay: 80 })
      await page.waitForTimeout(800)
    }
  },
  {
    name: 'permission-row',
    seed: async (page) => {
      await enrichBase(page)
      await page.evaluate(({ focused, sid }) => {
        const dev = (window as unknown as { __hyprpilot_dev: Record<string, (...args: unknown[]) => unknown> }).__hyprpilot_dev
        const extras = [
          { id: 'inst-blog', model: 'claude-haiku-4-5', cwd: '/home/dev/blog' },
          { id: 'inst-dotfiles', model: 'claude-sonnet-4-5', cwd: '/home/dev/dotfiles' }
        ]

        extras.forEach((i) => {
          dev.pushSessionInfoUpdate(i.id, { agent: 'claude-code', model: i.model })
          dev.setInstanceCwd(i.id, i.cwd)
          dev.setInstanceProfile(i.id, 'captain')
          dev.setInstanceMcpsCount(i.id, 2)
        })

        // Add a permission ROW to the focused instance so the screenshot
        // also showcases the inline permission strip with bash exec
        // pending a captain decision.
        const turnStart = Date.now() - 9000

        dev.pushTurnStarted(focused, { sessionId: sid, turnId: 't-multi', startedAtMs: turnStart })
        dev.pushTranscriptChunk(focused, sid, {
          sessionUpdate: 'user_message_chunk',
          content: { type: 'text', text: 'unify the dotfiles + blog deploys behind one taskfile' }
        })
        dev.pushTranscriptChunk(focused, sid, {
          sessionUpdate: 'agent_message_chunk',
          content: { type: 'text', text: 'Reading the existing Taskfiles in each repo to find the common shape.' }
        })
        dev.pushToolCall(focused, 'claude-code', sid, {
          sessionUpdate: 'tool_call',
          toolCallId: 'tc-multi-1',
          kind: 'read',
          status: 'completed',
          startedAtMs: turnStart + 1000,
          completedAtMs: turnStart + 1110,
          formatted: { title: 'read /home/dev/blog/Taskfile.yml', stats: [{ kind: 'duration', ms: 110 }], fields: [] }
        })
        // Row-class permission: kind=execute → presentation defaults
        // route bash to PermissionUi.Row, rendering inline above the
        // composer.
        dev.pushPermissionRequest(focused, sid, {
          agentId: 'claude-code',
          requestId: 'r-bash',
          tool: 'bash',
          kind: 'execute',
          rawInput: { command: 'cd /home/dev/dotfiles && cat Taskfile.yml' },
          options: [
            { optionId: 'allow_once', name: 'Allow once', kind: 'allow_once' },
            { optionId: 'allow_always', name: 'Allow always', kind: 'allow_always' },
            { optionId: 'reject', name: 'Deny', kind: 'reject_once' }
          ],
          formatted: {
            title: 'bash: cat /home/dev/dotfiles/Taskfile.yml',
            stats: [],
            fields: [
              { label: 'cwd', value: '/home/dev/dotfiles' },
              { label: 'command', value: 'cat Taskfile.yml' }
            ],
            description: 'Reading the dotfiles repo\'s Taskfile so I can compare its install / format / lint shape against the blog repo and propose a unified one.'
          }
        })
      }, { focused: PREVIEW_ID, sid: PREVIEW_SESSION })
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

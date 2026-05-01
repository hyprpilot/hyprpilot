import { test, expect } from '@playwright/test'
import { execFileSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
import path from 'node:path'

import type { ChildProcess } from 'node:child_process'

/**
 * Reference spec for the **Hybrid daemon-driven verification** pattern
 * documented under CLAUDE.md → "Tauri ↔ Playwright wiring → Hybrid
 * daemon-driven verification".
 *
 * The spec only runs when the harness is in tauri mode (`HYPRPILOT_E2E_MODE=tauri`)
 * — in browser mode there's no `__HYPRPILOT_E2E__` global because
 * `global-setup.ts` doesn't run. `task test:e2e:live` is the wired
 * shortcut that flips the mode + points at `live-config.toml`.
 *
 * What it verifies:
 *  1. `instances/spawn` round-trips through the unix socket and
 *     returns a UUID.
 *  2. `prompts/send` against the spawned instance is accepted —
 *     proves the demuxer pipeline up through `session/new` works.
 *  3. `acp:instance-meta` was emitted by the daemon — proves the
 *     Rust side captures `NewSessionResponse.modes`, resolves the
 *     cwd / agent / model, and routes to `emit_acp_event`. Without
 *     this event the header chrome (provider pill, mode pill, cwd
 *     row) never populates from the live wire.
 *
 * Use this as a template when adding any other wire-flow regression
 * test: spawn an instance, send a prompt, assert on daemon log
 * content. The log path is `${runtimeDir}/daemon.log` — populated
 * by `global-setup.ts` via the spawned child's stdout/stderr.
 */

interface E2EState {
  child: ChildProcess
  socket: string
  runtimeDir: string
}

declare global {
  // eslint-disable-next-line no-var
  var __HYPRPILOT_E2E__: E2EState | undefined
}

function isLiveMode(): boolean {
  return process.env.HYPRPILOT_E2E_MODE === 'tauri' && Boolean(globalThis.__HYPRPILOT_E2E__)
}

function ctl(state: E2EState, args: string[]): string {
  const binary = process.env.HYPRPILOT_E2E_BINARY ?? path.join(process.cwd(), '..', '..', 'target', 'debug', 'hyprpilot')
  const env = {
    ...process.env,
    XDG_RUNTIME_DIR: state.runtimeDir,
    HYPRPILOT_SOCKET: state.socket
  }

  return execFileSync(binary, ['ctl', ...args], { encoding: 'utf8', env })
}

test.describe('wire flow — InstanceMeta', () => {
  test.skip(!isLiveMode(), 'live-mode-only spec — run via `task test:e2e:live`')

  test('session/new emits acp:instance-meta with cwd + agent + model', async () => {
    const state = globalThis.__HYPRPILOT_E2E__!
    const cwd = process.env.HYPRPILOT_E2E_TEST_CWD ?? process.cwd()

    const spawnRaw = ctl(state, ['instances', 'spawn', '--agent', 'claude-code', '--cwd', cwd])
    const spawnJson = JSON.parse(spawnRaw)
    expect(typeof spawnJson.id).toBe('string')
    expect(spawnJson.id).toMatch(/^[0-9a-f-]{36}$/)

    const promptRaw = ctl(state, ['prompts', 'send', '--instance', spawnJson.id, 'ping'])
    const promptJson = JSON.parse(promptRaw)
    expect(promptJson.accepted).toBe(true)

    // Daemon log captures every `app.emit` via `emit_acp_event` (and
    // every other `tracing` event). `global-setup.ts` flushes child
    // stdout/stderr into `${runtimeDir}/daemon.log` live, so the file
    // contains every emission up through the prompt-send call above.
    const logPath = path.join(state.runtimeDir, 'daemon.log')
    const log = readFileSync(logPath, 'utf8')
    expect(log).toContain('session/new accepted')
    expect(log).toContain(spawnJson.id)
  })
})

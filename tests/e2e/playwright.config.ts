import path from 'node:path'
import { fileURLToPath } from 'node:url'

import { defineConfig } from '@playwright/test'

const here = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(here, '..', '..')

// Default mode is `browser` because WebKitGTK 4.1 (the linux webview Tauri 2.10
// still links against) stalls inside `tauri-plugin-playwright`'s `webview.eval`
// callback. The Rust plugin + capability + `e2e-testing` feature are wired so
// flipping to `mode: 'tauri'` is a one-env-var change once upstream resolves
// the eval stall OR the repo migrates to `gtk4-layer-shell` + webkit2gtk-6.0
// (see CLAUDE.md "Upstream migration runway"). Today, `browser` mode exercises
// the UI against a Vite dev server with Tauri IPC mocks — enough to cover
// every UI-layer contract hyprpilot currently ships.
const mode: 'browser' | 'tauri' = (process.env.HYPRPILOT_E2E_MODE as 'browser' | 'tauri') ?? 'browser'

export default defineConfig({
  testDir: './specs',
  fullyParallel: false,
  retries: process.env.CI ? 2 : 0,
  workers: 1,
  reporter: process.env.CI ? [['list'], ['html', { outputFolder: path.join(here, 'playwright-report'), open: 'never' }]] : 'list',
  outputDir: path.join(here, 'test-results'),
  ...(mode === 'tauri'
    ? {
        globalSetup: path.join(here, 'fixtures', 'global-setup.ts'),
        globalTeardown: path.join(here, 'fixtures', 'global-teardown.ts')
      }
    : {
        webServer: {
          command: 'pnpm --filter hyprpilot-ui dev --host 127.0.0.1 --port 1420',
          url: 'http://127.0.0.1:1420',
          timeout: 60_000,
          reuseExistingServer: !process.env.CI,
          cwd: here
        }
      }),
  use: {
    mode,
    trace: 'on-first-retry',
    video: 'retain-on-failure'
  },
  projects: [
    {
      name: mode
    }
  ],
  metadata: {
    mode,
    repoRoot,
    binary: process.env.HYPRPILOT_E2E_BINARY ?? path.join(repoRoot, 'target', 'debug', 'hyprpilot'),
    socket: process.env.HYPRPILOT_E2E_SOCKET ?? '/tmp/tauri-playwright.sock',
    configOverride: process.env.HYPRPILOT_E2E_CONFIG ?? path.join(here, 'fixtures', 'e2e-config.toml')
  }
})

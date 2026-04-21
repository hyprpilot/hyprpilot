import { test, expect } from '@playwright/test'

test.describe('hyprpilot overlay', () => {
  test.skip('placeholder view renders in the Tauri webview', async ({ page: _page }) => {
    // TODO: wire Playwright up to `tauri-driver` + WebKitGTK's WebDriver shim
    // so we can boot the packaged binary and drive the real webview. Leaving
    // a skipped test (rather than a failing one) so the Playwright runtime
    // stays exercised by `task test`.
    expect(true).toBe(true)
  })
})

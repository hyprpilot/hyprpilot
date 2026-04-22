import { getCapturedInvokes } from '@srsholmes/tauri-playwright'

import { expect, test } from '../fixtures/tauri'

test('webview boot calls get_theme and get_window_state via the IPC bridge', async({ tauriPage }) => {
  const page = (tauriPage as { playwrightPage: import('@playwright/test').Page }).playwrightPage

  await page.waitForLoadState('networkidle')

  const calls = await getCapturedInvokes(page)
  const commands = calls.map((c: { cmd: string }) => c.cmd)

  expect(commands).toContain('get_theme')
  expect(commands).toContain('get_window_state')
})

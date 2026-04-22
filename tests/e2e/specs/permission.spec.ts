import { emitMockEvent } from '@srsholmes/tauri-playwright'

import { expect, test } from '../fixtures/tauri'

test('acp:permission-request mock events reach the webview listener chain', async ({ tauriPage }) => {
  const page = (tauriPage as { playwrightPage: import('@playwright/test').Page }).playwrightPage
  await page.waitForLoadState('networkidle')

  // Register a capture listener through the Tauri event IPC call (which the
  // bridge's ipc-mock script intercepts via `plugin:event|listen`) so the
  // subsequent `emitMockEvent` reaches a real subscriber. This exercises
  // the exact wire `useAcpAgent` subscribes on.
  await page.evaluate(() => {
    const captured: unknown[] = []
    ;(window as unknown as { __captured: unknown[] }).__captured = captured
    const handlerId = Math.floor(Math.random() * 1_000_000)
    ;(window as unknown as Record<string, unknown>)[`_${handlerId}`] = (evt: { payload: unknown }) => {
      captured.push(evt.payload)
    }
    const listeners = ((window as unknown as { __TAURI_MOCK_LISTENERS__: Record<string, number[]> }).__TAURI_MOCK_LISTENERS__ ??= {})
    ;(listeners['acp:permission-request'] ??= []).push(handlerId)
  })

  await emitMockEvent(page, 'acp:permission-request', {
    request_id: 'req-1',
    tool_name: 'shell.execute',
    options: [
      { id: 'allow-once', label: 'Allow once', kind: 'allow' },
      { id: 'reject', label: 'Reject', kind: 'deny' }
    ],
    raw_input: { cmd: 'ls /tmp' }
  })

  const captured = await page.evaluate(() => (window as unknown as { __captured: unknown[] }).__captured)
  expect(captured).toHaveLength(1)
  expect(captured[0]).toMatchObject({ request_id: 'req-1', tool_name: 'shell.execute' })
})

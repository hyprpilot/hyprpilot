import { expect, test } from '../fixtures/tauri'

test('overlay webview loads with the chat shell mounted', async ({ tauriPage }) => {
  await expect(tauriPage).toHaveTitle('hyprpilot')

  const chat = tauriPage.getByTestId('chat')
  await expect(chat).toBeVisible()

  await expect(tauriPage.getByTestId('composer-textarea')).toBeVisible()
  await expect(tauriPage.getByTestId('composer-submit')).toBeVisible()
  await expect(tauriPage.getByTestId('status-strip')).toBeVisible()
})

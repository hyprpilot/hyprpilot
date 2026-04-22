import { expect, test } from '../fixtures/tauri'

test('overlay webview loads with the expected title and placeholder', async ({ tauriPage }) => {
  await expect(tauriPage).toHaveTitle('hyprpilot')

  const placeholder = tauriPage.getByTestId('placeholder')
  await expect(placeholder).toContainText('hyprpilot')

  await expect(tauriPage.getByTestId('placeholder-textarea')).toBeVisible()
  await expect(tauriPage.getByTestId('placeholder-submit')).toBeVisible()
  await expect(tauriPage.getByTestId('placeholder-cancel')).toBeVisible()
})

import { expect, test } from '../fixtures/tauri'

/**
 * wireframe fidelity specs — exercise the visual states the agent
 * verified manually via Playwright MCP. Same browser-mode harness +
 * IPC mocks as the smoke spec; what's specific here is asserting on
 * the wireframe-spec primitives (phase border, role lanes, palette shell,
 * idle wordmark, kbd legend) so a regression caught by `task test:e2e`
 * has a named owner.
 *
 * Real-app (mode: 'tauri') gates apply once the WebKitGTK eval-stall
 * clears — see CLAUDE.md "Tauri ↔ Playwright wiring" for the
 * upgrade path.
 */

test.describe('idle screen', () => {
  test('renders the wordmark + LFG accent + 4-row kbd legend', async({ tauriPage }) => {
    const page = (tauriPage as { playwrightPage: import('@playwright/test').Page }).playwrightPage

    await page.waitForLoadState('networkidle')

    const idle = page.getByTestId('idle-screen')

    await expect(idle).toBeVisible()
    await expect(idle).toContainText('hyprpilot')
    await expect(idle).toContainText('LFG.')

    // Each row of the kbd legend = a `<span class="idle-kbd">` keycap +
    // a `<span class="idle-kbd-label">` description. Four pairs cover
    // the legend the wireframe specifies.
    const keycaps = idle.locator('.idle-kbd')

    await expect(keycaps).toHaveCount(4)
    await expect(keycaps.nth(0)).toHaveText('Ctrl+K')
    await expect(keycaps.nth(1)).toHaveText('@')
    await expect(keycaps.nth(2)).toHaveText('+')
    await expect(keycaps.nth(3)).toHaveText('Esc')
  })

  test('phase border + profile pill share the idle color', async({ tauriPage }) => {
    const page = (tauriPage as { playwrightPage: import('@playwright/test').Page }).playwrightPage

    await page.waitForLoadState('networkidle')

    const frame = page.getByTestId('frame')

    await expect(frame).toBeVisible()

    // Visual law #1 — phase color paints both the 3px left frame stripe
    // and the profile pill bg. Idle = green (`--theme-state-idle`).
    const colors = await frame.evaluate((el) => {
      const cs = getComputedStyle(el)
      const pill = el.querySelector<HTMLElement>('.frame-profile-pill')
      const pillBg = pill ? getComputedStyle(pill).backgroundColor : ''

      return { borderLeftColor: cs.borderLeftColor, pillBg }
    })

    // Both should resolve the green idle token. Compare normalized rgb().
    expect(colors.borderLeftColor).toBe(colors.pillBg)
    expect(colors.borderLeftColor).toMatch(/152|151|153/) // ~rgb(152, 195, 121) for #98c379
  })
})

test.describe('wireframe command palette', () => {
  test('Ctrl+K opens the palette with the 11 root categories', async({ tauriPage }) => {
    const page = (tauriPage as { playwrightPage: import('@playwright/test').Page }).playwrightPage

    await page.waitForLoadState('networkidle')

    // Frame must be focusable for the document keydown to route — the
    // palette's onDocumentKeyDown listener fires capture-phase from
    // `document`, but the keydown needs an actual focused element.
    await page.locator('body').focus()
    await page.keyboard.press('Control+K')

    const palette = page.getByTestId('palette-frame')

    await expect(palette).toBeVisible()

    // 11 root categories per the wireframe spec (sessions, profiles,
    // models, modes, commands, cwd, instances, permissions, skills,
    // references, mcps).
    const rows = palette.locator('.palette-row')

    await expect(rows).toHaveCount(11)

    // Footer kbd hints — at least navigate / confirm / close.
    const footer = palette.locator('.palette-footer')

    await expect(footer).toContainText('navigate')
    await expect(footer).toContainText('confirm')
    await expect(footer).toContainText('close')
  })

  test('Escape closes the palette', async({ tauriPage }) => {
    const page = (tauriPage as { playwrightPage: import('@playwright/test').Page }).playwrightPage

    await page.waitForLoadState('networkidle')

    await page.locator('body').focus()
    await page.keyboard.press('Control+K')
    await expect(page.getByTestId('palette-frame')).toBeVisible()

    await page.keyboard.press('Escape')
    await expect(page.getByTestId('palette-frame')).toHaveCount(0)
  })
})

test.describe('composer', () => {
  test('renders the textarea + send + attach 44px button cluster', async({ tauriPage }) => {
    const page = (tauriPage as { playwrightPage: import('@playwright/test').Page }).playwrightPage

    await page.waitForLoadState('networkidle')

    const textarea = page.getByTestId('composer-textarea')
    const submit = page.getByTestId('composer-submit')
    const attach = page.getByTestId('composer-attach')

    await expect(textarea).toBeVisible()
    await expect(textarea).toHaveAttribute('placeholder', 'message pilot')
    await expect(submit).toBeVisible()
    await expect(attach).toBeVisible()

    // 44px cluster width per wireframe spec — the parent `.composer-actions`
    // div is the column carrying both buttons.
    const actionsWidth = await page.locator('.composer-actions').evaluate((el) => el.getBoundingClientRect().width)

    expect(actionsWidth).toBe(44)
  })

  test('send button starts ghost (empty composer), goes solid yellow on text input', async({ tauriPage }) => {
    const page = (tauriPage as { playwrightPage: import('@playwright/test').Page }).playwrightPage

    await page.waitForLoadState('networkidle')

    const submit = page.getByTestId('composer-submit')

    // Empty composer = ghost: data-empty="true", transparent bg.
    await expect(submit).toHaveAttribute('data-empty', 'true')

    await page.getByTestId('composer-textarea').fill('hello pilot')

    // Filled composer = solid yellow: data-empty="false".
    await expect(submit).toHaveAttribute('data-empty', 'false')
  })
})

import { FontAwesomeIcon } from '@fortawesome/vue-fontawesome'
import { cleanup } from '@testing-library/vue'
import { config, enableAutoUnmount } from '@vue/test-utils'
import { afterEach, vi } from 'vitest'

// jsdom omits `window.matchMedia` (it's a CSS Object Model API,
// not part of the DOM). xterm.js consumes it during construction
// to detect HiDPI; without a stub the terminal-rendering tests
// throw "this._parentWindow.matchMedia is not a function" at
// `term.open(...)`. Stub returns "no match" — xterm falls back
// to its default DPR handling, which is fine for tests that
// don't depend on resolution.
if (typeof window !== 'undefined' && !window.matchMedia) {
  window.matchMedia = vi.fn().mockImplementation((query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    addListener: vi.fn(),
    removeListener: vi.fn(),
    dispatchEvent: vi.fn().mockReturnValue(false)
  }))
}

// Tests bind `<FaIcon>` to the FontAwesome component globally so call
// sites pass an imported `IconDefinition` directly via `:icon="faFoo"`
// — no central `library.add(...)` registry, mirroring production
// per the no-`library.add` rule (CLAUDE.md / AGENTS.md).
config.global.components = { ...(config.global.components ?? {}), FaIcon: FontAwesomeIcon }

enableAutoUnmount(afterEach)

afterEach(() => {
  cleanup()
})

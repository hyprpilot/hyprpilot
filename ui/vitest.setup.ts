import { FontAwesomeIcon } from '@fortawesome/vue-fontawesome'
import { cleanup } from '@testing-library/vue'
import { config, enableAutoUnmount } from '@vue/test-utils'
import { afterEach } from 'vitest'

// Tests bind `<FaIcon>` to the FontAwesome component globally so call
// sites pass an imported `IconDefinition` directly via `:icon="faFoo"`
// — no central `library.add(...)` registry, mirroring production
// per the no-`library.add` rule (CLAUDE.md / AGENTS.md).
config.global.components = { ...(config.global.components ?? {}), FaIcon: FontAwesomeIcon }

enableAutoUnmount(afterEach)

afterEach(() => {
  cleanup()
})

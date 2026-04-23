import '@testing-library/vue'
import { library } from '@fortawesome/fontawesome-svg-core'
import { faCircle as farCircle, faSquare as farSquare } from '@fortawesome/free-regular-svg-icons'
import {
  faArrowRightToBracket,
  faArrowTurnDown,
  faBrain,
  faChevronDown,
  faChevronRight,
  faCircle,
  faCircleCheck,
  faCircleHalfStroke,
  faCircleInfo,
  faCircleNotch,
  faCircleXmark,
  faCodeBranch,
  faCube,
  faFileLines,
  faMagnifyingGlass,
  faPen,
  faPenToSquare,
  faPlug,
  faPlay,
  faReply,
  faSquareCheck,
  faTerminal,
  faTriangleExclamation,
  faUpDown,
  faUserGear,
  faXmark
} from '@fortawesome/free-solid-svg-icons'
import { FontAwesomeIcon } from '@fortawesome/vue-fontawesome'
import { cleanup } from '@testing-library/vue'
import { config } from '@vue/test-utils'
import { afterEach } from 'vitest'

// Mirror the production library in main.ts; `faPlay` is test-only (used
// in CommandPaletteRow.test.ts as a "renders icon" marker).
library.add(
  faArrowRightToBracket,
  faArrowTurnDown,
  faBrain,
  faChevronDown,
  faChevronRight,
  faCircle,
  faCircleCheck,
  faCircleHalfStroke,
  faCircleInfo,
  faCircleNotch,
  faCircleXmark,
  faCodeBranch,
  faCube,
  faFileLines,
  faMagnifyingGlass,
  faPen,
  faPenToSquare,
  faPlay,
  faPlug,
  faReply,
  faSquareCheck,
  faTerminal,
  faTriangleExclamation,
  faUpDown,
  faUserGear,
  faXmark,
  farCircle,
  farSquare
)
config.global.components = { ...(config.global.components ?? {}), FaIcon: FontAwesomeIcon }

afterEach(() => {
  cleanup()
})

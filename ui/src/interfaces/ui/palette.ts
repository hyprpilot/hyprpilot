/**
 * Palette-surface UI types.
 */

import type { IconDefinition } from '@fortawesome/fontawesome-svg-core'

export interface PaletteRowItem {
  id: string
  icon?: IconDefinition
  label: string
  hint?: string
  right?: string
  danger?: boolean
}

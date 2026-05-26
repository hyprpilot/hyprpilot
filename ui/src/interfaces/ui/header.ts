/**
 * Header-surface UI types — breadcrumb count chips.
 */

/** Breadcrumb count chip: `{ label, count, color? }`. */
export interface BreadcrumbCount {
  /// Stable identifier the consumer dispatches on (`mcps` /
  /// `sessions` / …). Defaults to `label` when unset.
  id?: string
  label: string
  count: number
  color?: string
}

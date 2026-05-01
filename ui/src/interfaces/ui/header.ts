/**
 * Header-surface UI types — breadcrumb count chips + git status pill.
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

/**
 * Git status summary for the Frame cwd row. `branch` is always
 * populated when the field is set at all; `ahead` / `behind` omitted
 * (or zero) when the branch is in sync with its upstream.
 * `worktree` is the checked-out worktree name when the cwd is inside
 * a `git-worktree` checkout, else undefined.
 */
export interface GitStatus {
  branch: string
  ahead?: number
  behind?: number
  worktree?: string
}

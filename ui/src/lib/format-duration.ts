import { intervalToDuration } from 'date-fns'

/**
 * Compact duration label for tool / turn pills. Branches:
 *   `< 1000ms` → `"850ms"`
 *   `< 60s`    → `"3s"`
 *   `< 60m`    → `"1m 3s"`  (seconds dropped when zero: `"2m"`)
 *   else       → `"1h 2m"`  (minutes dropped when zero: `"1h"`)
 *
 * `date-fns`'s native `formatDuration` is too verbose for our pill
 * chrome (`"3 seconds"`); we wrap `intervalToDuration` and assemble
 * the parts ourselves.
 */
export function formatDuration(ms: number): string {
  if (!Number.isFinite(ms)) {
    return '0ms'
  }

  if (ms < 1000) {
    return `${Math.max(0, Math.round(ms))}ms`
  }

  const d = intervalToDuration({ start: 0, end: ms })
  const hours = d.hours ?? 0
  const minutes = d.minutes ?? 0
  const seconds = d.seconds ?? 0

  if (hours > 0) {
    return minutes > 0 ? `${hours}h ${minutes}m` : `${hours}h`
  }

  if (minutes > 0) {
    return seconds > 0 ? `${minutes}m ${seconds}s` : `${minutes}m`
  }

  return `${seconds}s`
}

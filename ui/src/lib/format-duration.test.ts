import { describe, expect, it } from 'vitest'

import { formatDuration } from './format-duration'

describe('formatDuration', () => {
  it('renders sub-second as Nms', () => {
    expect(formatDuration(0)).toBe('0ms')
    expect(formatDuration(50)).toBe('50ms')
    expect(formatDuration(999)).toBe('999ms')
  })

  it('renders sub-minute as Ns', () => {
    expect(formatDuration(1000)).toBe('1s')
    expect(formatDuration(3000)).toBe('3s')
    expect(formatDuration(59_000)).toBe('59s')
  })

  it('renders sub-hour as Nm Ks (drops seconds when zero)', () => {
    expect(formatDuration(60_000)).toBe('1m')
    expect(formatDuration(63_000)).toBe('1m 3s')
    expect(formatDuration(120_000)).toBe('2m')
  })

  it('renders hours as Nh Km (drops minutes when zero)', () => {
    expect(formatDuration(3_600_000)).toBe('1h')
    expect(formatDuration(3_720_000)).toBe('1h 2m')
  })

  it('clamps negative input to 0ms', () => {
    expect(formatDuration(-5)).toBe('0ms')
  })

  it('returns "0ms" for NaN / Infinity instead of throwing', () => {
    expect(formatDuration(Number.NaN)).toBe('0ms')
    expect(formatDuration(Number.POSITIVE_INFINITY)).toBe('0ms')
    expect(formatDuration(Number.NEGATIVE_INFINITY)).toBe('0ms')
  })
})

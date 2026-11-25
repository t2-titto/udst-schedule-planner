import { describe, expect, it } from 'vitest'
import { formatScore, formatTime, freeDaysMatter, fromUi, toUi } from './scale'

describe('scale', () => {
  it('converts both ways', () => { expect(fromUi(toUi(7))).toBe(7); expect(toUi(5)).toBe(0) })
  it('formats', () => { expect(formatScore(0)).toBe('0'); expect(formatScore(2)).toBe('+2'); expect(formatScore(-1.5)).toBe('-1.5'); expect(formatTime(14 * 60)).toBe('2:00 PM') })
  it('free days matter above the far left', () => { expect(freeDaysMatter(0)).toBe(false); expect(freeDaysMatter(0.5)).toBe(true) })
})

import { DEFAULT_SETTINGS } from '../engine/types'
import { effectiveSettings } from './scale'
describe('effectiveSettings', () => {
  it('drops the minimum free days when free days do not matter', () => {
    const s = { ...DEFAULT_SETTINGS, weights: { ...DEFAULT_SETTINGS.weights, freeDay: 0 }, rules: { ...DEFAULT_SETTINGS.rules, minFreeDays: 2 } }
    expect(effectiveSettings(s).rules.minFreeDays).toBe(0)
    expect(effectiveSettings({ ...s, weights: { ...s.weights, freeDay: 5 } }).rules.minFreeDays).toBe(2)
  })
})

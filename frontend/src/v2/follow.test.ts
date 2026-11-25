import { describe, expect, it } from 'vitest'
import { DEFAULT_SETTINGS, type ClassOption, type Component } from '../engine/types'
import { followBest } from './follow'
import { emptyState, type PCourse } from './model'

const comp = (nbr: string, day: number, start: number, end: number): Component => ({ dates: '', kind: 'Lecture', classNbr: nbr, section: nbr, room: 'R', instructors: ['A'], meetings: [{ day, start, end }], seatsOpen: 5, seatsTotal: 20 })
const opt = (id: string, c: Component): ClassOption => ({ id, optionNo: id, status: 'Open', components: [c] })
const h = (n: number) => n * 60
const course: PCourse = { code: 'X 1000', title: '', term: 'W', options: [opt('morning', comp('1', 1, h(9), h(11))), opt('afternoon', comp('2', 1, h(15), h(17)))] }
const base = { ...DEFAULT_SETTINGS, rules: { ...DEFAULT_SETTINGS.rules, minFreeDays: 0 } }

describe('the calendar following the preferences', () => {
  it('moves to the best schedule when the preferred hours change', () => {
    const am = followBest([course], emptyState(), { ...base, timing: { start: h(8), end: h(12), fade: h(1) } })
    expect(am.choice['X 1000']).toBe('morning')
    const pm = followBest([course], am, { ...base, timing: { start: h(14), end: h(18), fade: h(1) } })
    expect(pm.choice['X 1000']).toBe('afternoon')
  })
  it('returns the same state when nothing would change (so no history step is made)', () => {
    const am = followBest([course], emptyState(), { ...base, timing: { start: h(8), end: h(12), fade: h(1) } })
    expect(followBest([course], am, { ...base, timing: { start: h(8), end: h(12), fade: h(1) } })).toBe(am)
  })
  it('a pin wins over the score', () => {
    const pinned = { ...emptyState(), pins: ['2'] }
    expect(followBest([course], pinned, { ...base, timing: { start: h(8), end: h(12), fade: h(1) } }).choice['X 1000']).toBe('afternoon')
  })
})

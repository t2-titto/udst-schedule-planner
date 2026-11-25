import { describe, expect, it } from 'vitest'
import { DEFAULT_SETTINGS, instructorKey, type ClassOption, type Component } from '../engine/types'
import { findOutlier } from './outlier'
import { emptyState, type PCourse } from './model'

const comp = (nbr: string, kind: string, day: number, start: number, end: number): Component => ({ dates: '', kind, classNbr: nbr, section: nbr, room: 'R', instructors: ['A'], meetings: [{ day, start, end }], seatsOpen: 5, seatsTotal: 20 })
const opt = (id: string, ...c: Component[]): ClassOption => ({ id, optionNo: id, status: 'Open', components: c })
const h = (n: number) => n * 60

describe('the one small thing', () => {
  const settings = { ...DEFAULT_SETTINGS, timing: { start: h(8), end: h(12), fade: h(3) }, rules: { ...DEFAULT_SETTINGS.rules, minFreeDays: 0 } }
  it('names the class outside the preferred hours and offers to widen them when that raises the score', () => {
    const only: PCourse = { code: 'LATE 1000', title: '', term: 'W', options: [opt('x', comp('1', 'Lecture', 1, h(14), h(16)))] }
    const o = findOutlier([only], emptyState(), settings)!
    expect(o.what).toContain('LATE 1000')
    expect(o.gain).toBeGreaterThan(0.15)
    const next = o.fix.settings!(settings)
    expect(next.timing.end).toBe(h(16))
  })
  it('says nothing when the best schedule is already inside the hours', () => {
    const fine: PCourse = { code: 'OK 1000', title: '', term: 'W', options: [opt('x', comp('1', 'Lecture', 1, h(9), h(11)))] }
    expect(findOutlier([fine], emptyState(), settings)).toBeNull()
  })
  it('a pin that costs points is offered for removal', () => {
    const c: PCourse = { code: 'P 1000', title: '', term: 'W', options: [opt('early', comp('1', 'Lecture', 1, h(9), h(11))), opt('late', comp('2', 'Lecture', 1, h(15), h(17)))] }
    const o = findOutlier([c], { ...emptyState(), pins: ['2'] }, settings)!
    expect(o.fix.label).toMatch(/Unpin 2|Prefer/)
    const unpin = findOutlier([c], { ...emptyState(), pins: ['2'] }, { ...settings, timing: { start: h(0), end: h(23), fade: h(3) } })
    expect(unpin === null || unpin.fix.label.startsWith('Unpin')).toBe(true)
  })
  it('an online class that only drags the score down is offered as "do not score online classes"', () => {
    const online = comp('1', 'Lecture', 1, h(9), h(11))
    online.room = 'Online2'
    const c: PCourse = { code: 'ON 1000', title: '', term: 'W', options: [opt('x', online)] }
    // Only the instructor counts, and this instructor is rated at the bottom: leaving the online class out is the one thing that helps.
    const s2 = { ...settings, weights: { instructor: 10, timing: 0, freeDay: 0 }, instructorScores: { [instructorKey('ON 1000', 'A')]: 0 } }
    const o = findOutlier([c], emptyState(), s2)!
    expect(o.fix.label).toBe("Don't score online classes")
    expect(o.fix.settings!(s2).rules.ignoreOnline).toBe(true)
  })
})

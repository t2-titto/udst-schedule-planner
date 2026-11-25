import { describe, expect, it } from 'vitest'
import type { PCourse } from './model'
import { planChanged, planDiff } from './verifyPlan'

const held = { id: 'h', optionNo: 'Enrolled', status: 'Open', components: [] }
const courses: PCourse[] = [
  { code: 'AAA 1000', title: '', term: 'Winter 2027', options: [], held },
  { code: 'BBB 2000', title: '', term: 'Winter 2027', options: [] },
]
const row = (label: string, status: string, term = 'Winter 2027') => ({ course_label: label, term, status })

describe('re-reading PeopleSoft before Apply', () => {
  it('no difference when the lists agree', () => {
    const d = planDiff([row('AAA 1000: A', 'Enrolled'), row('BBB 2000: B', 'Classes available'), row('ZZZ 9000: Z', 'Classes available', 'Fall 2027')], 'Winter 2027', courses)
    expect(planChanged(d)).toBe(false)
  })
  it('a course planned in PeopleSoft meanwhile is a difference (the credit total would be wrong)', () => {
    const d = planDiff([row('AAA 1000: A', 'Enrolled'), row('BBB 2000: B', 'Classes available'), row('SOFT 2301: S', 'Classes available')], 'Winter 2027', courses)
    expect(d.plannedNew).toEqual(['SOFT 2301'])
    expect(planChanged(d)).toBe(true)
  })
  it('a course dropped or un-planned meanwhile is a difference', () => {
    const d = planDiff([row('BBB 2000: B', 'Classes available')], 'Winter 2027', courses)
    expect(d.enrolledGone).toEqual(['AAA 1000'])
    const e = planDiff([row('AAA 1000: A', 'Enrolled')], 'Winter 2027', courses)
    expect(e.plannedGone).toEqual(['BBB 2000'])
  })
})


describe('a waitlisted course is held (AEMA 2133 case, 2026-11-23)', () => {
  const wl = [{ code: 'WAIT 2133', title: '', term: 'Winter 2027', options: [], held: { id: 'w', optionNo: 'Waitlisted', status: 'Open', components: [], waitlisted: true } }] as unknown as PCourse[]
  it('is not reported as no longer enrolled when PeopleSoft still lists it as Waitlist (switch on)', () => {
    const d = planDiff([row('WAIT 2133: W', 'Waitlist')], 'Winter 2027', wl, true)
    expect(planChanged(d)).toBe(false)
  })
  it('with the switch off the page does not hold it, so PeopleSoft listing it as Waitlist is not a held course either', () => {
    const d = planDiff([row('WAIT 2133: W', 'Waitlist')], 'Winter 2027', wl, false)
    expect(d.enrolledNow).toEqual([])
  })
})

describe('a course added from the catalog', () => {
  const add = { code: 'NEW 1000', title: '', term: 'Winter 2027', options: [], units: 3, addPlan: { year: 1, index: 8, toCode: '1271', to: 'Winter 2027' } }
  it('is expected to be missing from PeopleSoft\'s list: no change is reported', () => {
    const d = planDiff([], 'Winter 2027', [add])
    expect(planChanged(d)).toBe(false)
  })
  it('but if PeopleSoft meanwhile lists it as planned, the page is out of date', () => {
    const d = planDiff([{ course_label: 'NEW 1000: x', term: 'Winter 2027', status: 'Classes available' }], 'Winter 2027', [add])
    expect(d.plannedNew).toEqual(['NEW 1000'])
    expect(planChanged(d)).toBe(true)
  })
})

describe('data that changed in PeopleSoft between the last read and Apply (6.3)', () => {
  const held = { id: 'h', optionNo: '1', status: 'Open', components: [] }
  const mine: PCourse[] = [
    { code: 'A 1', title: '', term: 'Winter 2027', options: [], held, units: 3 },
    { code: 'B 2', title: '', term: 'Winter 2027', options: [], units: 3 },
  ]
  const live = (rows: [string, string][]) => rows.map(([c, s]) => ({ course_label: `${c}: x`, term: 'Winter 2027', status: s }))
  it('is clean when PeopleSoft still shows what the page loaded', () => {
    expect(planChanged(planDiff(live([['A 1', 'Enrolled'], ['B 2', 'Classes available']]), 'Winter 2027', mine))).toBe(false)
  })
  it('stops Apply when a class was dropped, enrolled or planned elsewhere meanwhile', () => {
    expect(planDiff(live([['B 2', 'Classes available']]), 'Winter 2027', mine).enrolledGone).toEqual(['A 1'])
    expect(planDiff(live([['A 1', 'Enrolled'], ['B 2', 'Enrolled']]), 'Winter 2027', mine).enrolledNow).toEqual(['B 2'])
    expect(planDiff(live([['A 1', 'Enrolled'], ['B 2', 'Classes available'], ['C 3', 'Classes available']]), 'Winter 2027', mine).plannedNew).toEqual(['C 3'])
    expect(planChanged(planDiff(live([['A 1', 'Enrolled']]), 'Winter 2027', mine))).toBe(true)
  })
})

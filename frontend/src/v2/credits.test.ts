import { describe, expect, it } from 'vitest'
import { creditsFor, limitFor } from './credits'
import { emptyState, parseUnits, type PCourse } from './model'
import type { ClassOption } from '../engine/types'

const held = (code: string): ClassOption => ({ id: `enrolled:${code}`, optionNo: 'Enrolled', status: 'Open', components: [] })
const enrolled = (code: string, units: number | null): PCourse => ({ code, title: '', term: 'Winter 2027', options: [], held: held(code), units })
const planned = (code: string, units: number | null): PCourse => ({ code, title: '', term: 'Winter 2027', options: [], units })

// The owner's real Winter 2027, as PeopleSoft showed it: 4 + 3 + 3 + 3 for the four required courses, plus the 3-credit GARC pick.
const real = [enrolled('DACS 2202', 4), enrolled('DSAI 2201', 3), enrolled('INFS 2201', 3), enrolled('INFS 3102', 3), enrolled('GARC 2001', 3)]

describe('the limit', () => {
  it('is 18 for Fall and Winter (confirmed) and 12 for Spring (not confirmed)', () => {
    expect(limitFor('Winter 2027')).toMatchObject({ max: 18, confirmed: true })
    expect(limitFor('Fall 2027')).toMatchObject({ max: 18, confirmed: true })
    expect(limitFor('Spring 2027')).toMatchObject({ max: 12, confirmed: false })
  })
})

describe('the recorded PeopleSoft case: 16 enrolled, then SOFT 2301 (3) planned = 19, refused', () => {
  it('16 credits is fine', () => {
    const c = creditsFor(real, emptyState(), 'Winter 2027')
    expect([c.schedule, c.now, c.after]).toEqual([16, 16, 16])
    expect(c.over).toBe(false)
    expect(c.blocked).toBe(false)
  })
  it('19 credits is over the limit and blocks', () => {
    const c = creditsFor([...real, planned('SOFT 2301', 3)], emptyState(), 'Winter 2027')
    expect(c.after).toBe(19)
    expect(c.excess).toBe(1)
    expect(c.over).toBe(true)
    expect(c.blocked).toBe(true)
  })
})

describe('what PeopleSoft counts', () => {
  it('a planned course the student unticked in the schedule still counts', () => {
    const cs = [...real, planned('SOFT 2301', 3)]
    const c = creditsFor(cs, { ...emptyState(), removed: ['SOFT 2301'] }, 'Winter 2027')
    expect(c.schedule).toBe(16) // not in the schedule
    expect(c.after).toBe(19) // but PeopleSoft still counts it
    expect(c.blocked).toBe(true)
  })
  it('an enrolled course taken out of the schedule is dropped by Apply, so it stops counting', () => {
    const cs = [...real, planned('SOFT 2301', 3)]
    const c = creditsFor(cs, { ...emptyState(), removed: ['GARC 2001'] }, 'Winter 2027')
    expect(c.after).toBe(16)
    expect(c.now).toBe(19)
    expect(c.blocked).toBe(false)
  })
  it('exactly 18 is allowed', () => {
    expect(creditsFor([...real, planned('X 1000', 2)], emptyState(), 'Winter 2027').blocked).toBe(false)
  })
})

describe('unknown units', () => {
  it('are never counted as 0 and block', () => {
    const c = creditsFor([...real, planned('NEW 1000', null)], emptyState(), 'Winter 2027')
    expect(c.after).toBe(16)
    expect(c.unknown).toEqual(['NEW 1000'])
    expect(c.blocked).toBe(true)
    expect(c.over).toBe(false)
  })
  it('an unknown course that does not count (enrolled and dropped) does not block', () => {
    const c = creditsFor([...real, enrolled('OLD 1000', null)], { ...emptyState(), removed: ['OLD 1000'] }, 'Winter 2027')
    expect(c.unknown).toEqual([])
    expect(c.blocked).toBe(false)
  })
  it('parseUnits never turns text into 0', () => {
    expect([parseUnits('3.00'), parseUnits(' 4 '), parseUnits(''), parseUnits(undefined), parseUnits('abc'), parseUnits('-1')]).toEqual([3, 4, null, null, null, null])
  })
})

describe('the way out', () => {
  const c = creditsFor([...real, planned('SOFT 2301', 3), planned('MATH 1030', 4)], emptyState(), 'Winter 2027')
  it('offers planned courses first, then enrolled ones as a drop', () => {
    expect(c.wayOut.map((w) => [w.code, w.action])).toEqual([['MATH 1030', 'unplan'], ['SOFT 2301', 'unplan'], ['DACS 2202', 'drop'], ['DSAI 2201', 'drop'], ['GARC 2001', 'drop'], ['INFS 2201', 'drop'], ['INFS 3102', 'drop']])
  })
  it('says which single removal is enough', () => {
    expect(c.after).toBe(23)
    expect(c.wayOut.filter((w) => w.enough)).toEqual([]) // 5 over: no single course is enough
    const d = creditsFor([...real, planned('SOFT 2301', 3)], emptyState(), 'Winter 2027')
    expect(d.wayOut.find((w) => w.code === 'SOFT 2301')?.enough).toBe(true)
  })
  it('is empty when not over', () => {
    expect(creditsFor(real, emptyState(), 'Winter 2027').wayOut).toEqual([])
  })
})

describe('Spring', () => {
  it('uses 12 and is over at 13', () => {
    const c = creditsFor([planned('A 1000', 7), planned('B 1000', 6)], emptyState(), 'Spring 2027')
    expect(c.limit.max).toBe(12)
    expect(c.over).toBe(true)
  })
})


describe('the planning-semester rule on the schedule', () => {
  const o = { id: 'x', optionNo: '1', status: 'Open', components: [] }
  const held = (code: string, units: number): PCourse => ({ code, title: '', term: 'Winter 2027', options: [], held: o, units })
  const planned = (code: string, units: number, extra: Partial<PCourse> = {}): PCourse => ({ code, title: '', term: 'Winter 2027', options: [], units, ...extra })
  const enrolled = [held('A 1', 4), held('B 2', 3), held('C 3', 3), held('D 4', 3), held('E 5', 3)] // 16
  it('over the limit only because of a planned course that belongs to another term: that course is not counted, and Apply is not blocked', () => {
    const cs = [...enrolled, planned('INFT 3101', 3)]
    expect(creditsFor(cs, emptyState(), 'Winter 2027').after).toBe(19)
    const c = creditsFor(cs, emptyState(), 'Winter 2027', { 'INFT 3101': 'Fall 2027' })
    expect(c.after).toBe(16)
    expect(c.over).toBe(false)
    expect(c.blocked).toBe(false)
    expect(c.deferred).toEqual([{ code: 'INFT 3101', to: 'Fall 2027', units: 3 }])
    expect(c.now).toBe(19) // what PeopleSoft lists today is still shown
  })
  it('enrolled courses always count, even when over', () => {
    const cs = [...enrolled, held('F 6', 3)]
    const c = creditsFor(cs, emptyState(), 'Winter 2027', { 'F 6': 'Fall 2027' })
    expect(c.after).toBe(19)
    expect(c.over).toBe(true)
    expect(c.deferred).toEqual([])
  })
  it('a planned course that belongs to this term, or has no known home, still counts; a course added from the catalog still counts', () => {
    const cs = [...enrolled, planned('P 7', 3)]
    expect(creditsFor(cs, emptyState(), 'Winter 2027', { 'P 7': 'Winter 2027' }).after).toBe(19)
    expect(creditsFor(cs, emptyState(), 'Winter 2027', {}).after).toBe(19)
    const added = [...enrolled, planned('NEW 8', 3, { addPlan: { year: 1, index: 0, toCode: '1271', to: 'Winter 2027' } })]
    expect(creditsFor(added, emptyState(), 'Winter 2027', { 'NEW 8': 'Fall 2027' }).after).toBe(19)
  })
  it('when it is still over after setting those aside, it stays blocked', () => {
    const cs = [...enrolled, planned('P 7', 3), planned('INFT 3101', 3)]
    const c = creditsFor(cs, emptyState(), 'Winter 2027', { 'INFT 3101': 'Fall 2027', 'P 7': 'Winter 2027' })
    expect(c.after).toBe(19)
    expect(c.blocked).toBe(true)
  })
})

describe('waitlisted classes (unverified)', () => {
  const waited = (code: string, units: number): PCourse => ({ ...enrolled(code, units), held: { ...held(code), waitlisted: true } })
  it('count in the total, and are named so the page can warn that PeopleSoft may not count them', () => {
    const c = creditsFor([...real, waited('SOFT 2301', 3)], emptyState(), 'Winter 2027')
    expect(c.after).toBe(19)
    expect(c.waitlistUnits).toBe(3)
    expect(c.lines.find((l) => l.code === 'SOFT 2301')?.waitlisted).toBe(true)
    expect(c.over && c.blocked && c.waitlistOnly).toBe(true)
  })
  it('over only because of the waitlist: blocked until the student overrides, then 16 is what is sent', () => {
    const cs = [...real, waited('SOFT 2301', 3)]
    const c = creditsFor(cs, emptyState(), 'Winter 2027', {}, true)
    expect([c.blocked, c.overridden, c.sent, c.after]).toEqual([false, true, 16, 19])
  })
  it('over with the waitlist left out too: the override does not unblock', () => {
    const c = creditsFor([...real, planned('X 1', 3), waited('SOFT 2301', 3)], emptyState(), 'Winter 2027', {}, true)
    expect([c.waitlistOnly, c.overridden, c.blocked, c.sent]).toEqual([false, false, true, 22])
  })
  it('an ordinary schedule has no waitlist units and sends its total', () => {
    const c = creditsFor(real, emptyState(), 'Winter 2027', {}, true)
    expect([c.waitlistUnits, c.waitlistOnly, c.overridden, c.sent]).toEqual([0, false, false, 16])
  })
})

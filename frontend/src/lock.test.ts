import { describe, expect, it } from 'vitest'
import { solve } from './engine/solver'
import { DEFAULT_SETTINGS, type ClassOption, type Component, type Course } from './engine/types'
import { applyLocks, codeOf, currentOption, enrolledOptions, isWaitlistedStatus, parseSchedule, waitlistedFromSummary, type EnrolledRow } from './lock'

const comp = (nbr: string, day: number, start: number, end: number, kind = 'Lecture'): Component => ({
  dates: '', kind, classNbr: nbr, section: '1', room: 'R1', instructors: ['A'], meetings: [{ day, start, end }], seatsOpen: 5, seatsTotal: 20,
})
const opt = (id: string, ...c: Component[]): ClassOption => ({ id, optionNo: id, status: 'Open', components: c })
const course = (code: string, ...o: ClassOption[]): Course => ({ code, title: code, term: 'Winter 2027', options: o })
const row = (nbr: string, courseText: string, schedule: string[], status = 'Enrolled'): EnrolledRow => ({ row: 0, class_nbr: nbr, component: 'Lecture', section: '1', course: courseText, schedule, status })

describe('parsing what PeopleSoft shows', () => {
  it('reads schedule lines and course codes', () => {
    expect(parseSchedule(['Monday 3:30PM to 5:00PM', 'Wednesday 3:30PM to 5:00PM'])).toEqual([
      { day: 1, start: 15 * 60 + 30, end: 17 * 60 }, { day: 3, start: 15 * 60 + 30, end: 17 * 60 },
    ])
    expect(parseSchedule(['Monday, Wednesday 9:30AM to 11:00AM']).map((m) => m.day)).toEqual([1, 3])
    expect(parseSchedule(['garbage'])).toEqual([])
    expect(codeOf('DACS 2202 Intro. to Computing Systems')).toBe('DACS 2202')
    expect(codeOf('DACS2202 Intro')).toBe('DACS 2202')
  })
  it('builds the enrolled option from the real list, including a class no fetched option has', () => {
    const o = enrolledOptions([row('3496', 'DSAI 2201 Intro', ['Sunday 8:00PM to 10:00PM']), row('3497', 'DSAI 2201 Intro', ['Tuesday 2:00PM to 3:30PM']), row('9', 'ZZZ 1000 Dropped', ['Monday 8:00AM to 9:00AM'], 'Dropped')])
    expect(Object.keys(o)).toEqual(['DSAI 2201'])
    expect(o['DSAI 2201'].components.map((c) => c.classNbr)).toEqual(['3496', '3497'])
  })
  it('finds the fetched option that matches what is held', () => {
    const c = course('AA 1000', opt('o1', comp('1', 1, 480, 570)), opt('o2', comp('2', 2, 480, 570)))
    const held = enrolledOptions([row('2', 'AA 1000 x', ['Tuesday 8:00AM to 9:30AM'])])['AA 1000']
    expect(currentOption(c, held)?.id).toBe('o2')
    expect(currentOption(c, undefined)).toBeUndefined()
  })
})

describe('locking', () => {
  const a = course('AA 1000', opt('a1', comp('1', 1, 480, 570)), opt('a2', comp('2', 2, 480, 570)))
  const b = course('BB 2000', opt('b1', comp('3', 1, 500, 590)), opt('b2', comp('4', 3, 480, 570)))
  const held = enrolledOptions([row('1', 'AA 1000 x', ['Monday 8:00AM to 9:30AM'])])

  it('an enrolled course is fixed to what the student holds, even when its seats are used up', () => {
    const r = applyLocks([a, b], held, [], {})
    expect(r.locks).toEqual({ 'AA 1000': 'enrolled' })
    expect(r.courses[0].options).toHaveLength(1)
    expect(r.courses[1].options).toHaveLength(2)
    // The held option has no seats left and would be excluded by "exclude full"; the lock must not let that happen.
    const full = { 'AA 1000': { ...held['AA 1000'], components: held['AA 1000'].components.map((c) => ({ ...c, seatsOpen: 0 })) } }
    const s = solve(applyLocks([a, b], full, [], {}).courses, DEFAULT_SETTINGS)
    expect(s.schedules.length).toBeGreaterThan(0)
    expect(s.schedules.every((x) => x.picks.find((p) => p.course.code === 'AA 1000')?.option.id === 'enrolled:AA 1000')).toBe(true)
  })

  it('unlocking frees every option of that course again', () => {
    const r = applyLocks([a, b], held, ['AA 1000'], {})
    expect(r.locks).toEqual({})
    expect(r.courses[0].options).toHaveLength(2)
  })

  it('a pin fixes a whole combination and wins over the enrolled lock of the same course', () => {
    const r = applyLocks([a, b], held, [], { 'AA 1000': 'a2' })
    expect(r.locks).toEqual({ 'AA 1000': 'pin' })
    expect(r.courses[0].options.map((o) => o.id)).toEqual(['a2'])
  })

  it('a pin that overlaps another enrolled course unlocks that course and says so', () => {
    // B's option b1 (Mon 8:20 to 9:50) overlaps the enrolled A (Mon 8:00 to 9:30).
    const r = applyLocks([a, b], held, [], { 'BB 2000': 'b1' })
    expect(r.locks).toEqual({ 'BB 2000': 'pin' })
    expect(r.autoUnlocked).toEqual(['AA 1000'])
    expect(r.notices[0]).toMatch(/BB 2000 is pinned and overlaps your enrolled AA 1000/)
    expect(r.courses[0].options).toHaveLength(2)
  })

  it('a pin that points at an option that no longer exists is ignored', () => {
    const r = applyLocks([a, b], {}, [], { 'AA 1000': 'gone' })
    expect(r.locks).toEqual({})
  })
})

describe('waitlisted classes (unverified)', () => {
  const w = (nbr: string, status: string) => row(nbr, 'AA 1000 Title', ['Monday 9:00AM to 10:00AM'], status)
  it('are not held unless waitlist support is on', () => {
    expect(enrolledOptions([w('1', 'Waitlisted')])).toEqual({})
  })
  it('are held and marked waitlisted when it is on, with the class numbers of the Drop page', () => {
    const o = enrolledOptions([w('1', 'Waitlisted')], true)['AA 1000']
    expect(o.waitlisted).toBe(true)
    expect(o.optionNo).toBe('Waitlisted')
    expect(o.components[0].classNbr).toBe('1')
    expect(enrolledOptions([w('1', 'Enrolled')], true)['AA 1000'].waitlisted).toBeUndefined()
    expect(isWaitlistedStatus('Wait List') && isWaitlistedStatus('Waitlisted') && !isWaitlistedStatus('Enrolled')).toBe(true)
  })
  it('one the Drop page does not list is built from the plan list text, with no class number', () => {
    const o = waitlistedFromSummary('AA 1000', 'Monday 10:00AM to 11:00AM Wednesday 10:00AM to 11:00AM')!
    expect(o.waitlisted).toBe(true)
    expect(o.components[0].meetings).toHaveLength(2)
    expect(o.components[0].classNbr).toBe('')
    expect(waitlistedFromSummary('AA 1000', '')).toBeNull()
  })
})

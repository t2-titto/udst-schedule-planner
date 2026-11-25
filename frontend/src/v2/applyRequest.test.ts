import { describe, expect, it } from 'vitest'
import type { ClassOption, Component, Course } from '../engine/types'
import { buildPlan } from '../plan'
import { planSwapRounds, type RoundSwap } from '../swapRounds'
import type { EnrolledState } from '../enrolled'
import { buildApplyRequest, NO_ROW, wireProblems, wireRow } from './applyRequest'
import { creditsFor } from './credits'
import { emptyState, buildCourses, type AddPlan, type PCourse } from './model'

const comp = (nbr: string, kind: string, day: number, start: number, end: number, seats: number | null = 5): Component => ({ dates: '', kind, classNbr: nbr, section: nbr, room: 'R', instructors: ['A'], meetings: [{ day, start, end }], seatsOpen: seats, seatsTotal: 20 })
const opt = (id: string, row: number | undefined, ...c: Component[]): ClassOption => ({ id, optionNo: id, status: 'Open', optionRow: row, components: c })
const course = (code: string, options: ClassOption[], row: number | undefined = 3): Course => ({ code, title: '', term: 'W', key: { strm: '1', crse_id: '2', crse_offer_nbr: '1', acad_career: 'C', institution: 'I' }, row, options }) as Course
const sched = (picks: { course: Course; option: ClassOption }[]) => ({ picks: picks.map((p, i) => ({ ...p, courseIndex: i, instructorScore: 0, limiting: [] })), blocks: [], freeDays: [], scores: { instructor: 0, timing: 0, freeDay: 0, total: 0 } })
const nothingEnrolled: EnrolledState = { term: 'Winter 2027', termRow: -1, rows: [], options: {}, unconfirmed: [] }

function request(picks: { course: Course; option: ClassOption }[], held: Record<string, ClassOption>, enrolled: EnrolledState, added: Record<string, AddPlan> = {}) {
  const rows = Object.keys(held).map((code, i) => ({ row: i, class_nbr: held[code].components[0].classNbr, component: 'Lecture', section: '1', course: `${code} x`, schedule: [], status: 'Enrolled' }))
  const plan = buildPlan(sched(picks), held, rows, { selected: [], hidden: [], courses: [], live: new Set(picks.map((p) => p.course.code)) })
  const rp = planSwapRounds(plan.swaps.map((s): RoundSwap => ({ code: s.course.code, now: s.now, next: s.next, seatsLeft: 5 })), plan.keeps.map((k) => k.now), plan.swaps.filter((s) => s.redo).map((s) => s.course.code))
  return { plan, req: buildApplyRequest(plan, rp, enrolled, plan.drops, creditsFor([], emptyState(), enrolled.term), added) }
}

describe('the apply request is something the server can read', () => {
  it('nothing enrolled in the term (term row -1): the term row is null (not known), never 0 or -1', () => {
    const o = opt('n', 4, comp('9', 'Lecture', 1, 540, 600))
    const { req } = request([{ course: course('NEW 1000', [o]), option: o }], {}, nothingEnrolled)
    expect(req.termRow).toBeNull()
    expect(req.enrols[0].pick).toMatchObject({ row: 3, optionRow: 4 })
    expect(wireProblems(req)).toEqual([])
    // What the server reads: JSON, and every row an unsigned integer.
    const wire = JSON.parse(JSON.stringify(req))
    expect(wire.termRow).toBeNull()
  })
  it('a course without a row or option row is sent as "not known" (the server finds it), never left out', () => {
    const o = opt('n', undefined, comp('9', 'Lecture', 1, 540, 600))
    const { req } = request([{ course: { ...course('NEW 1000', [o]), row: undefined }, option: o }], {}, nothingEnrolled)
    expect(req.enrols[0].pick.row).toBe(NO_ROW)
    expect(req.enrols[0].pick.optionRow).toBe(NO_ROW)
    expect(wireProblems(req)).toEqual([])
  })
  it('swaps, drops and a drop-and-enrol-again all build without problems', () => {
    const heldLec = comp('1', 'Lecture', 1, 540, 600)
    const heldLab = comp('2', 'Laboratory', 2, 540, 600)
    const held = opt('h', 0, heldLec, heldLab)
    const labOnly = opt('l', 1, heldLec, comp('3', 'Laboratory', 3, 540, 600))
    const other = opt('o', 2, comp('4', 'Lecture', 2, 700, 760))
    const gone = opt('g', 0, comp('5', 'Lecture', 3, 900, 960))
    const enrolled: EnrolledState = { term: 'Winter 2027', termRow: 1, rows: [], options: {}, unconfirmed: [] }
    const { req, plan } = request(
      [{ course: course('AA 1000', [held, labOnly]), option: labOnly }, { course: course('BB 1000', [other]), option: other }],
      { 'AA 1000': held, 'GG 1000': gone },
      enrolled,
    )
    expect(plan.swaps.find((s) => s.redo)?.course.code).toBe('AA 1000')
    expect(req.drops.map((d) => d.course).sort()).toEqual(['AA 1000', 'GG 1000'])
    expect(req.enrols.find((e) => e.afterDrop)?.pick.row).toBe(NO_ROW)
    expect(wireProblems(req)).toEqual([])
    expect(req.termRow).toBe(1)
  })
})

describe('what the checker catches', () => {
  it('undefined, negative and fractional numbers', () => {
    expect(wireProblems({ termRow: -1 })).toHaveLength(1)
    expect(wireProblems({ pick: { row: undefined } })).not.toEqual([])
    expect(wireProblems({ row: 1.5 })).toHaveLength(1)
    expect(wireProblems({ row: 0, optionRow: NO_ROW, termRow: 3 })).toEqual([])
  })
  it('wireRow', () => {
    expect([wireRow(0), wireRow(7), wireRow(-1), wireRow(undefined), wireRow(2.5), wireRow(NaN)]).toEqual([0, 7, NO_ROW, NO_ROW, NO_ROW, NO_ROW])
  })
})

describe('a course added from the catalog is planned first, then enrolled', () => {
  const add: AddPlan = { year: 1, index: 8, toCode: '1271', to: 'Winter 2027' }
  const o = opt('n', undefined, comp('9', 'Lecture', 1, 540, 600), comp('10', 'Laboratory', 2, 540, 600))
  it('sends a plan step and an enrolment that waits for it, with no row (the server finds it after planning)', () => {
    const { req } = request([{ course: { ...course('SOFT 2301', [o]), row: undefined }, option: o }], {}, nothingEnrolled, { 'SOFT 2301': add })
    expect(req.plans).toHaveLength(1)
    expect(req.plans[0]).toMatchObject({ course: 'SOFT 2301', year: 1, index: 8, termCode: '1271' })
    expect(req.plans[0].credits.term).toBe('Winter 2027')
    expect(req.enrols[0]).toMatchObject({ afterPlan: true, pick: { row: NO_ROW, optionRow: NO_ROW, classNbrs: ['9', '10'] } })
    expect(wireProblems(req)).toEqual([])
  })
  it('a course PeopleSoft already lists is an ordinary enrolment with no plan step', () => {
    const { req } = request([{ course: course('NEW 1000', [o]), option: o }], {}, nothingEnrolled, {})
    expect(req.plans).toEqual([])
    expect(req.enrols[0].afterPlan).toBeUndefined()
  })
  it('the added course counts towards the credits while it is in the schedule, and not after it is removed', () => {
    const a: PCourse = { code: 'SOFT 2301', title: '', term: 'Winter 2027', options: [o], units: 3, addPlan: add }
    const held: PCourse = { code: 'HELD 1000', title: '', term: 'Winter 2027', options: [], units: 16, held: opt('h', 0, comp('1', 'Lecture', 1, 700, 760)) }
    const withIt = creditsFor([held, a], emptyState(), 'Winter 2027')
    expect(withIt.after).toBe(19)
    expect(withIt.over).toBe(true)
    const removed = creditsFor([held, a], { ...emptyState(), removed: ['SOFT 2301'] }, 'Winter 2027')
    expect(removed.after).toBe(16)
  })
  it('buildCourses adds it once, and not when PeopleSoft already lists it', () => {
    const a: PCourse = { code: 'SOFT 2301', title: '', term: 'Winter 2027', options: [o], units: 3, addPlan: add }
    expect(buildCourses([], {}, [], {}, 'Winter 2027', [a]).map((c) => c.code)).toEqual(['SOFT 2301'])
    const listed = { ...course('SOFT 2301', [o]), units: '3.00' }
    const out = buildCourses([listed], {}, [], {}, 'Winter 2027', [a])
    expect(out.map((c) => c.code)).toEqual(['SOFT 2301'])
    expect(out[0].addPlan).toBeUndefined()
  })
})

describe('joining a waitlist (unverified)', () => {
  it('the pick carries waitlist: true only for an option PeopleSoft marks Wait List', () => {
    const wl = { ...opt('w', 4, comp('9', 'Lecture', 1, 540, 600, null)), waitlist: true }
    const open = opt('o', 5, comp('8', 'Lecture', 2, 540, 600))
    const a = request([{ course: course('NEW 1000', [wl]), option: wl }], {}, nothingEnrolled)
    expect(a.req.enrols[0].pick.waitlist).toBe(true)
    expect(wireProblems(a.req)).toEqual([])
    const b = request([{ course: course('NEW 1001', [open]), option: open }], {}, nothingEnrolled)
    expect(b.req.enrols[0].pick.waitlist).toBeUndefined()
  })
})


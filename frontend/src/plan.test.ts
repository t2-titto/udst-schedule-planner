import { describe, expect, it } from 'vitest'
import { solve } from './engine/solver'
import { DEFAULT_SETTINGS, type ClassOption, type Component, type Course } from './engine/types'
import { buildPlan, changesOf, labOnly, sameClasses } from './plan'
import type { EnrolledRow } from './lock'

const comp = (nbr: string, day: number, start: number, end: number, kind = 'Lecture'): Component => ({
  dates: '', kind, classNbr: nbr, section: '1', room: 'R', instructors: ['A'], meetings: [{ day, start, end }], seatsOpen: 5, seatsTotal: 20,
})
const opt = (id: string, row: number, ...c: Component[]): ClassOption => ({ id, optionRow: row, optionNo: id, status: 'Open', components: c })
const course = (code: string, ...o: ClassOption[]): Course => ({ code, title: code, term: 'Winter 2027', row: 1, key: { strm: '1', crse_id: '1', crse_offer_nbr: '1', acad_career: 'C', institution: 'I' }, options: o })

const a1 = opt('a1', 0, comp('11', 1, 480, 570), comp('12', 1, 600, 660, 'Laboratory'))
const a2 = opt('a2', 1, comp('21', 2, 480, 570), comp('22', 2, 600, 660, 'Laboratory'))
const a3 = opt('a3', 2, comp('11', 1, 480, 570), comp('13', 3, 600, 660, 'Laboratory')) // same lecture, other lab
const b1 = opt('b1', 0, comp('31', 3, 480, 570))
const b2 = opt('b2', 1, comp('41', 4, 480, 570))
const A = course('AA 1000', a1, a2, a3)
const B = course('BB 2000', b1, b2)
const heldA: ClassOption = { id: 'enrolled:AA 1000', optionNo: 'Enrolled', status: 'Open', components: a1.components }
const schedule = (...c: Course[]) => solve(c, DEFAULT_SETTINGS).schedules[0]
const ctx = (over = {}) => ({ selected: ['AA 1000', 'BB 2000'], hidden: [], courses: [A, B], live: new Set(['AA 1000', 'BB 2000']), ...over })

describe('what Enrol has to do', () => {
  it('keeps what already matches, enrols what is not held', () => {
    const s = schedule({ ...A, options: [a1] }, B)
    const p = buildPlan(s, { 'AA 1000': heldA }, [], ctx())
    expect(p.keeps.map((k) => k.code)).toEqual(['AA 1000'])
    expect(p.enrols.map((e) => e.course.code)).toEqual(['BB 2000'])
    expect(p.swaps).toHaveLength(0)
    expect(p.blockers).toEqual([])
    expect(changesOf(s, { 'AA 1000': heldA })).toBe(1)
  })

  it('same class numbers are the same class even if the option id differs', () => {
    expect(sameClasses(heldA, a1)).toBe(true)
    expect(sameClasses(heldA, a2)).toBe(false)
  })

  it('a different combination of a held course is a swap', () => {
    const s = schedule({ ...A, options: [a2] }, B)
    const p = buildPlan(s, { 'AA 1000': heldA }, [], ctx())
    expect(p.swaps.map((x) => x.course.code)).toEqual(['AA 1000'])
    expect(p.swaps[0].blocker).toBeUndefined()
  })

  it('a swap that changes only the lab is blocked, and says why', () => {
    expect(labOnly(heldA, a3)).toBe(true)
    expect(labOnly(heldA, a2)).toBe(false)
    const s = schedule({ ...A, options: [a3] }, B)
    const p = buildPlan(s, { 'AA 1000': heldA }, [], ctx())
    expect(p.blockers.some((b) => /only the lab/.test(b))).toBe(false)
    expect(p.swaps[0].redo).toBe(true)
  })

  it('an unticked held course is offered for a drop, a hidden one never', () => {
    const held = { 'AA 1000': heldA, 'BB 2000': { ...heldA, id: 'enrolled:BB 2000', components: b1.components } }
    const rows: EnrolledRow[] = [{ row: 0, class_nbr: '31', component: 'Lecture', section: '1', course: 'BB 2000 Title', schedule: [], status: 'Enrolled' }]
    const s = schedule({ ...A, options: [a1] })
    expect(buildPlan(s, held, rows, ctx({ selected: ['AA 1000'] })).drops.map((d) => d.code)).toEqual(['BB 2000'])
    expect(buildPlan(s, held, rows, ctx({ selected: ['AA 1000'], hidden: ['BB 2000'] })).drops).toEqual([])
  })

  it('a hidden held course still blocks its time', () => {
    const clashing: ClassOption = { id: 'enrolled:CC 3000', optionNo: 'Enrolled', status: 'Open', components: [comp('51', 1, 500, 560)] }
    const s = schedule({ ...A, options: [a1] })
    const p = buildPlan(s, { 'AA 1000': heldA, 'CC 3000': clashing }, [], ctx({ selected: ['AA 1000'], hidden: ['CC 3000'] }))
    expect(p.blockers.some((b) => /Clash/.test(b))).toBe(true)
  })

  it('a course not fetched live cannot be enrolled or swapped', () => {
    const s = schedule({ ...A, options: [a1] }, B)
    const p = buildPlan(s, {}, [], ctx({ live: new Set() }))
    expect(p.blockers.length).toBe(2)
  })

  it('an enrolment that overlaps a combination a swap releases waits for that swap', () => {
    const bx = opt('bx', 0, comp('61', 1, 480, 570)) // overlaps a1 (Mon 8:00 to 9:30)
    const B2 = course('BB 2000', bx)
    const s = schedule({ ...A, options: [a2] }, B2)
    const p = buildPlan(s, { 'AA 1000': heldA }, [], ctx({ courses: [A, B2] }))
    expect(p.enrols[0].depends).toEqual(['AA 1000'])
    expect(p.blockers).toEqual([])
  })
})

import { primaryClass } from './lock'
describe('dropping a course', () => {
  it('uses the lecture, never a lab on its own', () => {
    const r = (nbr: string, component: string) => ({ row: 0, class_nbr: nbr, component, section: '1', course: 'X 1000', schedule: [], status: 'Enrolled' })
    expect(primaryClass([r('9', 'Laboratory'), r('1', 'Lecture')])?.class_nbr).toBe('1')
    expect(primaryClass([r('5', 'LecTheatre'), r('6', 'Laboratory')])?.class_nbr).toBe('5')
  })
})

describe('an enrolment waits for the drop whose time it takes', () => {
  it('depends on the dropped course when the new classes overlap the dropped ones', () => {
    const mk = (id: string, nbr: string, day: number, start: number, end: number): ClassOption => ({ id, optionNo: id, status: 'Open', components: [{ dates: '', kind: 'Lecture', classNbr: nbr, section: '1', room: '', instructors: [], meetings: [{ day, start, end }], seatsOpen: 5, seatsTotal: 20 }] })
    const heldGone = mk('h', '1', 1, 600, 700)
    const newOpt = { ...mk('n', '2', 1, 650, 750), optionRow: 3 }
    const course = { code: 'NEW 1000', title: '', term: 'W', key: { a: 1 }, row: 0, options: [newOpt] } as unknown as Course
    const schedule = { picks: [{ course, courseIndex: 0, option: newOpt, instructorScore: 0, limiting: [] }], blocks: [], freeDays: [], scores: { instructor: 0, timing: 0, freeDay: 0, total: 0 } }
    const rows = [{ row: 0, class_nbr: '1', component: 'Lecture', section: '1', course: 'OLD 1000 Old', schedule: [], status: 'Enrolled' }]
    const p = buildPlan(schedule, { 'OLD 1000': heldGone }, rows, { selected: [], hidden: [], courses: [], live: new Set(['NEW 1000']) })
    expect(p.drops.map((d) => d.code)).toEqual(['OLD 1000'])
    expect(p.enrols[0].depends).toEqual(['OLD 1000'])
  })
})

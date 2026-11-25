/** Tests for the fixes that came out of the red-team report (docs/RED-TEAM-REPORT.md). Each names the finding it closes. */
import { renderToStaticMarkup } from 'react-dom/server'
import { afterEach, describe, expect, it, vi } from 'vitest'
import wire from './fixtures/degree-wire.json'
import type { ClassOption, Component, Course } from '../engine/types'
import { buildPlan } from '../plan'
import { ApiError, api } from '../api'
import { VERIFY_MAX_AGE_MS, keptDropClashes, verifyStale } from './applyGuard'
import { openTermOf } from './attention'
import { openTerm, termOrder } from './data'
import { buildDegree, type WireDegree } from './degree'
import { coursesOf, EMPTY_DRAFT, termLoads } from './draft'
import { Login } from './Login'
import { ACADEMIC_KEYS, clearAcademicData } from './storage'

const comp = (nbr: string, day: number, start: number, end: number): Component => ({ dates: '', kind: 'Lecture', classNbr: nbr, section: nbr, room: 'R', instructors: ['A'], meetings: [{ day, start, end }], seatsOpen: 5, seatsTotal: 20 })
const opt = (id: string, ...c: Component[]): ClassOption => ({ id, optionNo: id, status: 'Open', optionRow: undefined, components: c })
const course = (code: string, options: ClassOption[]): Course => ({ code, title: '', term: 'W', key: { strm: '1', crse_id: '2', crse_offer_nbr: '1', acad_career: 'C', institution: 'I' }, row: 3, options }) as Course

describe('H1: an unticked drop keeps a course that a new class would overlap', () => {
  const g = opt('g', comp('100', 1, 540, 600)) // held course G, Monday 9:00 to 10:00
  const n = opt('n', comp('200', 1, 570, 630)) // new course N, Monday 9:30 to 10:30: takes G's slot
  const sched = { picks: [{ course: course('NEW 1000', [n]), option: n, courseIndex: 0, instructorScore: 0, limiting: [] }], blocks: [], freeDays: [], scores: { instructor: 0, timing: 0, freeDay: 0, total: 0 } }
  const rows = [{ row: 0, class_nbr: '100', component: 'Lecture', section: '1', course: 'GONE 1000 x', schedule: [], status: 'Enrolled' }]
  const plan = buildPlan(sched, { 'GONE 1000': g }, rows, { selected: [], hidden: [], courses: [], live: new Set(['NEW 1000']) })
  it('the plan drops G and enrols N after it', () => {
    expect(plan.drops.map((d) => d.code)).toEqual(['GONE 1000'])
    expect(plan.enrols[0].depends).toEqual(['GONE 1000'])
  })
  it('with the drop ticked (or never touched) nothing is wrong; unticked, Apply gets a named reason', () => {
    expect(keptDropClashes(plan, { 'GONE 1000': g }, {})).toEqual([])
    expect(keptDropClashes(plan, { 'GONE 1000': g }, { 'GONE 1000': true })).toEqual([])
    const bad = keptDropClashes(plan, { 'GONE 1000': g }, { 'GONE 1000': false })
    expect(bad).toHaveLength(1)
    expect(bad[0]).toContain('GONE 1000')
    expect(bad[0]).toContain('NEW 1000')
  })
})

describe('M8: the re-check of PeopleSoft expires', () => {
  it('is stale when never made, and after two minutes', () => {
    expect(verifyStale(null, 1000)).toBe(true)
    expect(verifyStale(1000, 1000 + VERIFY_MAX_AGE_MS)).toBe(false)
    expect(verifyStale(1000, 1000 + VERIFY_MAX_AGE_MS + 1)).toBe(true)
  })
})

describe('L2: sign-out removes the saved academic data and keeps the settings', () => {
  it('removes every academic key and nothing else', () => {
    const removed: string[] = []
    clearAcademicData({ removeItem: (k: string) => { removed.push(k) } })
    expect(removed.sort()).toEqual([...ACADEMIC_KEYS].sort())
    expect(removed).not.toContain('schedule-planner:v2')
    expect(removed).toContain('schedule-planner:v2:degree')
    expect(removed).toContain('schedule-planner:v2:home')
  })
  it('a blocked storage does not stop the others', () => {
    let n = 0
    clearAcademicData({ removeItem: () => { n++; if (n === 1) throw new Error('blocked') } })
    expect(n).toBe(ACADEMIC_KEYS.length)
  })
})

describe('M5 and unknown credits on the board', () => {
  const w = JSON.parse(JSON.stringify(wire)) as WireDegree
  w.rows.find((r) => r.code === 'INFT 3101')!.term_choice!.selected = '1271' // stuck in Winter 2027, belongs to Fall 2027
  const v = buildDegree(w, 'Winter 2027', [{ code: 'GARC 2001', title: 'x', term: 'Winter 2027', units: 3 }])
  const cs = coursesOf(v)
  it('the display rule sets it aside (16) but a plan write is strict (19)', () => {
    expect(termLoads(cs, EMPTY_DRAFT).get('Winter 2027')!.credits).toBe(16)
    expect(termLoads(cs, EMPTY_DRAFT, false).get('Winter 2027')!.credits).toBe(19)
    expect(termLoads(cs, EMPTY_DRAFT, false).get('Winter 2027')!.deferred).toEqual([])
  })
  it('a course with unknown units is listed, never counted as 0', () => {
    const noUnits = JSON.parse(JSON.stringify(w)) as WireDegree
    noUnits.rows.find((r) => r.code === 'DACS 2202')!.units = null
    const l = termLoads(coursesOf(buildDegree(noUnits, 'Winter 2027')), EMPTY_DRAFT).get('Winter 2027')!
    expect(l.unknown.map((c) => c.code)).toEqual(['DACS 2202'])
    expect(l.credits).toBe(9 + 3) // the other enrolled three (9) and the stuck planned one (3), not 4 + more
  })
})

describe('L5: one open-term rule, with the same tie-break everywhere', () => {
  const rows = [
    { course_label: 'A 1: x', term: 'Spring 2027', status: 'Classes available' },
    { course_label: 'B 2: x', term: 'Winter 2027', status: 'Classes available' },
  ]
  it('a tie goes to the earlier term, whatever the order of the rows', () => {
    expect(openTerm(rows)).toBe('Winter 2027')
    expect(openTerm([...rows].reverse())).toBe('Winter 2027')
    expect(openTermOf(rows)).toBe('Winter 2027')
    expect(termOrder('Winter 2027')).toBeLessThan(termOrder('Spring 2027'))
  })
})

describe('L7: a 200 that is not JSON is an error, not a silent null', () => {
  afterEach(() => vi.unstubAllGlobals())
  it('throws an ApiError with a clear message', async () => {
    vi.stubGlobal('fetch', async () => ({ ok: true, status: 200, text: async () => '<!doctype html><html></html>' }))
    await expect(api.status()).rejects.toBeInstanceOf(ApiError)
    await expect(api.status()).rejects.toThrow(/not JSON/)
  })
  it('an empty 200 and a JSON 200 still work', async () => {
    vi.stubGlobal('fetch', async () => ({ ok: true, status: 200, text: async () => '{"signedIn":true,"userHint":"x"}' }))
    await expect(api.status()).resolves.toMatchObject({ signedIn: true })
  })
})

describe('M1: the sign-in screen asks for one acknowledgement before it signs in', () => {
  it('the button is off until the statements are loaded and accepted', () => {
    const html = renderToStaticMarkup(<Login signedInAs={null} onDone={() => {}} />)
    expect(html).toContain('I have read these and accept them')
    expect(html).toMatch(/<button[^>]*type="submit"[^>]*disabled/)
  })
})

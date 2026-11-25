import { describe, expect, it } from 'vitest'
import { curriculumTerms, planningMap, termFromOrder, buildDegree, type WireDegree } from './degree'
import wire from './fixtures/degree-wire.json'

const w = wire as unknown as WireDegree
const view = buildDegree(w, 'Winter 2027')
const course = (code: string) => view.years.flatMap((y) => y.semesters).flatMap((s) => s.blocks).flatMap((b) => (b.kind === 'course' && b.course.code === code ? [b.course] : []))[0]

describe('the degree tree, from the real capture (sanitized), as the page uses it', () => {
  it('has 4 years, 12 semesters and 39 rows, each in its year and semester', () => {
    expect(view.years).toHaveLength(4)
    expect(view.years.flatMap((y) => y.semesters)).toHaveLength(12)
    const blocks = view.years.flatMap((y) => y.semesters).flatMap((s) => s.blocks)
    expect(blocks).toHaveLength(39)
    expect(view.years[0].semesters[0].name).toBe('910-901-R Semester 1')
    expect(view.years[0].semesters[0].summary).toContain('Total Courses Required: 4')
  })
  it('classifies courses against the open term', () => {
    expect(course('COMM 1010').status).toBe('completed') // Fall 2025
    expect(course('DACS 2202').status).toBe('inProgress') // Winter 2027
    expect(course('SOFT 2301').status).toBe('planned') // a dropdown with a later term
    expect(course('SOFT 2301').plannable).toBe(true)
    expect(course('DACS 2202').plannable).toBe(false)
  })
  it('reads units and never turns a missing one into 0', () => {
    expect(course('DACS 2202').units).toBe(4)
    expect(course('COMM 1010').units).toBe(3)
    const lists = view.years.flatMap((y) => y.semesters).flatMap((s) => s.blocks).filter((b) => b.kind === 'group')
    expect(lists.length).toBeGreaterThanOrEqual(7)
  })
  it('credits per term: Winter 2027 is the four in-progress courses (13), and each term is checked against its limit', () => {
    const t = view.byTerm.find((x) => x.term === 'Winter 2027')!
    expect(t.credits).toBe(13) // 4+3+3+3 (GARC 2001 sits inside a Course List, so it is not a row of the tree)
    expect(t.limit.max).toBe(18)
    expect(view.byTerm.every((x) => x.over === x.credits > x.limit.max)).toBe(true)
    // in order of time: Fall 2025 before Winter 2026 before Spring 2026 ...
    expect(view.byTerm[0].term).toBe('Fall 2025')
    expect(view.byTerm[1].term).toBe('Winter 2026')
  })
  it('progress counts completed, in progress and planned credits of the listed courses', () => {
    const p = view.progress
    expect(p.completed + p.inProgress + p.planned + p.unplanned).toBe(p.total)
    expect(p.inProgress).toBe(13)
    expect(p.unknown).toBe(0)
  })
})


describe('courses PeopleSoft counts that the tree does not list', () => {
  const garc = { code: 'GARC 2001', title: 'Human Development in Qatar', term: 'Winter 2027', units: 3 }
  it('count towards their term (Winter 2027 is 16, not 13) and do not double count a tree course', () => {
    const plain = buildDegree(w, 'Winter 2027')
    const withExtra = buildDegree(w, 'Winter 2027', [garc, { code: 'DACS 2202', title: 'x', term: 'Winter 2027', units: 4 }])
    expect(plain.byTerm.find((t) => t.term === 'Winter 2027')!.credits).toBe(13)
    expect(withExtra.byTerm.find((t) => t.term === 'Winter 2027')!.credits).toBe(16)
    expect(withExtra.extras.map((c) => c.code)).toEqual(['GARC 2001'])
  })
  it('an extra is never movable and its units stay unknown when unknown', () => {
    const v = buildDegree(w, 'Winter 2027', [{ ...garc, units: null }])
    expect(v.extras[0].plannable).toBe(false)
    expect(v.extras[0].units).toBeNull()
  })
})


describe('an enrolled course being dropped no longer counts towards its term', () => {
  const garc = { code: 'GARC 2001', title: 'x', term: 'Winter 2027', units: 3 }
  it('Winter 2027 is 16 with GARC 2001 and 13 once it is being dropped; a tree course can be dropped too', () => {
    expect(buildDegree(w, 'Winter 2027', [garc]).byTerm.find((t) => t.term === 'Winter 2027')!.credits).toBe(16)
    expect(buildDegree(w, 'Winter 2027', [garc], ['GARC 2001']).byTerm.find((t) => t.term === 'Winter 2027')!.credits).toBe(13)
    expect(buildDegree(w, 'Winter 2027', [garc], ['garc2001', 'DACS 2202']).byTerm.find((t) => t.term === 'Winter 2027')!.credits).toBe(9)
  })
})

describe("the degree's own order of terms (the planning semester)", () => {
  it('term order round-trips and the semesters map to Fall, Winter, Spring in turn', () => {
    expect(termFromOrder(6075)).toBe('Fall 2025')
    expect(termFromOrder(6076)).toBe('Winter 2026')
    expect(termFromOrder(6077)).toBe('Spring 2026')
    expect(termFromOrder(6078)).toBe('Fall 2026')
    const t = curriculumTerms(w)
    expect([0, 1, 2, 3, 4, 5, 6, 7, 8].map((s) => t.get(s))).toEqual(['Fall 2025', 'Winter 2026', 'Spring 2026', 'Fall 2026', 'Winter 2027', 'Spring 2027', 'Fall 2027', 'Winter 2028', 'Spring 2028'])
    expect(t.get(11)).toBe('Spring 2029')
  })
  it('agrees with where PeopleSoft has the not-yet-taken courses planned by default (the capture)', () => {
    const m = planningMap(w)
    for (const r of w.rows) {
      if (!r.term_choice || !r.code) continue
      const planned = r.term_choice.options.find((o) => o.code === r.term_choice!.selected)?.label
      expect(m[r.code], r.code).toBe(planned)
    }
    expect(m['INFT 3101']).toBe('Fall 2027')
    expect(m['SOFT 2301']).toBe('Spring 2027')
  })
})

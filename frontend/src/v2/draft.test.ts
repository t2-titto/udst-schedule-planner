import { describe, expect, it } from 'vitest'
import wire from './fixtures/degree-wire.json'
import { buildDegree, type WireDegree } from './degree'
import { academicYearOf, allowedTerms, changeCount, coursesOf, EMPTY_DRAFT, moveCourse, movable, suggestTerm, termLoads, termOf, termsOfAcademicYear, warningsFor } from './draft'
import { prerequisiteOf } from './prereq'
import { commit, redo, startHistory, undo, HISTORY_LIMIT } from './history'

const w = wire as unknown as WireDegree
const view = buildDegree(w, 'Winter 2027')
const courses = coursesOf(view)
const optionsOf = (row: number) => (w.rows.find((r) => r.row === row)!.term_choice!.options.map((o) => ({ term: o.label, code: o.code })))
const soft = courses.find((c) => c.code === 'SOFT 2301')!
const allowed = allowedTerms(soft, optionsOf(soft.row), 'Winter 2027')

describe('draft moves', () => {
  it('only planned courses move; taken ones do not', () => {
    expect(movable(soft)).toBe(true)
    const done = courses.find((c) => c.status === 'completed')!
    expect(movable(done)).toBe(false)
    expect(moveCourse(EMPTY_DRAFT, done, 'Spring 2027', ['Spring 2027'])).toBe(EMPTY_DRAFT)
  })
  it('allowed terms start at the open term and are in order', () => {
    expect(allowed[0]).toBe('Winter 2027')
    expect(allowed).not.toContain('Fall 2026')
    expect(allowed).toEqual([...allowed].sort((a, b) => allowed.indexOf(a) - allowed.indexOf(b)))
  })
  it('a move changes the term and the totals; moving back removes the change', () => {
    const before = termLoads(courses, EMPTY_DRAFT)
    const d = moveCourse(EMPTY_DRAFT, soft, 'Fall 2027', allowed)
    expect(termOf(soft, d)).toBe('Fall 2027')
    const after = termLoads(courses, d)
    expect(after.get('Fall 2027')!.credits).toBe((before.get('Fall 2027')?.credits ?? 0) + 3)
    expect(after.get('Spring 2027')?.credits ?? 0).toBe(before.get('Spring 2027')!.credits - 3)
    expect(changeCount(d)).toBe(1)
    expect(changeCount(moveCourse(d, soft, 'Spring 2027', allowed))).toBe(0)
  })
  it('refuses a term PeopleSoft does not offer or one in the past', () => {
    expect(moveCourse(EMPTY_DRAFT, soft, 'Fall 2026', allowed)).toBe(EMPTY_DRAFT)
    expect(moveCourse(EMPTY_DRAFT, soft, 'Winter 2099', allowed)).toBe(EMPTY_DRAFT)
  })
  it('does not un-plan (that is Phase 4)', () => {
    expect(moveCourse(EMPTY_DRAFT, soft, null, allowed)).toBe(EMPTY_DRAFT)
  })
})

describe('academic year', () => {
  it('Fall Y, Winter Y+1, Spring Y+1', () => {
    expect(termsOfAcademicYear(2026)).toEqual(['Fall 2026', 'Winter 2027', 'Spring 2027'])
    expect(academicYearOf('Winter 2027')).toBe(2026)
    expect(academicYearOf('Fall 2027')).toBe(2027)
  })
})

describe('warnings', () => {
  it('says so when a term goes over its limit and clears it when moved away', () => {
    let d = EMPTY_DRAFT
    const winter = termLoads(courses, d).get('Winter 2027')!
    const filler = courses.filter((c) => movable(c) && c !== soft)
    expect(winter.credits).toBe(13)
    // push planned courses into Winter 2027 until over 18
    for (const c of filler) d = moveCourse(d, c, 'Winter 2027', allowedTerms(c, optionsOf(c.row), 'Winter 2027'))
    d = moveCourse(d, soft, 'Winter 2027', allowed)
    const over = warningsFor(courses, d, new Map(), 'Winter 2027').filter((x) => x.kind === 'credits')
    expect(termLoads(courses, d).get('Winter 2027')!.credits).toBeGreaterThan(18)
    expect(over.some((x) => x.term === 'Winter 2027')).toBe(true)
    expect(warningsFor(courses, EMPTY_DRAFT, new Map(), 'Winter 2027').filter((x) => x.kind === 'credits' && x.term === 'Winter 2027')).toHaveLength(0)
  })
  it('warns when a course is moved before its prerequisite', () => {
    const a = courses.find((c) => c.code === 'SOFT 2301')!
    const pre = courses.find((c) => c.status === 'planned' && c !== a && movable(c))!
    const req = prerequisiteOf(a.code, `Prerequisite(s): ${pre.code.replace(' ', '')}`)!
    const map = new Map([[a.code, req]])
    let d = moveCourse(EMPTY_DRAFT, pre, 'Fall 2027', allowedTerms(pre, optionsOf(pre.row), 'Winter 2027'))
    d = moveCourse(d, a, 'Spring 2027', allowed)
    expect(warningsFor(courses, d, map, 'Winter 2027').some((x) => x.kind === 'prerequisite' && x.course === a.code)).toBe(true)
    d = moveCourse(d, a, 'Fall 2028', allowed)
    expect(warningsFor(courses, d, map, 'Winter 2027').some((x) => x.kind === 'prerequisite')).toBe(false)
  })
  it('a prerequisite already completed never warns', () => {
    const done = courses.find((c) => c.status === 'completed')!
    const req = prerequisiteOf(soft.code, `Prerequisite(s): ${done.code.replace(' ', '')}`)!
    expect(warningsFor(courses, EMPTY_DRAFT, new Map([[soft.code, req]]), 'Winter 2027').filter((x) => x.kind === 'prerequisite')).toHaveLength(0)
  })
})

describe('suggestion', () => {
  it('picks the allowed term with the fewest credits that fits, nearest on a tie', () => {
    const s = suggestTerm(soft, EMPTY_DRAFT, courses, allowed, 'Winter 2027')!
    const loads = termLoads(courses.filter((c) => c !== soft), EMPTY_DRAFT)
    const credits = (t: string) => loads.get(t)?.credits ?? 0
    const fits = allowed.filter((t) => t !== 'Winter 2027' && credits(t) + 3 <= (t.startsWith('Spring') ? 12 : 18))
    const min = Math.min(...fits.map(credits))
    expect(credits(s)).toBe(min)
    expect(s).not.toBe('Winter 2027')
  })
  it('is null when nothing fits', () => {
    expect(suggestTerm(soft, EMPTY_DRAFT, courses, [], 'Winter 2027')).toBeNull()
  })
})

describe('history of a draft', () => {
  it('steps back and forward through 50 edits and Discard all returns the first state', () => {
    let h = startHistory(EMPTY_DRAFT)
    const terms = ['Fall 2027', 'Winter 2028']
    for (let i = 0; i < 60; i++) h = commit(h, moveCourse(h.present, soft, terms[i % 2], allowed), i * 1000)
    expect(h.past.length).toBe(HISTORY_LIMIT)
    for (let i = 0; i < HISTORY_LIMIT; i++) h = undo(h)
    expect(h.past.length).toBe(0)
    h = redo(h)
    expect(h.past.length).toBe(1)
    expect(changeCount(startHistory(EMPTY_DRAFT).present)).toBe(0)
  })
})


describe('the real Winter 2027: 16 enrolled credits include a Course List course', () => {
  const v = buildDegree(w, 'Winter 2027', [{ code: 'GARC 2001', title: 'Human Development in Qatar', term: 'Winter 2027', units: 3 }])
  const cs = coursesOf(v)
  const s = cs.find((c) => c.code === 'SOFT 2301')!
  const opts2 = w.rows.find((r) => r.row === s.row)!.term_choice!.options.map((o) => ({ term: o.label, code: o.code }))
  it('moving SOFT 2301 into Winter 2027 makes 19 and is flagged over the limit (the board used to say 16)', () => {
    const d = moveCourse(EMPTY_DRAFT, s, 'Winter 2027', allowedTerms(s, opts2, 'Winter 2027'))
    expect(termLoads(cs, d).get('Winter 2027')!.credits).toBe(19)
    expect(warningsFor(cs, d, new Map(), 'Winter 2027').some((x) => x.kind === 'credits' && x.term === 'Winter 2027')).toBe(true)
  })
})


describe('room made by a drop', () => {
  it('moving a 3-credit course into Winter 2027 fits (16 of 18) once GARC 2001 is being dropped, and not before', () => {
    const base = [{ code: 'GARC 2001', title: 'x', term: 'Winter 2027', units: 3 }]
    const kept = coursesOf(buildDegree(w, 'Winter 2027', base))
    const dropped = coursesOf(buildDegree(w, 'Winter 2027', base, ['GARC 2001']))
    const pick = (cs: typeof kept) => cs.find((c) => c.code === 'INFT 3101')!
    const into = (cs: typeof kept) => moveCourse(EMPTY_DRAFT, pick(cs), 'Winter 2027', ['Winter 2027'])
    expect(termLoads(kept, into(kept)).get('Winter 2027')!.credits).toBe(19)
    expect(termLoads(dropped, into(dropped)).get('Winter 2027')!.credits).toBe(16)
  })
})

describe('the planning-semester rule on the board', () => {
  // INFT 3101 sits in Winter 2027 in PeopleSoft (stuck there after an enrolment was dropped) but belongs to Fall 2027 in the degree.
  const stuck = JSON.parse(JSON.stringify(w)) as WireDegree
  stuck.rows.find((r) => r.code === 'INFT 3101')!.term_choice!.selected = '1271'
  const v = buildDegree(stuck, 'Winter 2027', [{ code: 'GARC 2001', title: 'x', term: 'Winter 2027', units: 3 }])
  const cs = coursesOf(v)
  const inft = cs.find((c) => c.code === 'INFT 3101')!
  it('a term over its limit does not count a planned course that belongs to another term, and counts it where it belongs', () => {
    const loads = termLoads(cs, EMPTY_DRAFT)
    const winter = loads.get('Winter 2027')!
    expect(winter.credits).toBe(16)
    expect(winter.over).toBe(false)
    expect(winter.deferred.map((c) => c.code)).toEqual(['INFT 3101'])
    expect(loads.get('Fall 2027')!.credits).toBe(12) // DACS 3101, INFT 3101, INFT 2201, DACS 3203
  })
  it('a term that is not over counts everything as PeopleSoft lists it', () => {
    const lighter = coursesOf(buildDegree(stuck, 'Winter 2027', [], ['DACS 2202']))
    const winter = termLoads(lighter, EMPTY_DRAFT).get('Winter 2027')!
    expect(winter.deferred).toEqual([])
    expect(winter.courses.map((c) => c.code)).toContain('INFT 3101')
  })
  it('a course the student moved there in this draft is never set aside', () => {
    const soft = cs.find((c) => c.code === 'SOFT 2301')!
    const opts3 = stuck.rows.find((r) => r.row === soft.row)!.term_choice!.options.map((o) => ({ term: o.label, code: o.code }))
    const d = moveCourse(EMPTY_DRAFT, soft, 'Winter 2027', allowedTerms(soft, opts3, 'Winter 2027'))
    const winter = termLoads(cs, d).get('Winter 2027')!
    expect(winter.courses.map((c) => c.code)).toContain('SOFT 2301')
    expect(winter.deferred.map((c) => c.code)).not.toContain('SOFT 2301')
  })
  void inft
})

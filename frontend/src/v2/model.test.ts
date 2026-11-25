import { describe, expect, it } from 'vitest'
import type { ClassOption, Component, Course } from '../engine/types'
import { redoChange, swappable, applySuggestion, asSchedule, blocksOf, buildCourses, effective, emptyState, fixesFor, isLocked, overlapsOf, type PCourse } from './model'

const comp = (nbr: string, day: number, start: number, end: number, kind = 'Lecture'): Component => ({
  dates: '', kind, classNbr: nbr, section: '1', room: 'R', instructors: ['A'], meetings: [{ day, start, end }], seatsOpen: 5, seatsTotal: 20,
})
const opt = (id: string, ...c: Component[]): ClassOption => ({ id, optionNo: id, status: 'Open', components: c })
const held = (code: string, ...c: Component[]): ClassOption => ({ id: `enrolled:${code}`, optionNo: 'Enrolled', status: 'Open', components: c })

const dsai: PCourse = { code: 'DSAI 2201', title: 't', term: 'W', options: [opt('d1', comp('1', 1, 570, 660)), opt('d2', comp('2', 3, 570, 660))], held: held('DSAI 2201', comp('1', 1, 570, 660)) }
const infs: PCourse = { code: 'INFS 2201', title: 't', term: 'W', options: [opt('i1', comp('5', 1, 480, 660)), opt('i2', comp('6', 2, 480, 660)), opt('i3', comp('7', 3, 700, 800))] }

describe('lab-only changes are not offered for a held course', () => {
  const lec = comp('1', 1, 570, 660)
  const h: PCourse = { code: 'LL 1000', title: '', term: 'W', held: held('LL 1000', lec, comp('2', 3, 480, 540, 'Laboratory')), options: [opt('a', lec, comp('2', 3, 480, 540, 'Laboratory')), opt('b', lec, comp('3', 4, 480, 540, 'Laboratory')), opt('c', comp('9', 2, 570, 660), comp('4', 4, 480, 540, 'Laboratory'))] }
  it('only an option that changes the lecture too can be swapped to', () => {
    expect(h.options.map((o) => swappable(h, o))).toEqual([true, false, true])
    expect(fixesFor(h, [h], emptyState()).map((o) => o.id)).toEqual(['c'])
  })
})

describe('what the calendar shows', () => {
  it('an enrolled course is locked to what is held until it is unlocked', () => {
    const s = emptyState()
    expect(isLocked(dsai, s)).toBe(true)
    expect(effective(dsai, s)?.id).toBe('enrolled:DSAI 2201')
    const u = { ...s, unlocked: ['DSAI 2201'], choice: { 'DSAI 2201': 'd2' } }
    expect(isLocked(dsai, u)).toBe(false)
    expect(effective(dsai, u)?.id).toBe('d2')
  })

  it('a removed course shows nothing', () => {
    expect(effective(infs, { ...emptyState(), removed: ['INFS 2201'] })).toBeNull()
  })

  it('an overlap is found and named, and is never hidden', () => {
    const s = { ...emptyState(), choice: { 'INFS 2201': 'i1' } }
    const o = overlapsOf(blocksOf([dsai, infs], s))
    expect(o).toHaveLength(1)
    expect([o[0].a.courseCode, o[0].b.courseCode].sort()).toEqual(['DSAI 2201', 'INFS 2201'])
    expect(o[0].start).toBe(570)
    expect(o[0].end).toBe(660)
  })

  it('classes of one course never overlap each other as far as the overlap list goes', () => {
    const c: PCourse = { code: 'X 1000', title: '', term: 'W', options: [opt('x', comp('1', 1, 480, 600), comp('2', 1, 540, 660))] }
    expect(overlapsOf(blocksOf([c], emptyState()))).toHaveLength(0)
  })

  it('the fix for an overlap is the nearest combination that overlaps nothing', () => {
    const s = { ...emptyState(), choice: { 'INFS 2201': 'i1' } }
    const fixes = fixesFor(infs, [dsai, infs], s)
    expect(fixes.map((f) => f.id)).toEqual(['i2', 'i3'])
  })

  it('a ranked suggestion moves every course that is free to move, and leaves locked ones', () => {
    const s = emptyState()
    const sched = asSchedule([dsai, infs], { ...s, choice: { 'INFS 2201': 'i2' } })
    const next = applySuggestion([dsai, infs], s, { ...sched, picks: sched.picks.map((p) => (p.course.code === 'DSAI 2201' ? { ...p, option: dsai.options[1] } : p)) })
    expect(next.choice['INFS 2201']).toBe('i2')
    expect(next.choice['DSAI 2201']).toBeUndefined() // locked
  })

  it('held courses come from the enrolled list, new ones from the plan', () => {
    const plan: Course[] = [{ code: 'INFS 2201', title: 'DB', term: 'W', options: infs.options }, { code: 'DSAI 2201', title: 'x', term: 'W', options: [] }]
    const courses = buildCourses(plan, { 'DSAI 2201': dsai.held! }, [{ row: 0, class_nbr: '1', component: 'Lecture', section: '1', course: 'DSAI 2201 Intro to AI', schedule: [], status: 'Enrolled' }], { 'DSAI 2201': { options: dsai.options } }, 'W')
    expect(courses.map((c) => c.code)).toEqual(['DSAI 2201', 'INFS 2201'])
    expect(courses[0].title).toBe('Intro to AI')
    expect(courses[0].options).toHaveLength(2)
    expect(courses[1].held).toBeUndefined()
  })
})

import { guidesFor, nearestGuide } from './guides'
import { bestEffort, suggestFixes } from './fixes'
import { DEFAULT_SETTINGS } from '../engine/types'

describe('drag guides', () => {
  const free = { ...infs, options: [opt('i1', comp('5', 1, 480, 660), comp('50', 3, 480, 540, 'Laboratory')), opt('i2', comp('6', 2, 480, 660), comp('60', 4, 480, 540, 'Laboratory')), opt('i3', comp('7', 3, 700, 800), comp('70', 4, 600, 660, 'Laboratory'))] }
  const st = { ...emptyState(), choice: { 'INFS 2201': 'i1' } }
  it('lists every real place with the other class of the same course and a score', () => {
    const b = blocksOf([dsai, free], st).find((x) => x.courseCode === 'INFS 2201' && x.kind === 'Lecture')!
    const gs = guidesFor([dsai, free], st, b, DEFAULT_SETTINGS)
    expect(gs).toHaveLength(3)
    const i2 = gs.find((g) => g.optionId === 'i2')!
    expect(i2.here).toEqual([{ day: 2, start: 480, end: 660 }])
    expect(i2.others.map((o) => o.kind)).toEqual(['Laboratory'])
    expect(Number.isFinite(i2.score)).toBe(true)
    expect(gs.find((g) => g.optionId === 'i1')!.current).toBe(true)
    expect(gs.find((g) => g.optionId === 'i1')!.clash).toBe(true) // overlaps the locked DSAI class on Mon
    expect(gs.find((g) => g.optionId === 'i2')!.clash).toBe(false)
  })
  it('snaps to the nearest real place', () => {
    const b = blocksOf([dsai, free], st)[0]
    const gs = guidesFor([dsai, free], st, blocksOf([dsai, free], st).find((x) => x.courseCode === 'INFS 2201' && x.kind === 'Lecture')!, DEFAULT_SETTINGS)
    expect(nearestGuide(gs, 2, 500)?.optionId).toBe('i2')
    expect(nearestGuide(gs, 3, 690)?.optionId).toBe('i3')
    expect(b.courseCode).toBe('DSAI 2201')
  })
  it('a locked course has no guides', () => {
    const b = blocksOf([dsai], emptyState())[0]
    expect(guidesFor([dsai], emptyState(), b, DEFAULT_SETTINGS)).toEqual([])
  })
})

describe('when no suggestion exists', () => {
  const x: PCourse = { code: 'XXX 1000', title: '', term: 'W', options: [opt('x1', comp('1', 1, 600, 700))] }
  const y: PCourse = { code: 'YYY 1000', title: '', term: 'W', options: [opt('y1', comp('2', 1, 650, 750))] }
  it('says why and offers fixes that really fix it', () => {
    const r = suggestFixes([x, y], emptyState(), DEFAULT_SETTINGS)
    expect(r.reasons.length).toBeGreaterThan(0)
    expect(r.fixes.some((f) => /Remove XXX 1000/.test(f.label) && f.state)).toBe(true)
    const last = r.fixes[r.fixes.length - 1]
    expect(typeof last.state).toBe('function')
  })
  it('a rule that blocks gets a loosen button', () => {
    const late: PCourse = { code: 'LLL 1000', title: '', term: 'W', options: [opt('l1', comp('1', 1, 1100, 1200))] }
    const settings = { ...DEFAULT_SETTINGS, rules: { ...DEFAULT_SETTINGS.rules, latestEnd: 18 * 60 } }
    const r = suggestFixes([late], emptyState(), settings)
    expect(typeof r.fixes[0].settings).toBe('function')
    expect(r.fixes[0].settings!(settings).rules.latestEnd).toBeNull()
  })
  it('best effort picks the least overlapping combination', () => {
    const z: PCourse = { code: 'ZZZ 1000', title: '', term: 'W', options: [opt('z1', comp('1', 1, 600, 700)), opt('z2', comp('2', 2, 600, 700))] }
    expect(bestEffort([x, z], emptyState()).choice['ZZZ 1000']).toBe('z2')
  })
})

describe('locked classes that overlap', () => {
  const l1: PCourse = { code: 'LA 1000', title: '', term: 'W', options: [], held: held('LA 1000', comp('1', 1, 600, 700)) }
  const l2: PCourse = { code: 'LB 1000', title: '', term: 'W', options: [], held: held('LB 1000', comp('2', 1, 650, 750)) }
  it('is named as the reason', () => {
    const r = suggestFixes([l1, l2], emptyState(), DEFAULT_SETTINGS)
    expect(r.reasons[0]).toMatch(/LA 1000 and LB 1000 are locked and overlap/)
  })
})

describe('an unlocked enrolled course may always stay where it is', () => {
  // DSAI-like: the held combination shares its lecture with itself, so "lab only" must not rule it out.
  const lec = comp('1', 1, 570, 660)
  const lab = comp('2', 3, 570, 660, 'Laboratory')
  const h: PCourse = { code: 'KK 1000', title: '', term: 'W', held: held('KK 1000', lec, lab), options: [opt('same', lec, lab), opt('lab2', lec, comp('3', 4, 480, 540, 'Laboratory')), opt('other', comp('9', 2, 570, 660), comp('4', 4, 480, 540, 'Laboratory'))] }
  it('the held combination is allowed, a lab-only change is not', () => {
    expect(h.options.map((o) => swappable(h, o))).toEqual([true, false, true])
  })
  it('the solver keeps the course with its held combination as a choice', async () => {
    const { solverCourses } = await import('./model')
    const sc = solverCourses([h], { ...emptyState(), unlocked: ['KK 1000'] })
    expect(sc[0].options.map((o) => o.id)).toEqual(['same', 'other'])
  })
})

describe('a held course with an extra auto-added class', () => {
  it('is still "the same as held", not a lab-only change', () => {
    const lec = comp('1', 1, 570, 660), lab = comp('2', 3, 570, 660, 'Laboratory'), extra = comp('99', 4, 480, 540, 'Laboratory')
    const h: PCourse = { code: 'EX 1000', title: '', term: 'W', held: held('EX 1000', lec, lab, extra), options: [opt('o1', lec, lab)] }
    expect(redoChange(h, h.options[0])).toBe(false)
    expect(swappable(h, h.options[0])).toBe(true)
  })
})

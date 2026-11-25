import { describe, expect, it } from 'vitest'
import wire from './fixtures/degree-wire.json'
import { buildDegree, type WireDegree } from './degree'
import { attentionList, countsFor, openTermOf, scheduleIssues, waitTransitions, type ClassRow } from './attention'
import { emptyState, type PCourse } from './model'
import { prerequisiteOf } from './prereq'

const w = wire as unknown as WireDegree
const row = (label: string, term: string, status: string): ClassRow => ({ course_label: label, term, status })
const rows: ClassRow[] = [
  row('A 1: x', 'Winter 2027', 'Enrolled'), row('B 2: x', 'Winter 2027', 'Enrolled'), row('C 3: x', 'Winter 2027', 'Classes available'),
  row('D 4: x', 'Winter 2027', 'No classes available'), row('E 5: x', 'Spring 2027', 'No classes available'),
]

describe('counts and the open term', () => {
  it('counts enrolled and planned for the open term only', () => {
    expect(countsFor(rows, 'Winter 2027')).toEqual({ enrolled: 2, planned: 1, noClasses: 1, waitlisted: 0 })
    expect(openTermOf(rows)).toBe('Winter 2027')
    expect(openTermOf([row('A 1: x', 'Fall 2026', 'Enrolled'), row('B 2: x', 'Winter 2027', 'Enrolled')])).toBe('Winter 2027')
  })
})

describe('the attention list', () => {
  const view = buildDegree(w, 'Winter 2027')
  const none = { prereqs: new Map(), overlaps: [], full: [] }
  it('a planned course with no classes this term is listed, one in another term is not', () => {
    const l = attentionList({ term: 'Winter 2027', rows, view: null, ...none })
    expect(l.map((i) => i.kind)).toEqual(['noclasses'])
    expect(l[0].text).toContain('D 4')
  })
  it('a waitlisted class is listed, saying waitlists are off when the switch is off', () => {
    const l = attentionList({ term: 'Winter 2027', rows: [row('W 9: x', 'Winter 2027', 'Waitlisted')], view: null, ...none })
    expect(l).toHaveLength(1)
    expect(l[0]).toMatchObject({ kind: 'waitlist' })
    expect(l[0].text).toContain('Waitlists are off')
  })
  it('with waitlists on it says you are not enrolled, and reports a class that left the waitlist', () => {
    const l = attentionList({ term: 'Winter 2027', rows: [row('W 9: x', 'Winter 2027', 'Waitlisted')], view: null, waitlistOn: true, waitMoved: ['V 1 is now enrolled: you came off the waitlist (Winter 2027).'], ...none })
    expect(l[0].text).toContain('You are not enrolled')
    expect(l[0].text).toContain('unverified')
    expect(l[1].text).toContain('V 1 is now enrolled')
  })
  it('waitTransitions names what happened to a waitlisted class between two reads', () => {
    const was = [row('A 1: x', 'Winter 2027', 'Waitlisted'), row('B 2: x', 'Winter 2027', 'Waitlisted'), row('C 3: x', 'Winter 2027', 'Waitlisted'), row('D 4: x', 'Winter 2027', 'Waitlisted')]
    const now = [row('A 1: x', 'Winter 2027', 'Enrolled'), row('B 2: x', 'Winter 2027', 'Classes available'), row('C 3: x', 'Winter 2027', 'Waitlisted')]
    const t = waitTransitions(was, now)
    expect(t).toHaveLength(3)
    expect(t[0]).toContain('A 1 is now enrolled')
    expect(t[1]).toContain('B 2 is no longer waitlisted')
    expect(t[2]).toContain('D 4 is no longer on your class list')
    expect(waitTransitions([], now)).toEqual([])
  })
  it('a term over its limit is listed for the open term and later ones, never for the past', () => {
    const over = JSON.parse(JSON.stringify(w)) as WireDegree
    over.rows.find((r) => r.code === 'DACS 3101')!.units = '10.00' // Fall 2027: 10 + 3 + 3 + 3 = 19 of 18, all belonging to Fall 2027
    const l = attentionList({ term: 'Winter 2027', rows: [], view: buildDegree(over, 'Winter 2027'), ...none })
    expect(l.filter((i) => i.kind === 'over').map((i) => i.text)).toEqual(['Fall 2027 has 19 credits; the limit is 18.'])
    expect(attentionList({ term: 'Winter 2027', rows: [], view, ...none }).filter((i) => i.kind === 'over')).toEqual([])
    // the same overload in a term that is already past is not shown
    expect(attentionList({ term: 'Spring 2028', rows: [], view: buildDegree(over, 'Spring 2028'), ...none }).filter((i) => i.kind === 'over')).toEqual([])
  })
  it('an unmet prerequisite (text already read) is listed; a met one is not', () => {
    const planned = view.years.flatMap((y) => y.semesters).flatMap((s) => s.blocks).flatMap((b) => (b.kind === 'course' && b.course.status === 'planned' ? [b.course] : []))
    const early = planned.find((c) => c.term === 'Spring 2027')! // the nearest planned course
    const late = planned.find((c) => c.term === 'Winter 2029')! // planned after it
    const unmet = new Map([[early.code, prerequisiteOf(early.code, `Prerequisite(s): ${late.code.replace(' ', '')}`)!]])
    const l = attentionList({ term: 'Winter 2027', rows: [], view, prereqs: unmet, overlaps: [], full: [] }).filter((i) => i.kind === 'prereq')
    expect(l).toHaveLength(1)
    expect(l[0].text).toContain(early.code)
    const done = view.years.flatMap((y) => y.semesters).flatMap((s) => s.blocks).flatMap((b) => (b.kind === 'course' && b.course.status === 'completed' ? [b.course] : []))[0]
    const met = new Map([[early.code, prerequisiteOf(early.code, `Prerequisite(s): ${done.code.replace(' ', '')}`)!]])
    expect(attentionList({ term: 'Winter 2027', rows: [], view, prereqs: met, overlaps: [], full: [] }).filter((i) => i.kind === 'prereq')).toEqual([])
  })
  it('overlaps and full classes saved from the last schedule are listed', () => {
    const l = attentionList({ term: 'Winter 2027', rows: [], view: null, prereqs: new Map(), overlaps: ['A 1 and B 2 overlap'], full: ['C 3: the chosen class is full or closed'] })
    expect(l.map((i) => i.kind)).toEqual(['overlap', 'full'])
  })
})

describe('issues found in a built schedule', () => {
  const comp = (nbr: string, day: number, start: number, end: number, seats: number | null) => ({ dates: '', kind: 'Lecture', classNbr: nbr, section: '1', room: '', instructors: ['X'], meetings: [{ day, start, end }], seatsOpen: seats, seatsTotal: 20 })
  const opt = (id: string, status: string, ...c: ReturnType<typeof comp>[]) => ({ id, optionNo: id, status, components: c })
  const pc = (code: string, o: ReturnType<typeof opt>): PCourse => ({ code, title: '', term: 'Winter 2027', options: [o] })
  it('finds an overlap and a full class, and nothing when the schedule is clean', () => {
    const a = pc('A 1', opt('a', 'Open', comp('1', 1, 540, 600, 5)))
    const b = pc('B 2', opt('b', 'Open', comp('2', 1, 570, 630, 0)))
    const s = { ...emptyState(), choice: { 'A 1': 'a', 'B 2': 'b' } }
    const issues = scheduleIssues([a, b], s)
    expect(issues.overlaps).toEqual(['A 1 and B 2 overlap'])
    expect(issues.full).toEqual(['B 2: the chosen class is full or closed'])
    const clean = scheduleIssues([pc('A 1', opt('a', 'Open', comp('1', 1, 540, 600, 5))), pc('B 2', opt('b', 'Open', comp('2', 2, 540, 600, 5)))], { ...emptyState(), choice: { 'A 1': 'a', 'B 2': 'b' } })
    expect(clean).toEqual({ overlaps: [], full: [] })
  })
})

import { describe, expect, it } from 'vitest'
import wire from './fixtures/degree-wire.json'
import { buildDegree, type WireDegree } from './degree'
import { allowedTerms, coursesOf, EMPTY_DRAFT, moveCourse, type Draft } from './draft'
import { compareTrees, moveProblems, moveRequest, planSave } from './savePlan'

const w = wire as unknown as WireDegree
const view = buildDegree(w, 'Winter 2027')
const courses = coursesOf(view)
const by = (code: string) => courses.find((c) => c.code === code)!
const opts = (code: string) => w.rows.find((r) => r.row === by(code).row)!.term_choice!.options.map((o) => ({ term: o.label, code: o.code }))
const move = (d: Draft, code: string, to: string) => moveCourse(d, by(code), to, allowedTerms(by(code), opts(code), 'Winter 2027'))

describe('the moves of a draft', () => {
  it('one move carries the year, the position in the year, the term code and the destination credits', () => {
    const p = planSave(view, w, 'Winter 2027', move(EMPTY_DRAFT, 'SOFT 2301', 'Fall 2027'))
    expect(p.stuck).toBeNull()
    expect(p.moves).toHaveLength(1)
    const m = p.moves[0]
    expect([m.year, m.index, m.toCode, m.fromCode, m.from, m.to]).toEqual([1, 8, '1278', '1275', 'Spring 2027', 'Fall 2027'])
    expect(m.after).toBe(15) // 12 credits already in Fall 2027 + 3
    expect(moveRequest(m)).toEqual({ args: { year: 1, index: 8, term_code: '1278' }, credits: { term: 'Fall 2027', limit: 18, after: 15, unknown: [] }, expect: ['SOFT2301'] })
    expect(moveProblems(m)).toEqual([])
  })
  it('a draft that leaves a term over its limit cannot be ordered', () => {
    let d = EMPTY_DRAFT
    for (const c of ['DACS 3101', 'INFT 3101', 'INFT 2201', 'DACS 3203', 'DACS 3201', 'SOFT 2301']) d = move(d, c, 'Winter 2027')
    const p = planSave(view, w, 'Winter 2027', d)
    expect(p.stuck).toContain('credit limit')
    expect(p.moves.length).toBeLessThan(6)
  })
  it('a move out of a full term goes before the move into it', () => {
    // Fall 2027 holds 12 credits; fill it to 18 with Winter 2028 courses, then swap SOFT 2301 in while another leaves.
    let d = EMPTY_DRAFT
    d = move(d, 'DACS 3201', 'Fall 2027')
    d = move(d, 'DACS 4101', 'Fall 2027') // Fall 2027 now 18
    d = move(d, 'SOFT 2301', 'Fall 2027') // 21 unless something leaves first
    d = move(d, 'DACS 3101', 'Winter 2028') // leaves Fall 2027
    d = move(d, 'INFT 3101', 'Winter 2028')
    const p = planSave(view, w, 'Winter 2027', d)
    expect(p.stuck).toBeNull()
    const order = p.moves.map((m) => m.course.code)
    expect(order.indexOf('DACS 3101')).toBeLessThan(order.indexOf('DACS 4101'))
    for (const m of p.moves) expect(m.after).toBeLessThanOrEqual(m.to.startsWith('Spring') ? 12 : 18)
  })
  it('a Spring move is limited to 12 credits', () => {
    let d = EMPTY_DRAFT
    for (const c of ['DACS 3101', 'INFT 3101', 'INFT 2201']) d = move(d, c, 'Spring 2028')
    const p = planSave(view, w, 'Winter 2027', d)
    expect(p.moves.every((m) => m.after <= 12)).toBe(true)
  })
})

describe('reading the whole plan back', () => {
  const clone = (): WireDegree => JSON.parse(JSON.stringify(w))
  const moved = [{ course: by('SOFT 2301'), toCode: '1278' }]
  it('is clean when only the moved course changed', () => {
    const after = clone()
    after.rows.find((r) => r.code === 'SOFT 2301')!.term_choice!.selected = '1278'
    expect(compareTrees(w, moved, after)).toEqual([])
  })
  it('names a course that did not move, one that moved by itself, and a missing row', () => {
    const none = clone()
    expect(compareTrees(w, moved, none)[0]).toContain('SOFT 2301')
    const other = clone()
    other.rows.find((r) => r.code === 'SOFT 2301')!.term_choice!.selected = '1278'
    other.rows.find((r) => r.code === 'DACS 3101')!.term_choice!.selected = '1288'
    expect(compareTrees(w, moved, other).join(' ')).toContain('DACS 3101')
    const shorter = clone()
    shorter.rows.pop()
    expect(compareTrees(w, moved, shorter).join(' ')).toContain('rows')
  })
})

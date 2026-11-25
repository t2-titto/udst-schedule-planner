import { describe, expect, it } from 'vitest'
import wire from './fixtures/degree-wire.json'
import { buildDegree, type WireDegree } from './degree'
import { availabilityOf, candidates, nextToCheck, OPEN_TTL_MS, remember, searchCandidates, termCode } from './ready'

const w = wire as unknown as WireDegree
const view = buildDegree(w, 'Winter 2027')

describe('which degree courses can be added to the open term', () => {
  const list = candidates(view, 'Winter 2027', [])
  it('only degree courses that are not taken, not in progress, not in the open term', () => {
    expect(list.length).toBeGreaterThan(10)
    for (const c of list) {
      expect(c.course.plannable).toBe(true)
      expect(['planned', 'unplanned']).toContain(c.course.status)
      expect(c.plannedTerm).not.toBe('Winter 2027')
    }
    expect(list.map((c) => c.code)).not.toContain('INFS 2201') // in progress
    expect(list.map((c) => c.code)).not.toContain('INFS 2101') // completed
  })
  it('a course already in the schedule is left out, however its code is written', () => {
    const without = candidates(view, 'Winter 2027', ['SOFT2301: Software Project Management'])
    expect(without.map((c) => c.code)).not.toContain('SOFT 2301')
    expect(without.length).toBe(list.length - 1)
  })
  it('the nearest planned term comes first', () => {
    expect(list[0].code).toBe('SOFT 2301') // Spring 2027, the nearest
    const terms = list.map((c) => c.plannedTerm)
    expect(terms[0]).toBe('Spring 2027')
  })
})

describe('the search box', () => {
  const list = candidates(view, 'Winter 2027', [])
  it('finds by code with or without the space, and by title, among degree courses only', () => {
    expect(searchCandidates(list, 'soft2301').map((c) => c.code)).toEqual(['SOFT 2301'])
    expect(searchCandidates(list, 'SOFT 23').map((c) => c.code)).toEqual(['SOFT 2301'])
    expect(searchCandidates(list, 'project management').map((c) => c.code)).toEqual(['SOFT 2301'])
    expect(searchCandidates(list, 'zzz')).toEqual([])
    expect(searchCandidates(list, '  ')).toEqual(list)
  })
})

describe('term codes and the availability memory', () => {
  it('maps terms to PeopleSoft codes', () => {
    expect(termCode('Winter 2027')).toBe('1271')
    expect(termCode('Spring 2027')).toBe('1275')
    expect(termCode('Fall 2027')).toBe('1278')
    expect(termCode('Winter 2028')).toBe('1281')
    expect(termCode('Summer 2027')).toBeNull()
  })
  it('remembers a check for six hours and then asks again', () => {
    let c = { keys: {}, open: {} }
    c = remember(c, 'Winter 2027', 'DACS 3101', 0, 1000)
    c = remember(c, 'Winter 2027', 'INFS 2201', 13, 1000)
    expect(availabilityOf(c, 'Winter 2027', 'DACS3101', 2000)).toEqual({ state: 'closed', at: 1000 })
    expect(availabilityOf(c, 'Winter 2027', 'INFS 2201', 2000)).toEqual({ state: 'open', options: 13, at: 1000 })
    expect(availabilityOf(c, 'Winter 2027', 'INFS 2201', 1000 + OPEN_TTL_MS + 1).state).toBe('unknown')
    expect(availabilityOf(c, 'Spring 2027', 'INFS 2201', 2000).state).toBe('unknown')
  })
  it('the next course to check skips known and in-flight ones', () => {
    const list = candidates(view, 'Winter 2027', [])
    let c = { keys: {}, open: {} }
    expect(nextToCheck(list, c, 'Winter 2027', 5, new Set())!.code).toBe(list[0].code)
    c = remember(c, 'Winter 2027', list[0].code, 0, 5)
    expect(nextToCheck(list, c, 'Winter 2027', 6, new Set())!.code).toBe(list[1].code)
    expect(nextToCheck(list, c, 'Winter 2027', 6, new Set(['DACS3101', list[1].code.replace(/\s/g, '')]))!.code).not.toBe(list[1].code)
  })
})

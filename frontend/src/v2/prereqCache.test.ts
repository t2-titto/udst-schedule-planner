import { describe, expect, it } from 'vitest'
import { nextToRead, prereqMap, PREREQ_TTL_MS } from './prereqCache'

const list = [{ code: 'A 1', year: 1, index: 0 }, { code: 'B 2', year: 1, index: 1 }, { code: 'C 3', year: 2, index: 0 }]

describe('the prerequisite memory', () => {
  it('asks for missing or day-old texts in list order, skipping the ones already tried', () => {
    expect(nextToRead(list, {}, 1000, new Set())!.code).toBe('A 1')
    const cache = { 'A 1': { text: 'x', at: 1000 } }
    expect(nextToRead(list, cache, 2000, new Set())!.code).toBe('B 2')
    expect(nextToRead(list, cache, 2000, new Set(['B 2']))!.code).toBe('C 3')
    expect(nextToRead(list, cache, 1000 + PREREQ_TTL_MS + 1, new Set(['B 2', 'C 3']))!.code).toBe('A 1')
    expect(nextToRead(list, { 'A 1': { text: 'x', at: 1 }, 'B 2': { text: 'x', at: 1 }, 'C 3': { text: 'x', at: 1 } }, 2, new Set())).toBeNull()
  })
  it('parses the good texts and drops the old ones', () => {
    const cache = { 'B 2': { text: 'Prerequisite(s): MATH1020 OR COMP2010', at: 100 }, 'C 3': { text: 'Prerequisite(s): MATH1020', at: 1 } }
    const m = prereqMap(cache, 100 + 10)
    expect(m.get('B 2')!.courses).toEqual(['MATH 1020', 'COMP 2010'])
    expect(prereqMap(cache, PREREQ_TTL_MS + 50).has('B 2')).toBe(true)
    expect(prereqMap(cache, PREREQ_TTL_MS + 50).has('C 3')).toBe(false)
  })
})

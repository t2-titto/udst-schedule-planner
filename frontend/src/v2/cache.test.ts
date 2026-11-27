import { describe, expect, it } from 'vitest'
import { ago, fingerprint, HELD_TTL_MS, usableHeld, type Cache } from './cache'

const codeOf = (c: string) => c.split(' ').slice(0, 2).join(' ')
const row = (nbr: string, course: string) => ({ row: 0, class_nbr: nbr, component: 'Lecture', section: '1', course, schedule: [], status: 'Enrolled' })

describe('what a refresh may reuse', () => {
  const cache: Cache = { term: 'Winter 2027', keys: {}, held: { 'AAA 100': { fp: '1,2', options: [], at: 1000 } } }
  it('reuses a course read for the same term and the same enrolled classes', () => {
    expect(usableHeld(cache, 'Winter 2027', 'AAA 100', '1,2', 2000)).toEqual(cache.held['AAA 100'])
  })
  it('re-reads when the enrolled classes changed, the term differs, or the copy is old', () => {
    expect(usableHeld(cache, 'Winter 2027', 'AAA 100', '1,3', 2000)).toBeUndefined()
    expect(usableHeld(cache, 'Fall 2026', 'AAA 100', '1,2', 2000)).toBeUndefined()
    expect(usableHeld(cache, 'Winter 2027', 'AAA 100', '1,2', 1000 + HELD_TTL_MS + 1)).toBeUndefined()
  })
  it('never reuses without a fingerprint or without a cache', () => {
    expect(usableHeld(cache, 'Winter 2027', 'AAA 100', '', 2000)).toBeUndefined()
    expect(usableHeld(null, 'Winter 2027', 'AAA 100', '1,2', 2000)).toBeUndefined()
  })
  it('fingerprints a course by its sorted class numbers', () => {
    const rows = [row('9', 'AAA 100 A'), row('3', 'AAA 100 A'), row('5', 'BBB 200 B')]
    expect(fingerprint(rows, 'AAA 100', codeOf)).toBe('3,9')
  })
})

describe('updated ago', () => {
  it('reads naturally', () => {
    expect(ago(0, 10_000)).toBe('just now')
    expect(ago(0, 3 * 60_000)).toBe('3 min ago')
    expect(ago(0, 2 * 3600_000)).toBe('2 h ago')
  })
})

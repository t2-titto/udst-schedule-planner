import { describe, expect, it } from 'vitest'
import { corequisiteOf, isMet, normCode, prerequisiteOf } from './prereq'

describe('prerequisite text', () => {
  it('"X is the pre-requisite of A, B" gives this course its requirement, with a placement alternative kept as a note', () => {
    const p = prerequisiteOf('MATH 1030', 'MATH1020 OR AMPII Score of 85% is the pre-requisite of MATH1030, AETN2302, RSST3001, AEMA2212.')!
    expect(p.courses).toEqual(['MATH 1020'])
    expect(p.notes).toEqual(['AMPII Score of 85%'])
    expect(p.op).toBe('or')
  })
  it('the other direction, "Prerequisite(s): ..."', () => {
    const p = prerequisiteOf('MATH 1020', 'Prerequisite(s): MATH1010 OR AMPII Score of 75%')!
    expect(p.courses).toEqual(['MATH 1010'])
    expect(p.notes).toEqual(['AMPII Score of 75%'])
  })
  it('a plain "X is a pre-requisite for A and B"', () => {
    const p = prerequisiteOf('INFS 3103', 'INFS2201 is a pre-requisite for INFS4105 and INFS3103.')!
    expect(p.courses).toEqual(['INFS 2201'])
    expect(p.op).toBe('none')
  })
  it('a list that uses an ampersand, as PeopleSoft writes it for INFS 2101', () => {
    const p = prerequisiteOf('INFS 2101', 'INFS1201 is a pre-requisite for INFS3102, INFS2101 & SOFT2101.')!
    expect(p.courses).toEqual(['INFS 1201'])
  })
  it('says nothing for a course the sentence does not name, and keeps the raw text', () => {
    expect(prerequisiteOf('COMM 1010', 'MATH1010 is the pre-requisite of MATH1020, AHHG2030')).toBeNull()
    expect(prerequisiteOf('MATH 1020', 'MATH1010 is the pre-requisite of MATH1020, AHHG2030')!.raw).toContain('MATH1010')
  })
  it('met / not met / open', () => {
    const p = prerequisiteOf('MATH 1030', 'MATH1020 OR AMPII Score of 85% is the pre-requisite of MATH1030')!
    expect(isMet(p, new Set(['MATH 1020']))).toBe(true)
    expect(isMet(p, new Set())).toBeNull() // a placement score may satisfy it
    const q = prerequisiteOf('INFS 3103', 'INFS2201 is a pre-requisite for INFS3103')!
    expect(isMet(q, new Set())).toBe(false)
    expect(isMet(q, new Set(['INFS 2201']))).toBe(true)
  })
  it('codes are normalised', () => {
    expect([normCode('MATH1020'), normCode('MATH 1020'), normCode('INFS3103'), normCode('x')]).toEqual(['MATH 1020', 'MATH 1020', 'INFS 3103', 'x'])
  })
  it('parses co-requisites from live PeopleSoft phrasing, including semicolon-separated clauses', () => {
    expect(corequisiteOf('AEMA 2221', 'AEMA2121 is the co-requisite for AEMA2221')).toEqual(['AEMA 2121'])
    expect(corequisiteOf('AEMA 2121', 'AEMA1213 OR AEMA1242 are the prerequisites of AEMA2121.')).toEqual([])
    const combined = 'AEMA1102 OR AEMR1102 is the prerequisite; AEMA2133 is the corequisite of AEMA2233.'
    expect(corequisiteOf('AEMA 2233', combined)).toEqual(['AEMA 2133'])
    expect(prerequisiteOf('AEMA 2233', combined)?.courses).toEqual(['AEMA 1102', 'AEMR 1102'])
    expect(corequisiteOf('AEMA 2221', 'Co-requisite(s): AEMA2121')).toEqual(['AEMA 2121'])
  })
})

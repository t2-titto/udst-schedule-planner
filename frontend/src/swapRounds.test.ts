import { describe, expect, it } from 'vitest'
import type { ClassOption, Component } from './engine/types'
import { blockerText, planSwapRounds, type RoundSwap } from './swapRounds'

const comp = (nbr: string, day: number, start: number, end: number): Component => ({ dates: '', kind: 'Lecture', classNbr: nbr, section: '1', room: '', instructors: ['A'], meetings: [{ day, start, end }], seatsOpen: null, seatsTotal: null })
const opt = (id: string, ...c: Component[]): ClassOption => ({ id, optionNo: id, status: 'Open', components: c })
const h = (m: number, min = 0) => m * 60 + min

// The real case from 2026-11-16 (times from PeopleSoft's own pages): DACS 2202, GARC 2001 and INFS 3102 swapped in one commit.
const dacs: RoundSwap = { code: 'DACS 2202', now: opt('n', comp('3382', 1, h(15, 30), h(17)), comp('3382b', 3, h(15, 30), h(17)), comp('3383', 0, h(14), h(17))), next: opt('x', comp('3380', 1, h(11), h(12, 30)), comp('3380b', 3, h(11), h(12, 30)), comp('3381', 0, h(11), h(14))), seatsLeft: 12 }
const garc: RoundSwap = { code: 'GARC 2001', now: opt('n', comp('2622', 0, h(9, 30), h(11)), comp('2623', 3, h(8), h(9, 30))), next: opt('x', comp('2616', 1, h(8), h(9, 30)), comp('2617', 2, h(9, 30), h(11))), seatsLeft: 27 }
const infs: RoundSwap = { code: 'INFS 3102', now: opt('n', comp('3633', 2, h(8), h(10)), comp('3634', 4, h(17), h(20))), next: opt('x', comp('3628', 3, h(14), h(16)), comp('3631', 0, h(8), h(11))), seatsLeft: 22 }
const dsai = opt('f', comp('3496', 0, h(20), h(22)), comp('3505', 1, h(9, 30), h(11)))

describe('swap rounds', () => {
  it('swaps that overlap nothing the others hold go out together', () => {
    const p = planSwapRounds([dacs], [dsai])
    expect(p.rounds.map((r) => r.map((s) => s.code))).toEqual([['DACS 2202']])
    expect(p.converted).toEqual([])
  })

  it('the real case: GARC and INFS 3102 wait for each other, so one of them becomes a drop and an enrolment', () => {
    const p = planSwapRounds([dacs, garc, infs], [dsai])
    expect(p.loops).toEqual([['GARC 2001', 'INFS 3102']])
    // GARC has the more seats left (27 against 22), so it is the one dropped and enrolled again.
    expect(p.converted.map((s) => s.code)).toEqual(['GARC 2001'])
    // The rest can be ordered: INFS 3102's new Wednesday class overlaps DACS 2202's current one, so DACS goes first.
    expect(p.rounds.map((r) => r.map((s) => s.code))).toEqual([['DACS 2202'], ['INFS 3102']])
    expect(p.waits['INFS 3102'][0].by).toBe('DACS 2202')
    expect(blockerText(p.waits['INFS 3102'][0])).toMatch(/waits for DACS 2202 to leave Wednesday/)
    expect(p.waits['DACS 2202']).toBeUndefined()
  })

  it('a chain with an order needs rounds but no loop', () => {
    const a: RoundSwap = { code: 'A 1000', now: opt('n', comp('1', 1, h(8), h(10))), next: opt('x', comp('2', 2, h(8), h(10))), seatsLeft: 5 }
    const b: RoundSwap = { code: 'B 2000', now: opt('n', comp('3', 2, h(8), h(10))), next: opt('x', comp('4', 3, h(8), h(10))), seatsLeft: 5 }
    // A's new class sits where B is today, so B moves first.
    const p = planSwapRounds([a, b], [])
    expect(p.rounds.map((r) => r.map((s) => s.code))).toEqual([['B 2000'], ['A 1000']])
    expect(p.loops).toEqual([])
    expect(p.waits['A 1000'][0].by).toBe('B 2000')
  })

  it('independent swaps share a round', () => {
    const a: RoundSwap = { code: 'A 1000', now: opt('n', comp('1', 1, h(8), h(10))), next: opt('x', comp('2', 1, h(10), h(12))), seatsLeft: 5 }
    const b: RoundSwap = { code: 'B 2000', now: opt('n', comp('3', 3, h(8), h(10))), next: opt('x', comp('4', 3, h(10), h(12))), seatsLeft: 5 }
    expect(planSwapRounds([a, b], []).rounds.map((r) => r.length)).toEqual([2])
  })

  it('a swap into a fixed course is not ordered away: it is kept so the plan can report the clash', () => {
    const a: RoundSwap = { code: 'A 1000', now: opt('n', comp('1', 1, h(8), h(10))), next: opt('x', comp('2', 0, h(20), h(21))), seatsLeft: 5 }
    const p = planSwapRounds([a], [dsai])
    expect(p.rounds.flat().map((s) => s.code)).toEqual(['A 1000'])
  })
})

describe('forced drop and re-enrol (lab-only changes)', () => {
  it('takes the forced course out of the rounds and marks it', () => {
    const mk = (code: string, day: number): RoundSwap => ({ code, now: opt(`${code}n`, comp(`${code}1`, day, 540, 600)), next: opt(`${code}x`, comp(`${code}2`, day + 1, 540, 600)), seatsLeft: 5 })
    const p = planSwapRounds([mk('A', 1), mk('B', 3)], [], ['A'])
    expect(p.converted.map((c) => c.code)).toEqual(['A'])
    expect(p.forced).toEqual(['A'])
    expect(p.rounds.flat().map((s) => s.code)).toEqual(['B'])
  })
})

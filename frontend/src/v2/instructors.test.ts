import { describe, expect, it } from 'vitest'
import type { ClassOption, Component } from '../engine/types'
import { instructorStats } from './instructors'
import { emptyState, pinnedOptions, solverCourses, type PCourse } from './model'

const comp = (nbr: string, kind: string, who: string, day = 1, start = 540, end = 600): Component => ({ dates: '', kind, classNbr: nbr, section: nbr, room: 'R', instructors: [who], meetings: [{ day, start, end }], seatsOpen: 5, seatsTotal: 20 })
const opt = (id: string, ...c: Component[]): ClassOption => ({ id, optionNo: id, status: 'Open', components: c })

const options = [
  opt('a', comp('1', 'Lecture', 'Dr A'), comp('11', 'Laboratory', 'Dr C')),
  opt('b', comp('1', 'Lecture', 'Dr A'), comp('12', 'Laboratory', 'Dr D')),
  opt('c', comp('2', 'Lecture', 'Dr B'), comp('11', 'Laboratory', 'Dr C')),
]

describe('instructor statistics', () => {
  const st = Object.fromEntries(instructorStats(options).map((s) => [s.name, s]))
  it('counts each class once, split into lectures and labs', () => {
    expect(st['Dr A']).toMatchObject({ lectures: 1, labs: 0 })
    expect(st['Dr C']).toMatchObject({ lectures: 0, labs: 1 })
    expect(instructorStats(options).map((s) => s.name).sort()).toEqual(['Dr A', 'Dr B', 'Dr C', 'Dr D'])
  })
  it('says which lecture a lab goes with, and with whom', () => {
    const lab11 = st['Dr C'].classes[0]
    expect(lab11.links.map((l) => [l.nbr, l.instructors[0]]).sort()).toEqual([['1', 'Dr A'], ['2', 'Dr B']])
    const lec1 = st['Dr A'].classes[0]
    expect(lec1.links.map((l) => l.nbr).sort()).toEqual(['11', '12'])
  })
})

describe('pins', () => {
  it('keep only the combinations that contain the pinned class', () => {
    expect(pinnedOptions(options, ['11']).options.map((o) => o.id)).toEqual(['a', 'c'])
    expect(pinnedOptions(options, ['11', '1']).options.map((o) => o.id)).toEqual(['a'])
  })
  it('a pin that contradicts an earlier one is left out, never emptying the course', () => {
    const r = pinnedOptions(options, ['12', '2'])
    expect(r.options.map((o) => o.id)).toEqual(['b'])
    expect(r.ignored).toEqual(['2'])
  })
  it('the solver only sees the pinned combinations of an unlocked course', () => {
    const c: PCourse = { code: 'X 1000', title: '', term: 'W', options }
    const sc = solverCourses([c], { ...emptyState(), pins: ['12'] })
    expect(sc[0].options.map((o) => o.id)).toEqual(['b'])
  })
  it('a pin of another course changes nothing', () => {
    const c: PCourse = { code: 'X 1000', title: '', term: 'W', options }
    expect(solverCourses([c], { ...emptyState(), pins: ['999'] })[0].options).toHaveLength(3)
  })
})

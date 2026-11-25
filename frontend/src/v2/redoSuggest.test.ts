import { describe, expect, it } from 'vitest'
import { DEFAULT_SETTINGS, type ClassOption, type Component } from '../engine/types'
import { emptyState, solverCourses, type PCourse } from './model'
import { redoSuggestions } from './redoSuggest'

const comp = (nbr: string, kind: string, day: number, start: number, end: number): Component => ({ dates: '', kind, classNbr: nbr, section: nbr, room: 'R', instructors: ['A'], meetings: [{ day, start, end }], seatsOpen: 5, seatsTotal: 20 })
const opt = (id: string, ...c: Component[]): ClassOption => ({ id, optionNo: id, status: 'Open', components: c })
const h = (n: number) => n * 60
const settings = { ...DEFAULT_SETTINGS, timing: { start: h(8), end: h(12), fade: h(1) }, rules: { ...DEFAULT_SETTINGS.rules, minFreeDays: 0 } }
const held = (code: string, ...c: Component[]): ClassOption => ({ id: `enrolled:${code}`, optionNo: 'Enrolled', status: 'Open', components: c })

// One lecture, two labs: the held lab is in the evening, the other in the morning (a lab-only change).
const lec = comp('1', 'Lecture', 1, h(9), h(10))
const evening = comp('2', 'Laboratory', 2, h(18), h(20))
const morning = comp('3', 'Laboratory', 2, h(9), h(11))
const enrolled: PCourse = { code: 'ENR 1000', title: '', term: 'W', held: held('ENR 1000', lec, evening), options: [opt('a', lec, evening), opt('b', lec, morning)] }
const unlocked = { ...emptyState(), unlocked: ['ENR 1000'] }

describe('drop and re-enrol suggestions', () => {
  it('the ordinary solver never offers a lab-only change, allowRedo does', () => {
    expect(solverCourses([enrolled], unlocked)[0].options.map((o) => o.id)).toEqual(['a'])
    expect(solverCourses([enrolled], unlocked, true)[0].options.map((o) => o.id)).toEqual(['a', 'b'])
  })
  it('shows the better schedule that needs a drop and re-enrol, with what it gains', () => {
    const r = redoSuggestions([enrolled], unlocked, settings)
    expect(r).toHaveLength(1)
    expect(r[0].redoCourses).toEqual(['ENR 1000'])
    expect(r[0].gain).toBeGreaterThan(0.05)
  })
  it('shows nothing when it would not matter (the lab-only change scores no better)', () => {
    const same: PCourse = { ...enrolled, options: [opt('a', lec, evening), opt('b', lec, comp('4', 'Laboratory', 2, h(19), h(21)))] }
    expect(redoSuggestions([same], unlocked, settings)).toEqual([])
  })
  it('shows nothing for a fresh enrolment, however many combinations it has', () => {
    const fresh: PCourse = { code: 'NEW 1000', title: '', term: 'W', options: [opt('a', lec, evening), opt('b', lec, morning)] }
    expect(redoSuggestions([fresh], emptyState(), settings)).toEqual([])
  })
  it('shows nothing while the course is locked', () => {
    expect(redoSuggestions([enrolled], emptyState(), settings)).toEqual([])
  })
})

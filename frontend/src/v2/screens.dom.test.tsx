// @vitest-environment jsdom
/** The screens run for real (effects, clicks, async), with the server API mocked. Closes the "only statically rendered" gap of the red-team report (L10). */
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import wire from './fixtures/degree-wire.json'
import type { ClassOption, Component } from '../engine/types'
import type { EnrolledState } from '../enrolled'

const calls: string[] = []
const mock = vi.hoisted(() => ({
  refreshRows: [] as { course_label: string; term: string; status: string }[],
  classRows: [] as { course_label: string; term: string; status: string }[],
  tree: null as unknown,
}))

vi.mock('../api', () => ({
  ApiError: class extends Error { status: number; constructor(m: string, s: number) { super(m); this.status = s } },
  api: {
    refresh: async () => ({ classes: { rows: mock.refreshRows, term_label: '', can_submit: true }, enrolled: null, stats: {} }),
    classes: async () => ({ rows: mock.classRows, term_label: '', can_submit: true }),
    writeStatus: async () => ({ armed: false, confirmMode: 'click', consent: { version: 'v', statements: [{ id: 'real', text: 'x' }] } }),
    writeArm: async () => { calls.push('arm') },
    writeDisarm: async () => { calls.push('disarm') },
    planApply: async () => { calls.push('planApply'); return { started: true } },
    planProgress: async () => ({ id: 1, phase: 'finished', finished: true, error: null, items: [] }),
    writeRead: async (tool: string) => {
      calls.push(`read:${tool}`)
      if (tool === 'degree_tree') return mock.tree
      if (tool === 'degree_requirement') return { course: 'ZZ 0', text: 'x' }
      throw new Error('unexpected ' + tool)
    },
  },
}))

import { Apply } from './Apply'
import { HomePanel } from './HomePanel'
import { emptyState, type PCourse } from './model'

const comp = (nbr: string, day: number, start: number, end: number): Component => ({ dates: '', kind: 'Lecture', classNbr: nbr, section: nbr, room: 'R', instructors: ['A'], meetings: [{ day, start, end }], seatsOpen: 5, seatsTotal: 20 })
const opt = (id: string, ...c: Component[]): ClassOption => ({ id, optionNo: id, status: 'Open', optionRow: 0, components: c })

beforeEach(() => { calls.length = 0; window.localStorage.clear() })
afterEach(() => cleanup())

describe('Apply, with a drop unticked (H1), run for real', () => {
  const g = opt('g', comp('100', 1, 540, 600)) // enrolled GONE 1000, Monday 9:00 to 10:00
  const n = opt('n', comp('200', 1, 570, 630)) // NEW 1000 takes that slot
  const courses: PCourse[] = [
    { code: 'GONE 1000', title: '', term: 'Winter 2027', options: [], held: g, units: 3 },
    { code: 'NEW 1000', title: '', term: 'Winter 2027', options: [n], units: 3, key: { strm: '1', crse_id: '2', crse_offer_nbr: '1', acad_career: 'C', institution: 'I' } as never, row: 3 },
  ]
  const state = { ...emptyState(), removed: ['GONE 1000'], choice: { 'NEW 1000': 'n' } }
  const enrolled: EnrolledState = { term: 'Winter 2027', termRow: 1, rows: [{ row: 0, class_nbr: '100', component: 'Lecture', section: '1', course: 'GONE 1000 x', schedule: [], status: 'Enrolled' }] as never, options: { 'GONE 1000': g }, unconfirmed: [] }

  it('Apply is on after the check; unticking the drop turns it off with a reason, ticking turns it on again, and nothing is sent while it is off', async () => {
    mock.refreshRows = [
      { course_label: 'GONE 1000: x', term: 'Winter 2027', status: 'Enrolled' },
      { course_label: 'NEW 1000: x', term: 'Winter 2027', status: 'Classes available' },
    ]
    render(<Apply courses={courses} planning={{}} state={state} enrolled={enrolled} onBack={() => {}} onDone={() => {}} onRefresh={async () => {}} onRefused={() => {}} />)
    const apply = () => screen.getByRole('button', { name: /^Apply$/ }) as HTMLButtonElement
    await waitFor(() => expect(apply().disabled).toBe(false))
    const box = screen.getByRole('checkbox', { name: /drop it/ })
    fireEvent.click(box) // untick: GONE 1000 stays enrolled, NEW 1000 still takes its slot
    await waitFor(() => expect(apply().disabled).toBe(true))
    expect(screen.getByText(/You kept GONE 1000/)).toBeTruthy()
    fireEvent.click(apply())
    expect(calls).not.toContain('planApply')
    fireEvent.click(box) // tick again
    await waitFor(() => expect(apply().disabled).toBe(false))
    expect(screen.queryByText(/You kept GONE 1000/)).toBeNull()
  })

  it('Apply sends the request only after the check, with termRow and dependencies the server accepts', async () => {
    mock.refreshRows = [
      { course_label: 'GONE 1000: x', term: 'Winter 2027', status: 'Enrolled' },
      { course_label: 'NEW 1000: x', term: 'Winter 2027', status: 'Classes available' },
    ]
    render(<Apply courses={courses} planning={{}} state={state} enrolled={enrolled} onBack={() => {}} onDone={() => {}} onRefresh={async () => {}} onRefused={() => {}} />)
    const apply = () => screen.getByRole('button', { name: /^Apply$/ }) as HTMLButtonElement
    await waitFor(() => expect(apply().disabled).toBe(false))
    fireEvent.click(apply())
    await waitFor(() => expect(calls).toContain('planApply'))
    expect(calls.indexOf('arm')).toBeLessThan(calls.indexOf('planApply'))
  })

  it('a change in PeopleSoft since the page loaded switches Apply off', async () => {
    mock.refreshRows = [{ course_label: 'GONE 1000: x', term: 'Winter 2027', status: 'Enrolled' }] // NEW 1000 is no longer planned there
    render(<Apply courses={courses} planning={{}} state={state} enrolled={enrolled} onBack={() => {}} onDone={() => {}} onRefresh={async () => {}} onRefused={() => {}} />)
    await waitFor(() => expect(screen.getByText(/PeopleSoft's plan changed/)).toBeTruthy())
    expect((screen.getByRole('button', { name: /^Apply$/ }) as HTMLButtonElement).disabled).toBe(true)
  })
})

describe('the home panel, run for real', () => {
  it('shows what needs attention from the class list and stops reading prerequisites while a schedule is being built', async () => {
    mock.classRows = [
      { course_label: 'A 1: x', term: 'Winter 2027', status: 'Enrolled' },
      { course_label: 'B 2: x', term: 'Winter 2027', status: 'No classes available' },
      { course_label: 'W 9: x', term: 'Winter 2027', status: 'Waitlisted' },
    ]
    mock.tree = wire
    const { rerender } = render(<HomePanel paused />)
    await waitFor(() => expect(screen.getByText(/B 2 is planned for Winter 2027 but has no classes/)).toBeTruthy())
    expect(screen.getByText(/W 9 is waitlisted/)).toBeTruthy()
    expect(screen.getByText(/Paused while your schedule is being built/)).toBeTruthy()
    expect(calls).not.toContain('read:degree_requirement')
    expect(calls).not.toContain('read:degree_tree')
    rerender(<HomePanel paused />)
  })
  it('reads the degree and then the prerequisites one at a time when nothing is paused, keeping only text that names the right course', async () => {
    mock.classRows = [{ course_label: 'A 1: x', term: 'Winter 2027', status: 'Enrolled' }]
    mock.tree = wire
    render(<HomePanel />)
    await waitFor(() => expect(calls).toContain('read:degree_requirement'), { timeout: 4000 })
    // the mocked answer names ZZ 0, never the course asked for: nothing of it may be kept
    await waitFor(() => expect(calls.filter((c) => c === 'read:degree_requirement').length).toBeGreaterThan(3), { timeout: 4000 })
    expect(window.localStorage.getItem('schedule-planner:v2:prereqs') ?? '{}').toBe('{}')
  })
})

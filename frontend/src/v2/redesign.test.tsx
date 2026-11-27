import { describe, expect, it } from 'vitest'
import { DEFAULT_SETTINGS, type ClassOption, type Component } from '../engine/types'
import { buildApplyRequest, wireProblems } from './applyRequest'
import { evalRuleTree, formatRuleTree, getCatalogRule, isPlacementRule } from './catalogRules'
import { completionFromView, discoverCompatibleCandidates, inferExemptedCourses, lookupCatalogKey } from './compatible'
import { creditsFor } from './credits'
import { buildDegree, type WireDegree } from './degree'
import wire from './fixtures/degree-wire.json'
import { guidesFor } from './guides'
import { emptyState, type PCourse } from './model'
import { buildSuggestions, DEFAULT_QUICK_SETUP } from './suggestions'

const comp = (nbr: string, kind: string, sec: string, teacher: string, day: number, start: number, end: number, open = 10, total = 30): Component => ({
  dates: '05/01/2027 - 14/04/2027',
  kind,
  classNbr: nbr,
  section: sec,
  room: 'B1-101',
  instructors: [teacher],
  meetings: [{ day, start, end }],
  seatsOpen: open,
  seatsTotal: total,
})

describe('Planie redesign helper invariants', () => {
  it('catalogKeys.json resolves public UDST courses cleanly for direct section lookup', () => {
    const key = lookupCatalogKey('DACS 2202')
    expect(key).not.toBeNull()
    expect(key?.crse_id).toBeTruthy()
    expect(key?.offer_nbr).toBe('1')
  })

  it('evalRuleTree enforces earnedCredits for Minimum N Credits and flags placement notes', () => {
    expect(evalRuleTree({ note: 'Minimum 80 Credits' }, new Set(), 60)).toBe(false)
    expect(evalRuleTree({ note: 'Minimum 80 Credits' }, new Set(), 85)).toBe(true)
    expect(evalRuleTree({ note: 'AMPII Score of 85%' }, new Set(), 120)).toBeNull()
    expect(isPlacementRule({ note: 'AMPII Score of 85%' })).toBe(true)
    expect(isPlacementRule({ note: 'Minimum 80 Credits' })).toBe(false)

    const formatted = formatRuleTree({ op: 'AND', items: ['INFT 1201', { op: 'OR', items: ['MATH 1010', 'MATH 1020'] }] })
    expect(formatted).toBe('INFT 1201 and (MATH 1010 or MATH 1020)')
  })

  it('compatible discovery marks eligible courses, co-reqs, and unlocks using strictly completed courses', () => {
    const view = buildDegree(wire as WireDegree, 'Winter 2027')
    const { eligible, locked, earnedCredits } = discoverCompatibleCandidates(view, 'Winter 2027', [])
    expect(earnedCredits).toBeGreaterThan(0)
    expect(eligible.length + locked.length).toBeGreaterThan(0)
    // Verify rule table integrity on known courses
    const acct = getCatalogRule('ACCT 1001')
    expect(acct?.unlocks?.length).toBeGreaterThan(0)
  })

  it('guidesFor respects lab-only redo opt-in and wait/closed availability filters', () => {
    const heldOpt: ClassOption = {
      id: 'h1',
      optionNo: '1',
      status: 'Open',
      components: [comp('3380', 'Lecture', '01', 'Dr. A', 0, 540, 600), comp('3381', 'Laboratory', '01L', 'Dr. A', 2, 600, 720)],
    }
    const labOnlyOpt: ClassOption = {
      id: 'h2',
      optionNo: '2',
      status: 'Open',
      components: [comp('3380', 'Lecture', '01', 'Dr. A', 0, 540, 600), comp('3382', 'Laboratory', '02L', 'Dr. B', 4, 600, 720)],
    }
    const closedOpt: ClassOption = {
      id: 'h3',
      optionNo: '3',
      status: 'Closed',
      components: [comp('3383', 'Lecture', '02', 'Dr. C', 1, 540, 600, 0, 30), comp('3384', 'Laboratory', '03L', 'Dr. C', 3, 600, 720, 0, 30)],
    }
    const c: PCourse = { code: 'DACS 2202', title: 'Networks', term: 'Winter 2027', units: 4, options: [heldOpt, labOnlyOpt, closedOpt], held: heldOpt }
    const st = { ...emptyState(), unlocked: ['DACS 2202'] }
    const lecBlock = { courseCode: 'DACS 2202', classNbr: '3380', kind: 'Lecture', room: 'B1', instructors: ['Dr. A'], day: 0, start: 540, end: 600, locked: false, held: true }

    const gsStrict = guidesFor([c], st, lecBlock, DEFAULT_SETTINGS, { open: true, waitlist: false, closed: false, allowRedo: false })
    expect(gsStrict.map((g) => g.optionId)).toEqual(['h1'])

    const gsFull = guidesFor([c], st, lecBlock, DEFAULT_SETTINGS, { open: true, waitlist: false, closed: true, allowRedo: true })
    expect(gsFull.map((g) => g.optionId)).toContain('h3')
    expect(gsFull.find((g) => g.optionId === 'h3')?.status).toBe('Closed')
  })

  it('creditsFor overrideLimit unlocks blocked state and buildApplyRequest serializes standalone plans cleanly', () => {
    const mk = (code: string, units: number): PCourse => ({
      code,
      title: code,
      term: 'Winter 2027',
      units,
      options: [{ id: '1', optionNo: '1', status: 'Open', components: [comp(code, 'Lecture', '01', 'Dr. A', 0, 540, 600)] }],
    })
    const cs = [mk('A 1', 4), mk('B 2', 4), mk('C 3', 4), mk('D 4', 4), mk('E 5', 4)] // 20 > 18
    const normal = creditsFor(cs, emptyState(), 'Winter 2027')
    expect(normal.blocked).toBe(true)
    expect(normal.overrideLimit).toBe(false)

    const over = creditsFor(cs, emptyState(), 'Winter 2027', {}, false, true)
    expect(over.blocked).toBe(false)
    expect(over.overrideLimit).toBe(true)

    const req = buildApplyRequest(
      { keeps: [], swaps: [], enrols: [], drops: [], blockers: [], checked: 0 },
      { rounds: [], converted: [], waits: {}, loops: [], forced: [] },
      { term: 'Winter 2027', termRow: -1, rows: [], options: {}, unconfirmed: [] },
      [],
      over,
      {},
      [{ course: 'MATH 2030', year: 1, index: 4, termCode: '1275', credits: { term: 'Spring 2027', limit: 12, after: 9, unknown: [] } }],
    )
    expect(req.plans.length).toBe(1)
    expect(req.plans[0].course).toBe('MATH 2030')
    expect(wireProblems(req)).toEqual([])
  })

  it('completionFromView counts prior-term inProgress courses as satisfied prerequisites for openTerm', () => {
    const view = buildDegree(
      {
        years: [{ index: 0, name: 'Y1', summary: '', state: 'open' }, { index: 1, name: 'Y2', summary: '', state: 'open' }],
        semesters: [{ index: 3, year: 1, name: 'Fall 2026', summary: '' }, { index: 4, year: 1, name: 'Winter 2027', summary: '' }],
        rows: [
          { row: 0, year: 1, semester: 3, label: 'INFT 3101', code: 'INFT 3101', title: 'Security Intro', category: 'Req', units: '3.00', term: 'Fall 2026', joined_by_and: true, has_select: false, has_requirement: false, term_choice: null },
          { row: 1, year: 1, semester: 4, label: 'DACS 2202', code: 'DACS 2202', title: 'Networks', category: 'Req', units: '3.00', term: 'Winter 2027', joined_by_and: true, has_select: false, has_requirement: false, term_choice: null },
        ],
      },
      'Winter 2027',
    )
    const { doneCodes, earnedCredits } = completionFromView(view, 'Winter 2027')
    expect(doneCodes.has('INFT 3101')).toBe(true)
    expect(doneCodes.has('DACS 2202')).toBe(false)
    expect(earnedCredits).toBe(3)
  })

  it('buildSuggestions produces Waitlist fallback and N-1 Best-fit suggestions when Open-only options clash', () => {
    const c1: PCourse = {
      code: 'A 101',
      title: 'Course A',
      term: 'Winter 2027',
      units: 3,
      held: { id: 'a1', optionNo: '1', status: 'Open', components: [comp('1', 'Lecture', '01', 'Dr. A', 0, 600, 690)] },
      options: [{ id: 'a1', optionNo: '1', status: 'Open', components: [comp('1', 'Lecture', '01', 'Dr. A', 0, 600, 690)] }],
    }
    const c2: PCourse = {
      code: 'B 202',
      title: 'Course B',
      term: 'Winter 2027',
      units: 3,
      options: [{ id: 'b1', optionNo: '1', status: 'Open', components: [comp('2', 'Lecture', '01', 'Dr. B', 0, 600, 690)] }],
    }
    const st = { ...emptyState(), unlocked: ['A 101'] }
    const partial = buildSuggestions([c1, c2], st, DEFAULT_SETTINGS, DEFAULT_QUICK_SETUP)
    expect(partial.top.length).toBeGreaterThan(0)
    expect(partial.top[0].skippedCodes).toEqual(['B 202'])
    expect(partial.top[0].label).toContain('1 of 2')
    expect(partial.top[0].detail).toContain('Removes B 202')
    expect(partial.partialNotice).toContain('No conflict-free suggestion fits all 2 courses')
    expect(partial.partialNotice).toContain('B 202')

    const c2Wl: PCourse = {
      ...c2,
      options: [
        ...c2.options,
        { id: 'b2', optionNo: '2', status: 'Wait List', waitlist: true, components: [{ ...comp('3', 'Lecture', '02', 'Dr. C', 2, 600, 690, 0, 20), waitPlaces: 5 }] },
      ],
    }
    const wlRes = buildSuggestions([c1, c2Wl], st, DEFAULT_SETTINGS, DEFAULT_QUICK_SETUP)
    expect(wlRes.top.length).toBeGreaterThan(0)
    expect(wlRes.top[0].usesWaitlist).toBe(true)
    expect(wlRes.top[0].skippedCodes).toBeUndefined()
    expect(wlRes.partialNotice).toBeNull()
  })

  it('applies strict suggestion preferences first and falls back gracefully with a notice if impossible', () => {
    const earlyOpt: ClassOption = { id: 'o-8am', optionNo: '1', status: 'Open', components: [comp('1', 'Lecture', '01', 'Dr. Early', 0, 480, 570)] }
    const lateOpt: ClassOption = { id: 'o-11am', optionNo: '2', status: 'Open', components: [comp('2', 'Lecture', '02', 'Dr. Late', 2, 660, 750)] }
    const both: PCourse = { code: 'C 301', title: 'Course C', term: 'Winter 2027', units: 3, options: [earlyOpt, lateOpt] }

    // Strict "No 8 AMs + Sun free" keeps only o-11am
    const strictOk = buildSuggestions([both], emptyState(), DEFAULT_SETTINGS, {
      ...DEFAULT_QUICK_SETUP,
      freeDays: [0],
      noEarly8am: true,
      strict: true,
    })
    expect(strictOk.prefNotice).toBeNull()
    expect(strictOk.top.every((card) => card.schedule.picks[0].option.id === 'o-11am')).toBe(true)

    // Strict "Sun free + Wed free" is impossible when only Sun and Wed sections exist -> graceful fallback + notice
    const fallback = buildSuggestions([both], emptyState(), DEFAULT_SETTINGS, {
      ...DEFAULT_QUICK_SETUP,
      freeDays: [0, 2],
      strict: true,
    })
    expect(fallback.top.length).toBeGreaterThan(0)
    expect(fallback.prefNotice).toContain('No full schedule satisfied every strict preference')
  })

  it('preserves untouched registered schedule as Original when no alternate full schedule fits', () => {
    const heldFull: ClassOption = { id: 'a1', optionNo: '1', status: 'Closed', components: [comp('1', 'Lecture', '01', 'Dr. A', 0, 600, 690, 0, 20)] }
    const heldB: ClassOption = { id: 'b1', optionNo: '1', status: 'Closed', components: [comp('2', 'Lecture', '02', 'Dr. B', 1, 600, 690, 0, 20)] }
    const c1: PCourse = { code: 'A 101', title: 'Course A', term: 'Winter 2027', units: 3, held: heldFull, options: [heldFull] }
    const c2: PCourse = { code: 'B 202', title: 'Course B', term: 'Winter 2027', units: 3, held: heldB, options: [heldB] }
    const sug = buildSuggestions([c1, c2], emptyState(), DEFAULT_SETTINGS, DEFAULT_QUICK_SETUP)
    expect(sug.top.length).toBeGreaterThan(0)
    expect(sug.top[0].isOriginal).toBe(true)
    expect(sug.top[0].label).toContain('Original')
    expect(sug.top[0].detail).toContain('Only fit')
  })

  it('infers exempted prerequisite courses (e.g. MATH 1020 completed omits MATH 1010 unless retaken)', () => {
    const exempted = inferExemptedCourses(new Set(['MATH 1020']))
    expect(exempted.get('MATH 1010')).toBe('MATH 1020')

    const customView = {
      years: [{
        index: 0,
        name: 'Year 1',
        summary: '',
        semesters: [{
          index: 1,
          year: 0,
          name: 'Sem 1',
          summary: '',
          credits: 6,
          unknownUnits: 0,
          blocks: [
            { kind: 'course' as const, course: { row: 1, year: 0, code: 'MATH 1010', title: 'Pre-Calc', category: '', units: 3, term: 'Winter 2027', status: 'planned' as const, plannable: true, hasRequirement: false } },
            { kind: 'course' as const, course: { row: 2, year: 0, code: 'MATH 1020', title: 'Calculus I', category: '', units: 3, term: 'Fall 2025', status: 'completed' as const, plannable: false, hasRequirement: false } },
          ],
        }],
      }],
      extras: [],
      byTerm: [],
      progress: { completed: 3, inProgress: 0, planned: 3, unplanned: 0, total: 6, unknown: 0 },
    }
    const normal = discoverCompatibleCandidates(customView, 'Winter 2027', [])
    expect(normal.exempted.get('MATH 1010')).toBe('MATH 1020')
    expect(normal.eligible.some((c) => c.code === 'MATH 1010')).toBe(false)

    const retaking = discoverCompatibleCandidates(customView, 'Winter 2027', [], ['MATH 1010'])
    expect(retaking.eligible.some((c) => c.code === 'MATH 1010')).toBe(true)
  })

  it('pairs "Course" and "Course (Lab)" courses automatically without merging their sections, and ignores online scoring while blocking online overlaps', () => {
    const pairView = {
      years: [{
        index: 0,
        name: 'Year 1',
        summary: '',
        semesters: [{
          index: 1,
          year: 0,
          name: 'Sem 1',
          summary: '',
          credits: 4,
          unknownUnits: 0,
          blocks: [
            { kind: 'course' as const, course: { row: 1, year: 0, code: 'CHEM 1900', title: 'Organic Foundations', category: '', units: 3, term: 'Winter 2027', status: 'planned' as const, plannable: true, hasRequirement: false } },
            { kind: 'course' as const, course: { row: 2, year: 0, code: 'CHEM 1901', title: 'Organic Foundations (Lab)', category: '', units: 1, term: 'Winter 2027', status: 'planned' as const, plannable: true, hasRequirement: false } },
          ],
        }],
      }],
      extras: [],
      byTerm: [],
      progress: { completed: 0, inProgress: 0, planned: 4, unplanned: 0, total: 4, unknown: 0 },
    }
    const paired = discoverCompatibleCandidates(pairView, 'Winter 2027', [])
    const lec = paired.eligible.find((c) => c.code === 'CHEM 1900')
    const lab = paired.eligible.find((c) => c.code === 'CHEM 1901')
    expect(lec?.labRole).toBe('lecture')
    expect(lec?.coreqs).toContain('CHEM 1901')
    expect(lab?.labRole).toBe('lab')
    expect(lab?.coreqs).toContain('CHEM 1900')

    // Online courses must be ignored in free-day/timing scoring, yet NEVER overlap each other at the exact same slot
    const onlineA: ClassOption = { id: 'oa', optionNo: '1', status: 'Open', components: [{ ...comp('1', 'Lecture', '01', 'Dr. A', 0, 540, 600), room: 'Online' }] }
    const onlineB1: ClassOption = { id: 'ob1', optionNo: '1', status: 'Open', components: [{ ...comp('2', 'Lecture', '01', 'Dr. B', 0, 540, 600), room: 'Online' }] }
    const onlineB2: ClassOption = { id: 'ob2', optionNo: '2', status: 'Open', components: [{ ...comp('3', 'Lecture', '02', 'Dr. B', 0, 660, 720), room: 'Online' }] }
    const cA: PCourse = { code: 'ONL 101', title: 'Online A', term: 'Winter 2027', units: 3, options: [onlineA] }
    const cB: PCourse = { code: 'ONL 102', title: 'Online B', term: 'Winter 2027', units: 3, options: [onlineB1, onlineB2] }
    const onlineSug = buildSuggestions([cA, cB], emptyState(), DEFAULT_SETTINGS, { ...DEFAULT_QUICK_SETUP, freeDays: [0], strict: true })
    expect(onlineSug.top.length).toBeGreaterThan(0)
    expect(onlineSug.prefNotice).toBeNull()
    expect(onlineSug.top[0].schedule.picks.find((p) => p.course.code === 'ONL 102')?.option.id).toBe('ob2')
  })
})

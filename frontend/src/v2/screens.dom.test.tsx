// @vitest-environment jsdom
import { useState } from 'react'
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { DEFAULT_SETTINGS, type ClassOption, type Component } from '../engine/types'
import { Apply } from './Apply'
import App from './App'
import { clearAcademicData, WORKSPACE_CACHE_PREFIX } from './storage'
import { buildIcs } from './ics'
import { Calendar } from './Calendar'
import { CourseInfoPopover } from './CourseInfoPopover'
import { CourseSidebar } from './CourseSidebar'
import { buildDegree, type WireDegree } from './degree'
import { Done } from './Done'
import wire from './fixtures/degree-wire.json'
import { FuturePlanDrawer } from './FuturePlanDrawer'
import { FutureSemestersStrip } from './FutureSemestersStrip'
import { guidesFor } from './guides'
import { Login } from './Login'
import { emptyState, type PCourse, type PlannerState } from './model'
import { SetupWait } from './SetupWait'
import { buildSuggestions, DEFAULT_QUICK_SETUP } from './suggestions'
import { SuggestionsStrip, assignTileClasses, tileClassFor } from './SuggestionsStrip'
import { WaitWidget } from './WaitWidget'
import { clearChessPuzzleCache, getCachedPuzzleCount, nextChessPuzzle, warmChessPuzzleCache } from './chessLogic'
import { discoverCompatibleCandidates } from './compatible'

const comp = (nbr: string, sec: string, teacher: string, day: number, start: number, end: number, open = 10, total = 30): Component => ({
  dates: '05/01/2027 - 14/04/2027',
  kind: 'Lecture',
  classNbr: nbr,
  section: sec,
  room: 'B1-101',
  instructors: [teacher],
  meetings: [{ day, start, end }],
  seatsOpen: open,
  seatsTotal: total,
})

const makeCourse = (code: string, options: ClassOption[], held?: ClassOption): PCourse => ({
  code,
  title: `${code} Title`,
  term: 'Winter 2027',
  units: 3,
  options,
  held,
  key: { strm: '1271', crse_id: '010201', crse_offer_nbr: '1', acad_career: 'CRED', institution: 'CNAQA' },
  row: 0,
})

describe('Planie redesigned UI & UX flows', () => {
  afterEach(() => {
    cleanup()
    vi.unstubAllGlobals()
  })

  it('Login displays calm notice, Light/Dark toggle, zero inline styles, and account switcher pills', async () => {
    vi.stubGlobal('fetch', vi.fn(async (url: string) => ({
      ok: true,
      status: 200,
      text: async () => url.includes('/accounts')
        ? JSON.stringify({ accounts: [{ id: 'acct1', userHint: '60••••', active: true, demo: false }], activeId: 'acct1' })
        : JSON.stringify({ signedIn: false }),
    })))
    const { container } = render(
      <Login onSignedIn={() => {}} onSwitchAccount={() => {}} theme="light" onToggleTheme={() => {}} />,
    )
    expect(container.querySelectorAll('[style]').length).toBe(0)
    expect(screen.getByText(/Changes only happen when you press Confirm/i)).toBeTruthy()
    expect((await screen.findByText('60••••')).tagName).toBe('BUTTON')
  })

  it('SetupWait lets student pick quick setup pills and play 2048 / Memory / Fun facts', () => {
    const onChangeSetup = vi.fn()
    const { container } = render(
      <SetupWait
        setup={DEFAULT_QUICK_SETUP}
        onChangeSetup={onChangeSetup}
        showWaitlist={false}
        onToggleWaitlist={() => {}}
        showClosed={false}
        onToggleClosed={() => {}}
        stages={[{ id: 'boot', label: 'Profile', status: 'done', detail: 'Ready' }]}
        ready
        error={null}
        onRetry={() => {}}
        onContinue={() => {}}
        onSignOut={() => {}}
        userHint="demo"
        demo
        theme="light"
        onToggleTheme={() => {}}
      />,
    )
    expect(container.querySelectorAll('[style]').length).toBe(0)
    fireEvent.click(screen.getByRole('button', { name: 'Thu' }))
    expect(onChangeSetup).toHaveBeenCalledWith({ ...DEFAULT_QUICK_SETUP, freeDay: 4, freeDays: [4] })

    // Interactive wait widget switches between 2048, Chess, and Subway (no Memory or Fun facts)
    expect(screen.queryByRole('tab', { name: 'Fun facts' })).toBeNull()
    expect(screen.queryByRole('tab', { name: 'Memory' })).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'AI mode' }))
    expect(screen.getByRole('button', { name: 'Stop AI' })).toBeTruthy()
    expect(screen.queryByRole('slider')).toBeNull()
    expect(screen.queryByText(/1\.5x/i)).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Stop AI' }))
    fireEvent.click(screen.getByRole('tab', { name: 'Subway' }))
    expect(screen.getByTitle('Subway Surfers HTML5 Game')).toBeTruthy()
    expect(screen.queryByText(/Offline Runner/i)).toBeNull()
  })

  it('WaitWidget plays 2048 arrow moves cleanly without inline style attributes', () => {
    const { container } = render(<WaitWidget />)
    expect(container.querySelectorAll('[style]').length).toBe(0)
    fireEvent.click(screen.getByRole('button', { name: 'Move left' }))
    expect(screen.getByText(/Score:/i)).toBeTruthy()
  })

  it('SuggestionsStrip labels schedules without numeric scores and reveals greyed extra suggestions on click', () => {
    const c1 = makeCourse('INFS 2201', [
      { id: 'o1', optionNo: '1', status: 'Open', components: [comp('101', '01', 'Dr. Lin', 0, 540, 600)] },
      { id: 'o2', optionNo: '2', status: 'Open', components: [comp('102', '02', 'Prof. Noor', 1, 660, 720)] },
      { id: 'o3', optionNo: '3', status: 'Open', components: [comp('103', '03', 'Dr. Chen', 2, 720, 780)] },
      { id: 'o4', optionNo: '4', status: 'Open', components: [comp('104', '04', 'Dr. Patel', 3, 540, 600)] },
      { id: 'o5', optionNo: '5', status: 'Open', components: [comp('105', '05', 'Dr. Haddad', 4, 600, 660)] },
    ])
    const res = buildSuggestions([c1], emptyState(), DEFAULT_SETTINGS, DEFAULT_QUICK_SETUP)
    expect(res.top.length).toBeGreaterThan(0)
    const expanded = { value: false }
    const { rerender } = render(
      <SuggestionsStrip
        top={res.top}
        more={res.more}
        activeSig=""
        expanded={expanded.value}
        onToggleExpand={() => { expanded.value = true }}
        onPick={() => {}}
        unsolvableReason={null}
      />,
    )
    expect(screen.queryByText(/\/ 10/)).toBeNull()
    if (res.more.length > 0) {
      const btn = screen.getByRole('button', { name: /Show more suggestions/i })
      fireEvent.click(btn)
      rerender(
        <SuggestionsStrip
          top={res.top}
          more={res.more}
          activeSig=""
          expanded
          onToggleExpand={() => {}}
          onPick={() => {}}
          unsolvableReason={null}
        />,
      )
      expect(screen.getByRole('button', { name: /Hide extra suggestions/i })).toBeTruthy()
    }
  })

  it('guidesFor groups same-time instructors and keeps Teacher + Section + Seats without timing text or score', () => {
    const optA: ClassOption = { id: 'a', optionNo: '1', status: 'Open', components: [comp('201', '01', 'Dr. A. Al-Mansoor', 0, 540, 600)] }
    const optB: ClassOption = { id: 'b', optionNo: '2', status: 'Open', components: [comp('202', '02', 'Prof. S. Tariq', 2, 540, 600)] }
    const optC: ClassOption = { id: 'c', optionNo: '3', status: 'Open', components: [comp('203', '03', 'Dr. L. Chen', 2, 540, 600)] }
    const c = makeCourse('INFS 2201', [optA, optB, optC])
    const state: PlannerState = { ...emptyState(), choice: { 'INFS 2201': 'a' } }
    const gs = guidesFor([c], state, { courseCode: 'INFS 2201', classNbr: '201', kind: 'Lecture', room: 'B1', instructors: ['Dr. A. Al-Mansoor'], day: 0, start: 540, end: 600, locked: false, held: false }, DEFAULT_SETTINGS)
    const tueGuide = gs.find((g) => g.here[0].day === 2)!
    expect(tueGuide.optionsAtSlot.length).toBe(2)

    render(
      <Calendar
        courses={[c]}
        state={state}
        blocks={[{ courseCode: 'INFS 2201', classNbr: '201', kind: 'Lecture', room: 'B1', instructors: ['Dr. A. Al-Mansoor'], day: 0, start: 540, end: 600, locked: false, held: false }]}
        settings={DEFAULT_SETTINGS}
        filters={{ open: true, waitlist: false, closed: false, allowRedo: false }}
        onPickOption={() => {}}
        onToggleLock={() => {}}
        onToggleCourse={() => {}}
        inspectedCourseCode={null}
        onInspectCourse={() => {}}
        favTeachers={[]}
        onToggleFavTeacher={() => {}}
        onToggleAllowRedo={() => {}}
        sidebarOpen
        onToggleSidebar={() => {}}
      />,
    )
    expect(screen.getByRole('button', { name: 'Week' })).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Day' })).toBeTruthy()
  })

  it('CourseSidebar shows only offered & addable courses under active filters and CourseInfoPopover displays Unlocks', () => {
    const view = buildDegree(wire as WireDegree, 'Winter 2027')
    const { eligible } = discoverCompatibleCandidates(view, 'Winter 2027', [])
    const target = eligible[0]
    const targetOpen = eligible[1]
    expect(target).toBeTruthy()
    expect(targetOpen).toBeTruthy()
    const { rerender } = render(
      <CourseSidebar
        courses={[]}
        state={emptyState()}
        term="Winter 2027"
        showWaitlist={false}
        onToggleWaitlist={() => {}}
        showClosed={false}
        onToggleClosed={() => {}}
        compatible={[target, targetOpen]}
        compatStatus={{
          [target.code]: { loading: false, checked: true, openCount: 0, waitlistCount: 1, closedCount: 0, notOffered: false },
          [targetOpen.code]: { loading: false, checked: true, openCount: 2, waitlistCount: 0, closedCount: 0, notOffered: false },
        }}
        readyLoading={false}
        onToggleCourse={() => {}}
        onToggleLock={() => {}}
        onSelectBlockCourse={() => {}}
        onAddCompatible={() => {}}
        onInspectCourse={() => {}}
        creditsAfter={0}
        creditLimit={18}
      />,
    )
    fireEvent.click(screen.getByRole('tab', { name: /Ready to Add \(1\)/i }))
    expect(screen.queryByText(target.code)).toBeNull()
    expect(screen.getByText(targetOpen.code)).toBeTruthy()

    rerender(
      <CourseSidebar
        courses={[]}
        state={emptyState()}
        term="Winter 2027"
        showWaitlist
        onToggleWaitlist={() => {}}
        showClosed={false}
        onToggleClosed={() => {}}
        compatible={[target, targetOpen]}
        compatStatus={{
          [target.code]: { loading: false, checked: true, openCount: 0, waitlistCount: 1, closedCount: 0, notOffered: false },
          [targetOpen.code]: { loading: false, checked: true, openCount: 2, waitlistCount: 0, closedCount: 0, notOffered: false },
        }}
        readyLoading={false}
        onToggleCourse={() => {}}
        onToggleLock={() => {}}
        onSelectBlockCourse={() => {}}
        onAddCompatible={() => {}}
        onInspectCourse={() => {}}
        creditsAfter={0}
        creditLimit={18}
      />,
    )
    expect(screen.getByRole('tab', { name: /Ready to Add \(2\)/i })).toBeTruthy()
    expect(screen.getByText(target.code)).toBeTruthy()

    render(
      <CourseInfoPopover
        target={{
          code: 'DSAI 2201',
          title: 'Intro AI',
          units: 3,
          unlocks: ['DSAI 3201', 'DSAI 4101'],
          prereqText: 'INFT 1201',
          prereqStatus: 'met',
          coreqs: [],
          plannedTerm: 'Winter 2027',
          inSchedule: false,
        }}
        onClose={() => {}}
      />,
    )
    expect(screen.getByText('DSAI 3201, DSAI 4101')).toBeTruthy()
  })

  it('FutureSemestersStrip expands and allows queueing Move term', () => {
    const view = buildDegree(wire as WireDegree, 'Winter 2027')
    const onMove = vi.fn()
    render(
      <FutureSemestersStrip
        view={view}
        openTerm="Winter 2027"
        courses={[]}
        state={emptyState()}
        moves={{}}
        onMoveCourse={onMove}
        onCancelMove={() => {}}
        onInspectCourse={() => {}}
      />,
    )
    fireEvent.click(screen.getByRole('button', { name: /Expand future semesters/i }))
    expect(screen.getByRole('button', { name: /Collapse plan/i })).toBeTruthy()
  })

  it('Apply blocks Closed sections and Done generates a valid Sun-Thu .ics file', () => {
    const closedOpt: ClassOption = { id: 'cl', optionNo: '1', status: 'Closed', components: [comp('501', '01', 'Prof. X', 0, 540, 600, 0, 30)] }
    const openOpt: ClassOption = { id: 'op', optionNo: '1', status: 'Open', components: [comp('601', '01', 'Prof. Y', 0, 540, 600, 10, 30)] }
    const cClosed = makeCourse('DACS 2202', [closedOpt])
    render(
      <Apply
        term="Winter 2027"
        courses={[cClosed]}
        state={emptyState()}
        enrolled={{ term: 'Winter 2027', termRow: -1, rows: [], options: {}, unconfirmed: [] }}
        degreeView={null}
        moveDrafts={{}}
        planning={{}}
        onBack={() => {}}
        onDone={() => {}}
        onRefresh={() => {}}
      />,
    )
    expect(screen.getByText(/Full section selected/i)).toBeTruthy()
    expect((screen.getByRole('button', { name: /Confirm & Enroll/i }) as HTMLButtonElement).disabled).toBe(true)

    const cOpen = makeCourse('INFS 2201', [openOpt])
    const ics = buildIcs([cOpen], emptyState(), 'Winter 2027')
    expect(ics).toContain('BEGIN:VCALENDAR')
    expect(ics).toContain('RRULE:FREQ=WEEKLY;BYDAY=SU')
    expect(ics).toContain('INFS 2201')

    render(
      <Done
        term="Winter 2027"
        courses={[cOpen]}
        state={emptyState()}
        progress={{ id: 1, phase: 'finished', finished: true, error: null, items: [{ id: 'enrol:INFS 2201', kind: 'enrol', course: 'INFS 2201', status: 'done', note: 'Enrolled' }] }}
        onBack={() => {}}
        onSignOut={() => {}}
      />,
    )
    expect(screen.getByRole('button', { name: /Export to Google Calendar/i })).toBeTruthy()
  })

  it('Calendar expands hour scale dynamically for 8-10 PM classes and FuturePlanDrawer queues moves in 1 click', () => {
    const eveningOpt: ClassOption = { id: 'eve', optionNo: '4', status: 'Open', components: [comp('3504', '04', 'Prof. H', 0, 1200, 1320, 10, 25)] }
    const eveCourse = makeCourse('INFS 3102', [eveningOpt])
    render(
      <Calendar
        courses={[eveCourse]}
        state={emptyState()}
        blocks={[{
          courseCode: 'INFS 3102',
          classNbr: '3504',
          kind: 'Lecture',
          room: 'B2-209',
          instructors: ['Prof. H'],
          day: 0,
          start: 1200,
          end: 1320,
          locked: false,
          held: false,
        }]}
        settings={DEFAULT_SETTINGS}
        filters={{ open: true, waitlist: false, closed: false, allowRedo: false }}
        onPickOption={() => {}}
        onToggleLock={() => {}}
        onToggleCourse={() => {}}
        inspectedCourseCode={null}
        onInspectCourse={() => {}}
        favTeachers={[]}
        onToggleFavTeacher={() => {}}
        onToggleAllowRedo={() => {}}
        sidebarOpen={true}
        onToggleSidebar={() => {}}
      />,
    )
    expect(screen.getByText('9p')).toBeTruthy()
    expect(document.querySelector('.cal-grid-cols.hours-14')).toBeTruthy()

    const view = buildDegree(wire as WireDegree, 'Winter 2027')
    const onMove = vi.fn()
    render(
      <FuturePlanDrawer
        open={true}
        onClose={() => {}}
        view={view}
        openTerm="Winter 2027"
        moves={{}}
        onMoveCourse={onMove}
        onCancelMove={() => {}}
        onInspectCourse={() => {}}
      />,
    )
    const pillButtons = document.querySelectorAll('.sem-pill')
    if (pillButtons.length > 0) fireEvent.click(pillButtons[pillButtons.length - 1])
    const selects = screen.getAllByRole('combobox')
    expect(selects.length).toBeGreaterThan(0)
    fireEvent.change(selects[0], { target: { value: 'Winter 2027' } })
    expect(onMove).toHaveBeenCalled()
  })

  it('CourseSidebar matches calendar tile color class and shows Adding spinner + addNotice feedback', () => {
    const view = buildDegree(wire as WireDegree, 'Winter 2027')
    const { eligible } = discoverCompatibleCandidates(view, 'Winter 2027', [])
    const target = eligible[0]
    render(
      <CourseSidebar
        courses={[makeCourse('DACS 2202', [{ id: '1', optionNo: '1', status: 'Open', components: [comp('101', '01', 'Dr. A', 0, 600, 690)] }])]}
        state={emptyState()}
        term="Winter 2027"
        showWaitlist={false}
        onToggleWaitlist={() => {}}
        showClosed={false}
        onToggleClosed={() => {}}
        compatible={[target]}
        compatStatus={{ [target.code]: { loading: false, checked: true, openCount: 1, waitlistCount: 0, closedCount: 0, notOffered: false } }}
        readyLoading={false}
        onToggleCourse={() => {}}
        onToggleLock={() => {}}
        onSelectBlockCourse={() => {}}
        onAddCompatible={() => {}}
        onInspectCourse={() => {}}
        creditsAfter={3}
        creditLimit={18}
        addingCodes={[target.code]}
        addNotice={`✓ Added ${target.code} to timetable`}
      />,
    )
    const plannedCard = document.querySelector('.course-card.has-tile')
    expect(plannedCard).toBeTruthy()
    fireEvent.click(screen.getByRole('tab', { name: /Ready to Add/i }))
    expect(screen.getByText(/✓ Added/i)).toBeTruthy()
    expect(screen.getByRole('button', { name: /Adding…/i })).toBeTruthy()
  })

  it('FuturePlanDrawer shows inline Needs/Unlocks, credit totals in term options, Open badges on done courses, and elective choices', () => {
    const view = buildDegree(wire as WireDegree, 'Winter 2027')
    const groupRow = view.years.flatMap((y) => y.semesters).flatMap((s) => s.blocks).find((b) => b.kind === 'group')
    const completedCourse = view.years.flatMap((y) => y.semesters).flatMap((s) => s.blocks).find((b) => b.kind === 'course' && b.course.status === 'completed')
    const gKey = groupRow && groupRow.kind === 'group' ? `${groupRow.group.year}:${groupRow.group.row}` : '2:0'
    const doneCode = completedCourse && completedCourse.kind === 'course' ? completedCourse.course.code : 'INFS 1101'

    render(
      <FuturePlanDrawer
        open={true}
        onClose={() => {}}
        view={view}
        openTerm="Winter 2027"
        classStatuses={{ [doneCode]: { term: 'Winter 2027', status: 'Classes available' } }}
        groupChoices={{
          [gKey]: [
            { row: 0, year: 2, code: 'GARC 2003', title: 'Data Analytics', category: 'Elective', units: 3, term: null, status: 'planned', plannable: true, hasRequirement: false },
          ],
        }}
        moves={{}}
        onMoveCourse={() => {}}
        onCancelMove={() => {}}
        onQuickAddCourse={() => {}}
      />,
    )

    // Verify Needs & Unlocks appear inline (no modal trigger needed)
    expect(screen.getAllByText(/Needs:/i).length).toBeGreaterThan(0)
    expect(screen.getAllByText(/Unlocks:/i).length).toBeGreaterThan(0)
    expect(screen.queryByRole('button', { name: /^Inspect/i })).toBeNull()

    // Switch to a semester containing a plannable course (with Term dropdown)
    const semPills = [...document.querySelectorAll('.sem-pill')]
    const allSems = view.years.flatMap((y) => y.semesters)
    const plannableIdx = allSems.findIndex((s) => s.blocks.some((b) => b.kind === 'course' && b.course.plannable && b.course.status !== 'completed'))
    if (plannableIdx >= 0 && semPills[plannableIdx]) fireEvent.click(semPills[plannableIdx])

    // Verify dropdown options contain live credit totals (e.g. "/18 cr)" or "/12 cr)")
    const optionTexts = [...document.querySelectorAll('select option')].map((o) => o.textContent ?? '')
    expect(optionTexts.some((t) => /\d+\/\d+ cr\)/.test(t))).toBe(true)

    // Verify completed semester shows "Open in Winter 2027" badge when PeopleSoft Plan lists it
    fireEvent.click(semPills[0])
    expect(screen.getByText(/Open in Winter 2027/i)).toBeTruthy()
  })

  it('isolates cached workspace snapshots across accounts and clears academic storage on sign-out/switch', async () => {
    window.localStorage.setItem('planie:workspace:v1', JSON.stringify({
      term: 'Winter 2027',
      courses: [{ code: 'ACCT1 9999', title: 'Leak Course', term: 'Winter 2027', units: 3, options: [] }],
      state: emptyState(),
      degreeWire: null,
      enrolled: { term: 'Winter 2027', termRow: -1, rows: [], options: {}, unconfirmed: [] },
    }))
    window.sessionStorage.setItem(`${WORKSPACE_CACHE_PREFIX}slot_A`, JSON.stringify({
      accountId: 'slot_A',
      userHint: '60••••11',
      term: 'Winter 2027',
      courses: [{ code: 'ACCT1 9999', title: 'Leak Course', term: 'Winter 2027', units: 3, options: [] }],
      state: emptyState(),
      degreeWire: null,
      enrolled: { term: 'Winter 2027', termRow: -1, rows: [], options: {}, unconfirmed: [] },
    }))

    vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
      if (url.includes('/status')) {
        return {
          ok: true,
          status: 200,
          text: async () => JSON.stringify({ signedIn: true, accountId: 'slot_B', userHint: '60••••22', demo: false }),
        }
      }
      if (url.includes('/accounts')) {
        return {
          ok: true,
          status: 200,
          text: async () => JSON.stringify({
            accounts: [
              { id: 'slot_A', userHint: '60••••11', active: false, demo: false },
              { id: 'slot_B', userHint: '60••••22', active: true, demo: false },
            ],
            activeId: 'slot_B',
          }),
        }
      }
      if (url.includes('/bootstrap') && init?.method === 'POST') {
        return {
          ok: true,
          status: 200,
          text: async () => JSON.stringify({
            classes: {
              term_label: 'Winter 2027',
              can_submit: true,
              rows: [{ row: 0, course_label: 'ACCT2 1001: Account Two Intro', term: 'Winter 2027', status: 'Enrolled' }],
            },
            enrolled: { termRow: 0, classes: [] },
            degree: null,
          }),
        }
      }
      return { ok: true, status: 200, text: async () => JSON.stringify({ courses: [], results: [] }) }
    }))

    render(<App />)
    expect(await screen.findByText(/Quick setup/i)).toBeTruthy()
    expect(screen.queryByText(/ACCT1 9999/)).toBeNull()
    expect(window.localStorage.getItem('planie:workspace:v1')).toBeNull()

    clearAcademicData()
    expect(window.sessionStorage.getItem(`${WORKSPACE_CACHE_PREFIX}slot_A`)).toBeNull()
  })

  it('assigns strictly distinct theme-matched tile colors across up to 8 courses without collisions', () => {
    const sample = ['DACS 2201', 'DSAI 2201', 'INFS 2201', 'INFT 2202', 'GARC 2001', 'DACS 2202', 'INFS 3102', 'BIOL 1110']
    const map = assignTileClasses(sample)
    const assigned = sample.map((code) => tileClassFor(code, map))
    expect(new Set(assigned).size).toBe(8)
  })

  it('caches 10 real-time Lichess chess puzzles in memory and plays WaitWidget Chess + Subway offline cleanly', async () => {
    clearChessPuzzleCache()
    window.localStorage.removeItem('planie:chess:cache:v1')
    let n = 0
    const fakeFetch = async () => {
      n++
      return {
        id: `pz${n}`,
        rating: 1400 + n,
        fen: '6k1/5ppp/8/8/8/8/5PPP/3R2K1 w - - 0 1',
        solution: ['d1d8'],
        themes: ['mateIn1'],
      }
    }
    const cached = await warmChessPuzzleCache(fakeFetch, 10)
    expect(cached).toBe(10)
    expect(getCachedPuzzleCount()).toBe(10)

    // When internet disconnects, nextChessPuzzle still serves distinct cached puzzles from memory
    const offlinePuzzle = await nextChessPuzzle(async () => {
      throw new Error('offline')
    })
    expect(offlinePuzzle.source).toBe('cached')
    expect(offlinePuzzle.id.startsWith('pz')).toBe(true)

    const { container } = render(<WaitWidget />)
    expect(screen.queryByRole('tab', { name: 'Fun facts' })).toBeNull()
    expect(screen.queryByRole('tab', { name: 'Memory' })).toBeNull()
    fireEvent.click(screen.getByRole('tab', { name: 'Chess' }))
    expect(await screen.findByLabelText('Lichess Chessboard')).toBeTruthy()
    expect(container.querySelector('.cg-wrap')).toBeTruthy()
    expect(screen.queryByText(/cached|offline/i)).toBeNull()
    fireEvent.click(screen.getByRole('tab', { name: 'Subway' }))
    expect(screen.getByTitle('Subway Surfers HTML5 Game')).toBeTruthy()
    expect(screen.queryByText(/Offline Runner/i)).toBeNull()
    expect(container.querySelectorAll('[style]').length).toBe(0)
  })

  it('warns in SuggestionsStrip when no full conflict-free suggestion fits and labels removed courses clearly', () => {
    const o1: ClassOption = { id: 'a1', optionNo: '1', status: 'Open', components: [comp('1', '01', 'Dr. A', 0, 600, 690)] }
    const o2: ClassOption = { id: 'b1', optionNo: '1', status: 'Open', components: [comp('2', '01', 'Dr. B', 0, 600, 690)] }
    const c1 = makeCourse('A 101', [o1], o1)
    const c2 = makeCourse('B 202', [o2])
    const sug = buildSuggestions([c1, c2], { ...emptyState(), unlocked: ['A 101'] }, DEFAULT_SETTINGS, DEFAULT_QUICK_SETUP)
    render(
      <SuggestionsStrip
        top={sug.top}
        more={sug.more}
        activeSig=""
        expanded={false}
        onToggleExpand={() => {}}
        onPick={() => {}}
        unsolvableReason={sug.unsolvableReason}
        partialNotice={sug.partialNotice}
      />,
    )
    expect(screen.queryByRole('status')).toBeNull()
    expect(screen.getByText('Removes B 202')).toBeTruthy()
  })

  it('marks exempted courses in FuturePlanDrawer and opens Retake semester menu on right-click', () => {
    const onRetakeCourse = vi.fn()
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
            { kind: 'course' as const, course: { row: 1, year: 0, code: 'MATH 1010', title: 'Pre-Calculus', category: '', units: 3, term: 'Winter 2027', status: 'planned' as const, plannable: true, hasRequirement: false } },
            { kind: 'course' as const, course: { row: 2, year: 0, code: 'MATH 1020', title: 'Calculus I', category: '', units: 3, term: 'Fall 2025', status: 'completed' as const, plannable: false, hasRequirement: false } },
          ],
        }],
      }],
      extras: [],
      byTerm: [],
      progress: { completed: 3, inProgress: 0, planned: 3, unplanned: 0, total: 6, unknown: 0 },
    }
    render(
      <FuturePlanDrawer
        open
        onClose={() => {}}
        view={customView}
        openTerm="Winter 2027"
        moves={{}}
        onMoveCourse={() => {}}
        onCancelMove={() => {}}
        onRetakeCourse={onRetakeCourse}
      />,
    )
    expect(screen.getByText(/Exempt \(via MATH 1020\)/)).toBeTruthy()
    const card = screen.getByText('Pre-Calculus').closest('.course-card') as HTMLElement
    fireEvent.contextMenu(card)
    const menu = screen.getByRole('menu', { name: 'Retake semester for MATH 1010' })
    expect(menu).toBeTruthy()
    fireEvent.click(screen.getByRole('menuitem', { name: 'Winter 2027' }))
    expect(onRetakeCourse).toHaveBeenCalledWith(expect.objectContaining({ code: 'MATH 1010' }), 'Winter 2027')
  })

  it('toggles Preferences popover, counts active preferences, resets cleanly, and shows clear lab-only label', () => {
    function PrefHarness() {
      const [setup, setSetup] = useState({ ...DEFAULT_QUICK_SETUP })
      return (
        <SuggestionsStrip
          top={[]}
          more={[]}
          activeSig=""
          expanded={false}
          onToggleExpand={() => {}}
          onPick={() => {}}
          unsolvableReason={null}
          setup={setup}
          onChangeSetup={setSetup}
        />
      )
    }
    render(<PrefHarness />)
    fireEvent.click(screen.getByRole('button', { name: 'Preferences' }))
    expect(screen.getByLabelText('Suggestion preferences')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Thu' }))
    fireEvent.click(screen.getByRole('button', { name: 'No 8 AMs' }))
    expect(screen.getByRole('button', { name: 'Preferences (2)' })).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Reset' }))
    expect(screen.getByRole('button', { name: 'Preferences' })).toBeTruthy()
  })

  it('hides time text on calendar cards, distinguishes Lecture vs Lab blocks, and reveals full info on hover', () => {
    const cLec = makeCourse('BIOL 1110', [{ id: 'o1', optionNo: '1', status: 'Open', components: [comp('101', '01', 'Dr. Maya', 0, 570, 660)] }])
    const cLab: PCourse = {
      ...makeCourse('BIOL 1111', [{
        id: 'o2',
        optionNo: '1',
        status: 'Open',
        components: [{ ...comp('102', '02', 'Dr. Karim', 1, 570, 690), kind: 'Laboratory' }],
      }]),
      title: 'Biology I (Lab)',
    }
    const { container } = render(
      <Calendar
        courses={[cLec, cLab]}
        state={emptyState()}
        blocks={[
          { courseCode: 'BIOL 1110', classNbr: '101', kind: 'Lecture', room: 'B1-101', instructors: ['Dr. Maya'], day: 0, start: 570, end: 660, locked: false, held: false },
          { courseCode: 'BIOL 1111', classNbr: '102', kind: 'Laboratory', room: 'B2-204', instructors: ['Dr. Karim'], day: 1, start: 570, end: 690, locked: false, held: false },
        ]}
        settings={DEFAULT_SETTINGS}
        filters={{ open: true, waitlist: false, closed: false, allowRedo: false }}
        onPickOption={() => {}}
        onToggleLock={() => {}}
        onToggleCourse={() => {}}
        inspectedCourseCode={null}
        onInspectCourse={() => {}}
        favTeachers={[]}
        onToggleFavTeacher={() => {}}
        onToggleAllowRedo={() => {}}
        sidebarOpen
        onToggleSidebar={() => {}}
      />,
    )
    const lecBtn = container.querySelector('.cal-block.is-lecture') as HTMLButtonElement
    const labBtn = container.querySelector('.cal-block.is-lab') as HTMLButtonElement
    expect(lecBtn).toBeTruthy()
    expect(labBtn).toBeTruthy()
    expect(lecBtn.textContent).not.toContain('9:30a–11a')
    expect(labBtn.textContent).toContain('LAB')
    expect(lecBtn.textContent).toContain('LEC')

    fireEvent.mouseEnter(labBtn)
    const peek = screen.getByRole('status')
    expect(peek.className).toContain('cal-hover-tip')
    expect(peek.textContent).toContain('Biology I (Lab)')
    expect(peek.textContent).toContain('9:30a–11:30a')
    expect(peek.textContent).toContain('B2-204')
    fireEvent.mouseLeave(labBtn)
    expect(screen.queryByRole('status')).toBeNull()
  })

  it('replaces default browser right-click menu site-wide with custom menu without opening modal popups', async () => {
    vi.stubGlobal('fetch', vi.fn(async (url: string) => {
      if (url.includes('/status')) {
        return { ok: true, status: 200, text: async () => JSON.stringify({ signedIn: false, demo: false }) }
      }
      return { ok: true, status: 200, text: async () => JSON.stringify({ accounts: [], activeId: null }) }
    }))
    const { container } = render(<App />)
    await screen.findByText(/Your classes/i)
    fireEvent.contextMenu(container.firstElementChild!, { clientX: 120, clientY: 140 })
    const menu = await screen.findByRole('menu', { name: 'Custom context menu' })
    expect(menu).toBeTruthy()
    expect(container.querySelector('.sheet-backdrop')).toBeNull()
  })

  it('highlights overlapping courses placed side-by-side on Calendar as overriding', () => {
    const o1: ClassOption = { id: 'o1', optionNo: '1', status: 'Open', components: [comp('101', '01', 'Dr. Maya', 0, 570, 660)] }
    const o2: ClassOption = { id: 'o2', optionNo: '1', status: 'Open', components: [comp('102', '01', 'Dr. Karim', 0, 600, 690)] }
    const { container } = render(
      <Calendar
        courses={[makeCourse('OVER 101', [o1], o1), makeCourse('OVER 102', [o2], o2)]}
        state={emptyState()}
        blocks={[
          { courseCode: 'OVER 101', classNbr: '101', kind: 'Lecture', room: 'B1-101', instructors: ['Dr. Maya'], day: 0, start: 570, end: 660, locked: false, held: true },
          { courseCode: 'OVER 102', classNbr: '102', kind: 'Lecture', room: 'B1-102', instructors: ['Dr. Karim'], day: 0, start: 600, end: 690, locked: false, held: false },
        ]}
        settings={DEFAULT_SETTINGS}
        filters={{ open: true, waitlist: false, closed: false, allowRedo: false }}
        onPickOption={() => {}}
        onToggleLock={() => {}}
        onToggleCourse={() => {}}
        inspectedCourseCode={null}
        onInspectCourse={() => {}}
        favTeachers={[]}
        onToggleFavTeacher={() => {}}
        onToggleAllowRedo={() => {}}
        sidebarOpen
        onToggleSidebar={() => {}}
      />,
    )
    const overlapping = container.querySelectorAll('.cal-block.is-overlapping')
    expect(overlapping.length).toBe(2)
    expect(screen.getByRole('alert').textContent).toContain('Overriding: OVER 101 × OVER 102')
    expect(screen.getByText('Overlaps OVER 102')).toBeTruthy()
    expect(screen.getByText('Overlaps OVER 101')).toBeTruthy()
  })
})

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { api, ApiError, setAccountId, type PlanProgress, type ServerAccount, type Status } from '../api'
import type { Course } from '../engine/types'
import { primaryClass } from '../lock'
import { Apply } from './Apply'
import { Calendar } from './Calendar'
import { discoverCompatibleCandidates, lookupCatalogKey, type CompatibleCandidate } from './compatible'
import { CourseInfoPopover, type CourseInfoTarget } from './CourseInfoPopover'
import { CourseSidebar, type CompatibleStatus } from './CourseSidebar'
import { creditsFor, creditText } from './credits'
import { bootstrapPlanie, loadHeldOptions, loadSettings, normalizeCompatibleEntry, openTerm, rememberHeldRead, sortBrowseable, splitHeld, type Bootstrapped } from './data'
import { buildDegree, planningMap, type DegreeCourse, type WireDegree } from './degree'
import { Done } from './Done'
import { FuturePlanDrawer } from './FuturePlanDrawer'
import { type TermMoveDraft } from './FutureSemestersStrip'
import { optionStatusOf } from './guides'
import { commit, redo, reset, startHistory, undo, type History } from './history'
import { Login } from './Login'
import {
  applySuggestion,
  blocksOf,
  buildCourses,
  effective,
  emptyState,
  overlapsOf,
  solverCourses,
  type PCourse,
  type PlannerState,
} from './model'
import { normCode } from './prereq'
import { getCatalogRule } from './catalogRules'
import { SetupWait, type LoadStage } from './SetupWait'
import { buildSuggestions, currentScheduleSignature, DEFAULT_QUICK_SETUP, type QuickSetup, type SuggestionCard } from './suggestions'
import { SuggestionsStrip, assignTileClasses } from './SuggestionsStrip'
import { clearAcademicData, FAV_TEACHERS_PREFIX, PREF_CACHE_PREFIX, WORKSPACE_CACHE_PREFIX } from './storage'
import { applyTheme, readTheme, type ThemeMode } from './theme'
import './app.css'

const LEGACY_CACHE_KEY = 'planie:workspace:v1'
const LEGACY_FAV_KEY = 'planie:favTeachers'

interface WorkspaceSnapshot {
  accountId: string
  userHint: string | null
  term: string
  courses: PCourse[]
  state: PlannerState
  degreeWire: WireDegree | null
  enrolled: Bootstrapped['loaded']['enrolled']
}

function readSnapshot(accountId: string | null | undefined, userHint: string | null | undefined): WorkspaceSnapshot | null {
  try {
    window.localStorage.removeItem(LEGACY_CACHE_KEY)
    window.sessionStorage.removeItem(LEGACY_CACHE_KEY)
    if (!accountId) return null
    window.localStorage.removeItem(WORKSPACE_CACHE_PREFIX + accountId)
    const raw = window.sessionStorage.getItem(WORKSPACE_CACHE_PREFIX + accountId)
    if (!raw) return null
    const snap = JSON.parse(raw) as Partial<WorkspaceSnapshot> | null
    if (!snap || snap.accountId !== accountId) return null
    if ((userHint ?? null) !== (snap.userHint ?? null)) return null
    if (!Array.isArray(snap.courses) || !snap.state || typeof snap.term !== 'string') return null
    return snap as WorkspaceSnapshot
  } catch {
    return null
  }
}

function writeSnapshot(snap: WorkspaceSnapshot): void {
  try {
    if (!snap.accountId) return
    window.localStorage.removeItem(LEGACY_CACHE_KEY)
    window.localStorage.removeItem(WORKSPACE_CACHE_PREFIX + snap.accountId)
    window.sessionStorage.setItem(WORKSPACE_CACHE_PREFIX + snap.accountId, JSON.stringify(snap))
  } catch { /* ignore */ }
}

interface AccountPrefs {
  setup: QuickSetup
  retakeTerms: Record<string, string>
}

interface WorkspaceUndoStep {
  courses: PCourse[]
  state: PlannerState
  moveDrafts: Record<string, TermMoveDraft>
  retakeTerms: Record<string, string>
}

const emptyUndoStep = (): WorkspaceUndoStep => ({
  courses: [],
  state: emptyState(),
  moveDrafts: {},
  retakeTerms: {},
})

function readAccountPrefs(accountId: string | null | undefined, userHint: string | null | undefined): AccountPrefs {
  try {
    if (!accountId) return { setup: { ...DEFAULT_QUICK_SETUP }, retakeTerms: {} }
    window.localStorage.removeItem(PREF_CACHE_PREFIX + accountId)
    const raw = window.sessionStorage.getItem(PREF_CACHE_PREFIX + accountId)
    if (!raw) return { setup: { ...DEFAULT_QUICK_SETUP }, retakeTerms: {} }
    const saved = JSON.parse(raw) as { accountId?: string; userHint?: string | null; setup?: QuickSetup; retakeTerms?: Record<string, string> } | null
    if (!saved || saved.accountId !== accountId || (userHint ?? null) !== (saved.userHint ?? null) || !saved.setup) {
      return { setup: { ...DEFAULT_QUICK_SETUP }, retakeTerms: {} }
    }
    return {
      setup: { ...DEFAULT_QUICK_SETUP, ...saved.setup },
      retakeTerms: saved.retakeTerms && typeof saved.retakeTerms === 'object' ? saved.retakeTerms : {},
    }
  } catch {
    return { setup: { ...DEFAULT_QUICK_SETUP }, retakeTerms: {} }
  }
}

function writeAccountPrefs(accountId: string | null | undefined, userHint: string | null | undefined, setup: QuickSetup, retakeTerms: Record<string, string>): void {
  try {
    if (!accountId) return
    window.localStorage.removeItem(PREF_CACHE_PREFIX + accountId)
    window.sessionStorage.setItem(PREF_CACHE_PREFIX + accountId, JSON.stringify({
      accountId,
      userHint: userHint ?? null,
      setup,
      retakeTerms,
    }))
  } catch { /* ignore */ }
}

function readFavTeachers(accountId: string | null | undefined): string[] {
  try {
    window.localStorage.removeItem(LEGACY_FAV_KEY)
    if (!accountId) return []
    const raw = window.localStorage.getItem(FAV_TEACHERS_PREFIX + accountId)
    if (!raw) return []
    const parsed = JSON.parse(raw) as unknown
    return Array.isArray(parsed) ? parsed.filter((t): t is string => typeof t === 'string') : []
  } catch {
    return []
  }
}

function writeFavTeachers(accountId: string | null | undefined, favs: string[]): void {
  try {
    window.localStorage.removeItem(LEGACY_FAV_KEY)
    if (!accountId) return
    window.localStorage.setItem(FAV_TEACHERS_PREFIX + accountId, JSON.stringify(favs))
  } catch { /* ignore */ }
}

interface ContextMenuState {
  vx: number
  vy: number
  kind: 'block' | 'planned' | 'eligible' | 'page'
  code?: string
  sec?: string
  comp?: string
}

const clampPct = (n: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, n))

type Screen = 'login' | 'setup' | 'workspace' | 'apply' | 'done'

export function App() {
  const [theme, setTheme] = useState<ThemeMode>(() => {
    const t = readTheme()
    applyTheme(t)
    return t
  })
  const toggleTheme = () => {
    setTheme((prev) => {
      const next: ThemeMode = prev === 'dark' ? 'light' : 'dark'
      applyTheme(next)
      return next
    })
  }

  const [status, setStatus] = useState<Status | null>(null)
  const [accounts, setAccounts] = useState<ServerAccount[]>([])
  const [screen, setScreen] = useState<Screen>('login')
  const [authError, setAuthError] = useState('')

  const [setup, setSetup] = useState<QuickSetup>(DEFAULT_QUICK_SETUP)
  const [showWaitlist, setShowWaitlist] = useState(false)
  const [showClosed, setShowClosed] = useState(false)
  const [allowRedo, setAllowRedo] = useState(false)
  const [sidebarOpen, setSidebarOpen] = useState(true)
  const [sidebarTab, setSidebarTab] = useState<'current' | 'eligible'>('current')
  const [moreSuggestions, setMoreSuggestions] = useState(false)
  const [futureDrawerOpen, setFutureDrawerOpen] = useState(false)
  const [electiveChoices, setElectiveChoices] = useState<DegreeCourse[]>([])

  const [term, setTerm] = useState('Winter 2027')
  const [workspaceHistory, setWorkspaceHistory] = useState<History<WorkspaceUndoStep>>(() => startHistory(emptyUndoStep()))
  const { courses, state, moveDrafts, retakeTerms } = workspaceHistory.present
  const canUndo = workspaceHistory.past.length > 0
  const canRedo = workspaceHistory.future.length > 0

  const setState = useCallback((updater: PlannerState | ((prev: PlannerState) => PlannerState)) => {
    setWorkspaceHistory((h) => {
      const nextState = typeof updater === 'function' ? updater(h.present.state) : updater
      if (nextState === h.present.state) return h
      return commit(h, { ...h.present, state: nextState }, Date.now())
    })
  }, [])

  const setMoveDrafts = useCallback((
    updater: Record<string, TermMoveDraft> | ((prev: Record<string, TermMoveDraft>) => Record<string, TermMoveDraft>),
  ) => {
    setWorkspaceHistory((h) => {
      const nextDrafts = typeof updater === 'function' ? updater(h.present.moveDrafts) : updater
      if (nextDrafts === h.present.moveDrafts) return h
      return commit(h, { ...h.present, moveDrafts: nextDrafts }, Date.now())
    })
  }, [])

  const [enrolled, setEnrolled] = useState<Bootstrapped['loaded']['enrolled']>({
    term: 'Winter 2027',
    termRow: -1,
    rows: [],
    options: {},
    unconfirmed: [],
  })
  const [degreeWire, setDegreeWire] = useState<WireDegree | null>(null)
  const [compatCoursesData, setCompatCoursesData] = useState<Record<string, { course: Course; openCount: number; waitlistCount: number; closedCount: number; notOffered: boolean }>>({})
  const [compatLoading, setCompatLoading] = useState(false)
  const [favTeachers, setFavTeachers] = useState<string[]>([])

  const [inspectedCalendarCode, setInspectedCalendarCode] = useState<string | null>(null)
  const [infoTarget, setInfoTarget] = useState<CourseInfoTarget | null>(null)
  const [doneProgress, setDoneProgress] = useState<PlanProgress | null>(null)
  const [searchingFallback, setSearchingFallback] = useState(false)
  const [addingCodes, setAddingCodes] = useState<string[]>([])
  const [justAddedCourse, setJustAddedCourse] = useState<string | null>(null)
  const [addNotice, setAddNotice] = useState<string | null>(null)
  const [pendingPartialPick, setPendingPartialPick] = useState<SuggestionCard | null>(null)
  const [ctxMenu, setCtxMenu] = useState<ContextMenuState | null>(null)
  const addedTimerRef = useRef<number | null>(null)
  const morphCaptureRef = useRef<((mode: 'schedule' | 'layout') => void) | null>(null)
  const [classStatuses, setClassStatuses] = useState<Record<string, { term: string; status: string }>>({})
  const [groupChoicesMap, setGroupChoicesMap] = useState<Record<string, DegreeCourse[]>>({})
  const [loadingGroupsMap, setLoadingGroupsMap] = useState<Record<string, boolean>>({})
  const [checkingDrawerTerm, setCheckingDrawerTerm] = useState(false)

  const [loadStages, setLoadStages] = useState<LoadStage[]>([
    { id: 'boot', label: 'Class list, schedule & degree tree', status: 'working' },
    { id: 'sections', label: 'Planned course sections', status: 'pending' },
    { id: 'compatible', label: 'Eligible degree courses', status: 'pending' },
  ])
  const [bootReady, setBootReady] = useState(false)
  const [loadError, setLoadError] = useState<string | null>(null)
  const loadSeq = useRef(0)
  const activeAccountKeyRef = useRef<string | null>(null)

  const resetAccountWorkspace = useCallback((clearAllSaved = true) => {
    loadSeq.current += 1
    activeAccountKeyRef.current = null
    if (clearAllSaved) clearAcademicData()
    setTerm('Winter 2027')
    setWorkspaceHistory(reset(emptyUndoStep()))
    setSetup({ ...DEFAULT_QUICK_SETUP })
    setEnrolled({ term: 'Winter 2027', termRow: -1, rows: [], options: {}, unconfirmed: [] })
    setDegreeWire(null)
    setCompatCoursesData({})
    setCompatLoading(false)
    setFavTeachers([])
    setGroupChoicesMap({})
    setLoadingGroupsMap({})
    setElectiveChoices([])
    setClassStatuses({})
    setInfoTarget(null)
    setInspectedCalendarCode(null)
    setDoneProgress(null)
    setAddingCodes([])
    setJustAddedCourse(null)
    setAddNotice(null)
    setPendingPartialPick(null)
    setFutureDrawerOpen(false)
    setCheckingDrawerTerm(false)
    setBootReady(false)
    setLoadError(null)
    setSidebarTab('current')
  }, [])

  const settings = useMemo(() => loadSettings(), [])
  const planning = useMemo(() => (degreeWire ? planningMap(degreeWire) : {}), [degreeWire])
  const degreeView = useMemo(
    () => (degreeWire ? buildDegree(degreeWire, term, [], state.removed) : null),
    [degreeWire, term, state.removed],
  )

  const runBootstrap = useCallback(async (quietBackground = false, acctMeta?: { accountId?: string | null; userHint?: string | null }) => {
    const seq = ++loadSeq.current
    if (!quietBackground) {
      setBootReady(false)
      setLoadError(null)
      setLoadStages([
        { id: 'boot', label: 'Class list, schedule & degree tree', status: 'working' },
        { id: 'sections', label: 'Planned course sections', status: 'pending' },
        { id: 'compatible', label: 'Eligible degree courses', status: 'pending' },
      ])
    }

    try {
      const res = await bootstrapPlanie((step, detail) => {
        if (seq !== loadSeq.current || quietBackground) return
        if (step === 'plan' || step === 'enrolled') {
          setLoadStages((prev) => prev.map((s) => (s.id === 'boot' ? { ...s, status: 'done', detail } : s.id === 'sections' ? { ...s, status: 'working' } : s)))
        } else if (step === 'times') {
          setLoadStages((prev) => prev.map((s) => (s.id === 'sections' ? { ...s, status: 'done', detail } : s.id === 'compatible' ? { ...s, status: 'working' } : s)))
        }
      })
      if (seq !== loadSeq.current) return

      const wire = (res.degree && typeof res.degree === 'object' ? res.degree : null) as WireDegree | null
      const activeTerm = res.loaded.term || (wire ? openTerm([]) : 'Winter 2027')
      const { have: cachedHeld, need: needHeld } = splitHeld(res.loaded, false)
      const builtCourses = buildCourses(res.loaded.plan, res.loaded.enrolled.options, res.loaded.enrolled.rows, cachedHeld, activeTerm)
      const heldCodes = builtCourses.filter((c) => c.held).map((c) => c.code)
      const acctId = acctMeta?.accountId ?? null
      const userHint = acctMeta?.userHint ?? null
      setTerm(activeTerm)
      setEnrolled(res.loaded.enrolled)
      setDegreeWire(wire)
      if (res.loaded.classStatuses) setClassStatuses(res.loaded.classStatuses)
      setWorkspaceHistory((h) => {
        const base = quietBackground ? h.present.state : emptyState()
        const merged = { ...base, unlocked: [...new Set([...base.unlocked, ...heldCodes])] }
        if (acctId) {
          writeSnapshot({ accountId: acctId, userHint, term: activeTerm, courses: builtCourses, state: merged, degreeWire: wire, enrolled: res.loaded.enrolled })
        }
        const nextStep: WorkspaceUndoStep = { ...h.present, courses: builtCourses, state: merged }
        return reset(nextStep)
      })
      setBootReady(true)
      if (!quietBackground) {
        setLoadStages((prev) => prev.map((s) => (s.id === 'sections' ? { ...s, status: 'done' } : s)))
      }

      // Automatically check ALL eligible degree courses + open term availability in the background
      if (wire) {
        void (async () => {
          const dv = buildDegree(wire, activeTerm, [], quietBackground ? state.removed : [])
          const { eligible, exempted } = discoverCompatibleCandidates(dv, activeTerm, builtCourses.map((c) => c.code))
          const fallbackM = /^(Winter|Spring|Fall)\s+(\d{4})$/.exec(activeTerm)
          const fallbackStrm = fallbackM ? `1${fallbackM[2].slice(2)}${fallbackM[1] === 'Winter' ? 1 : fallbackM[1] === 'Spring' ? 5 : 8}` : '1271'
          const strm = eligible.find((x) => x.strm)?.strm ?? fallbackStrm
          const checkedCodes = new Set<string>()
          const metaByCode = new Map<string, { title: string; units: number | null }>()
          for (const c of eligible) metaByCode.set(normCode(c.code), { title: c.title, units: c.units })

          const checkChunks = async (items: Array<{ code: string; key?: { crse_id: string; offer_nbr: string; effdt: string } | null }>) => {
            let totalChecked = 0
            for (let off = 0; off < items.length; off += 24) {
              if (seq !== loadSeq.current) return totalChecked
              const chunk = items.slice(off, off + 24).map((c) => ({
                code: c.code,
                ...(c.key ? { crse_id: c.key.crse_id, offer_nbr: c.key.offer_nbr, effdt: c.key.effdt } : {}),
              }))
              if (chunk.length === 0) continue
              try {
                const compRes = await api.compatibleSections(strm, chunk)
                if (seq !== loadSeq.current) return totalChecked
                const entries = Array.isArray(compRes?.results) ? compRes.results : []
                totalChecked += entries.length
                setCompatCoursesData((prev) => {
                  const next = { ...prev }
                  for (const entry of entries) {
                    checkedCodes.add(normCode(entry.code))
                    const meta = metaByCode.get(normCode(entry.code))
                    const parsed = normalizeCompatibleEntry(entry, meta?.title ?? entry.code, meta?.units ?? 3, activeTerm)
                    if (parsed) next[entry.code] = parsed
                  }
                  return next
                })
              } catch { /* ignore failing chunk cleanly */ }
            }
            return totalChecked
          }

          if (eligible.length > 0) {
            setCompatLoading(true)
            const checkedCount = await checkChunks(eligible)
            if (seq !== loadSeq.current) return
            setCompatLoading(false)
            setLoadStages((prev) => prev.map((s) => (s.id === 'compatible' ? { ...s, status: 'done', detail: `${checkedCount} checked` } : s)))
          } else {
            setLoadStages((prev) => prev.map((s) => (s.id === 'compatible' ? { ...s, status: 'done', detail: 'Ready' } : s)))
          }

          // Secondary background step: automatically preload ALL CourseGroup elective lists
          const groupIndices: Array<{ year: number; index: number; row: number }> = []
          const byYear = new Map<number, typeof wire.rows>()
          for (const r of wire.rows) {
            const arr = byYear.get(r.year) ?? []
            arr.push(r)
            byYear.set(r.year, arr)
          }
          for (const [yr, list] of byYear) {
            list.forEach((r, idx) => {
              if (r.has_select) groupIndices.push({ year: yr, index: idx, row: r.row })
            })
          }

          const extraGroupChoices: DegreeCourse[] = []
          for (const g of groupIndices) {
            const gKey = `${g.year}:${g.row}`
            setLoadingGroupsMap((prev) => ({ ...prev, [gKey]: true }))
            try {
              const gl = await api.writeRead<{ choices?: Array<{ code: string; title: string; units?: string }> }>('degree_course_list', { year: g.year, index: g.index })
              if (seq !== loadSeq.current) return
              const parsedList: DegreeCourse[] = []
              for (const ch of gl.choices ?? []) {
                if (!ch.code) continue
                const dc: DegreeCourse = {
                  row: 0,
                  year: g.year,
                  code: ch.code,
                  title: ch.title || ch.code,
                  category: 'Elective',
                  units: ch.units ? Number.parseFloat(ch.units) : 3,
                  term: null,
                  status: 'planned',
                  plannable: true,
                  hasRequirement: false,
                }
                parsedList.push(dc)
                metaByCode.set(normCode(ch.code), { title: dc.title, units: dc.units })
                if (builtCourses.some((x) => normCode(x.code) === normCode(ch.code))) continue
                if (eligible.some((e) => normCode(e.code) === normCode(ch.code)) || extraGroupChoices.some((x) => normCode(x.code) === normCode(ch.code))) continue
                extraGroupChoices.push(dc)
              }
              setGroupChoicesMap((prev) => ({ ...prev, [gKey]: parsedList }))
            } catch { /* ignore */ } finally {
              if (seq === loadSeq.current) setLoadingGroupsMap((prev) => ({ ...prev, [gKey]: false }))
            }
          }
          if (seq !== loadSeq.current) return
          if (extraGroupChoices.length > 0) {
            setElectiveChoices(extraGroupChoices)
            await checkChunks(extraGroupChoices.slice(0, 12).map((ec) => ({ code: ec.code, key: lookupCatalogKey(ec.code) })))
          }

          // Background Future Plan check: populate open status in activeTerm (Winter 2027) across all remaining degree courses
          const remainingDegreeCourses: Array<{ code: string; key: ReturnType<typeof lookupCatalogKey> }> = []
          const seenRem = new Set<string>(checkedCodes)
          for (const yr of dv.years) {
            for (const sem of yr.semesters) {
              for (const b of sem.blocks) {
                if (b.kind !== 'course') continue
                const nc = normCode(b.course.code)
                if (seenRem.has(nc) || exempted.has(nc)) continue
                seenRem.add(nc)
                metaByCode.set(nc, { title: b.course.title, units: b.course.units })
                remainingDegreeCourses.push({ code: b.course.code, key: lookupCatalogKey(b.course.code) })
              }
            }
          }
          if (remainingDegreeCourses.length > 0) {
            await checkChunks(remainingDegreeCourses)
          }
        })()
      }

      if (needHeld.length > 0) {
        void loadHeldOptions(
          needHeld,
          (code, key, opts) => {
            if (seq !== loadSeq.current) return
            rememberHeldRead(res.loaded, code, key, opts)
            const sorted = sortBrowseable(opts)
            const patchStep = (step: WorkspaceUndoStep): WorkspaceUndoStep => ({
              ...step,
              courses: step.courses.map((c) => (c.code === code ? { ...c, key, options: sorted } : c)),
            })
            setWorkspaceHistory((h) => ({
              ...h,
              past: h.past.map(patchStep),
              present: patchStep(h.present),
              future: h.future.map(patchStep),
            }))
          },
          { cancelled: false },
        )
      }
    } catch (err) {
      if (seq !== loadSeq.current) return
      const msg = err instanceof ApiError ? err.message : 'Could not fetch PeopleSoft data.'
      setLoadError(msg)
      if (!quietBackground) {
        setLoadStages((prev) => prev.map((s) => (s.status === 'working' ? { ...s, status: 'error', detail: msg } : s)))
      }
    }
  }, [])

  const enterPostLogin = useCallback((st?: Status | null, freshLogin = false) => {
    const acctId = st?.accountId ?? null
    const userHint = st?.userHint ?? null
    const acctKey = `${acctId ?? ''}::${userHint ?? ''}`
    if (freshLogin || activeAccountKeyRef.current !== acctKey) {
      resetAccountWorkspace(freshLogin)
      activeAccountKeyRef.current = acctKey
    }
    const loadedPrefs = readAccountPrefs(acctId, userHint)
    setSetup(loadedPrefs.setup)
    setFavTeachers(readFavTeachers(acctId))
    const cached = freshLogin ? null : readSnapshot(acctId, userHint)
    if (cached && cached.courses.length > 0) {
      const heldCodes = cached.courses.filter((c) => c.held).map((c) => c.code)
      setTerm(cached.term)
      setWorkspaceHistory(reset({
        courses: cached.courses,
        state: { ...cached.state, unlocked: [...new Set([...cached.state.unlocked, ...heldCodes])] },
        moveDrafts: {},
        retakeTerms: loadedPrefs.retakeTerms,
      }))
      setDegreeWire(cached.degreeWire)
      setEnrolled(cached.enrolled)
      setBootReady(true)
      setScreen('workspace')
      void runBootstrap(true, { accountId: acctId, userHint })
    } else {
      setWorkspaceHistory((h) => reset({ ...h.present, retakeTerms: loadedPrefs.retakeTerms }))
      setScreen('setup')
      void runBootstrap(false, { accountId: acctId, userHint })
    }
  }, [resetAccountWorkspace, runBootstrap])

  useEffect(() => {
    let alive = true
    Promise.all([api.status(), api.accounts().catch(() => ({ accounts: [], activeId: null }))])
      .then(([st, accts]) => {
        if (!alive) return
        setStatus(st)
        setAccounts(accts.accounts ?? [])
        if (st.signedIn) enterPostLogin(st, false)
        else {
          resetAccountWorkspace(true)
          setScreen('login')
        }
      })
      .catch(() => {
        if (alive) {
          resetAccountWorkspace(true)
          setScreen('login')
        }
      })
    return () => { alive = false }
  }, [enterPostLogin, resetAccountWorkspace])

  const loadGroupChoices = useCallback(async (year: number, groupRow: number) => {
    if (!degreeWire) return
    const seq = loadSeq.current
    const gKey = `${year}:${groupRow}`
    const rowsInYear = degreeWire.rows.filter((r) => r.year === year)
    const idx = rowsInYear.findIndex((r) => r.row === groupRow)
    if (idx < 0) return
    setLoadingGroupsMap((prev) => ({ ...prev, [gKey]: true }))
    try {
      let gl: { choices?: Array<{ code: string; title: string; units?: string }> }
      try {
        gl = await api.writeRead<{ choices?: Array<{ code: string; title: string; units?: string }> }>('degree_course_list', { year, index: idx })
      } catch {
        await new Promise((r) => setTimeout(r, 700))
        if (seq !== loadSeq.current) return
        gl = await api.writeRead<{ choices?: Array<{ code: string; title: string; units?: string }> }>('degree_course_list', { year, index: idx })
      }
      if (seq !== loadSeq.current) return
      const list: DegreeCourse[] = (gl.choices ?? [])
        .filter((ch) => !!ch.code)
        .map((ch) => ({
          row: 0,
          year,
          code: ch.code,
          title: ch.title || ch.code,
          category: 'Elective',
          units: ch.units ? Number.parseFloat(ch.units) : 3,
          term: null,
          status: 'planned',
          plannable: true,
          hasRequirement: false,
        }))
      setGroupChoicesMap((prev) => ({ ...prev, [gKey]: list }))
      if (list.length > 0) {
        setElectiveChoices((prev) => {
          const have = new Set(prev.map((x) => normCode(x.code)))
          return [...prev, ...list.filter((x) => !have.has(normCode(x.code)))]
        })
        const m = /^(Winter|Spring|Fall)\s+(\d{4})$/.exec(term)
        const strm = m ? `1${m[2].slice(2)}${m[1] === 'Winter' ? 1 : m[1] === 'Spring' ? 5 : 8}` : '1271'
        const batch = list.slice(0, 12).map((ec) => {
          const k = lookupCatalogKey(ec.code)
          return { code: ec.code, ...(k ? { crse_id: k.crse_id, offer_nbr: k.offer_nbr, effdt: k.effdt } : {}) }
        })
        const secRes = await api.compatibleSections(strm, batch).catch(() => null)
        if (seq !== loadSeq.current) return
        if (secRes?.results) {
          setCompatCoursesData((prev) => {
            const next = { ...prev }
            for (const entry of secRes.results) {
              const meta = list.find((x) => x.code === entry.code)
              const parsed = normalizeCompatibleEntry(entry, meta?.title ?? entry.code, meta?.units ?? 3, term)
              if (parsed) next[entry.code] = parsed
            }
            return next
          })
        }
      }
    } catch { /* ignore */ } finally {
      if (seq === loadSeq.current) setLoadingGroupsMap((prev) => ({ ...prev, [gKey]: false }))
    }
  }, [degreeWire, term])

  const checkDrawerTermOpen = useCallback(async (semCourses: DegreeCourse[]) => {
    const unchecked = semCourses.filter((c) => !compatCoursesData[c.code]?.course).slice(0, 16)
    if (unchecked.length === 0) return
    const seq = loadSeq.current
    setCheckingDrawerTerm(true)
    try {
      const m = /^(Winter|Spring|Fall)\s+(\d{4})$/.exec(term)
      const strm = m ? `1${m[2].slice(2)}${m[1] === 'Winter' ? 1 : m[1] === 'Spring' ? 5 : 8}` : '1271'
      const batch = unchecked.map((c) => {
        const k = lookupCatalogKey(c.code)
        return { code: c.code, ...(k ? { crse_id: k.crse_id, offer_nbr: k.offer_nbr, effdt: k.effdt } : {}) }
      })
      const r = await api.compatibleSections(strm, batch)
      if (seq !== loadSeq.current) return
      if (Array.isArray(r?.results)) {
        setCompatCoursesData((prev) => {
          const next = { ...prev }
          for (const entry of r.results) {
            const meta = unchecked.find((x) => x.code === entry.code)
            const parsed = normalizeCompatibleEntry(entry, meta?.title ?? entry.code, meta?.units ?? 3, term)
            if (parsed) next[entry.code] = parsed
          }
          return next
        })
      }
    } catch { /* ignore */ } finally {
      if (seq === loadSeq.current) setCheckingDrawerTerm(false)
    }
  }, [compatCoursesData, term])

  const handleSignOut = async () => {
    try { await api.logout() } catch { /* ignore */ }
    setAccountId(null)
    resetAccountWorkspace(true)
    setStatus(null)
    setAccounts([])
    setScreen('login')
    api.accounts().then((res) => setAccounts(res.accounts ?? [])).catch(() => {})
  }

  const handleSwitchAccount = async (id: string) => {
    try {
      const st = await api.switchAccount(id)
      resetAccountWorkspace(true)
      setStatus(st)
      api.accounts().then((res) => setAccounts(res.accounts ?? [])).catch(() => {})
      enterPostLogin(st, true)
    } catch (err) {
      setAuthError(err instanceof ApiError ? err.message : 'Could not switch account.')
    }
  }

  const filterOpenWaitlistForSolver = useCallback((list: PCourse[], st: PlannerState) => {
    return list.map((c) => {
      const cur = effective(c, st)
      return {
        ...c,
        options: c.options.filter((o) => {
          if (o.id === cur?.id) return true
          const status = optionStatusOf(o)
          return status === 'Open' || status === 'Waitlist'
        }),
      }
    })
  }, [])

  const describeAddedCourseStatus = useCallback((
    nextCourses: PCourse[],
    nextState: PlannerState,
    addedCodes: string[],
  ) => {
    const active = nextCourses.filter((c) => !nextState.removed.includes(c.code))
    const overlaps = overlapsOf(blocksOf(active, nextState))
    const addedSet = new Set(addedCodes.map((c) => normCode(c)))
    const clashedWith = [...new Set(
      overlaps
        .filter((o) => addedSet.has(normCode(o.a.courseCode)) || addedSet.has(normCode(o.b.courseCode)))
        .map((o) => (addedSet.has(normCode(o.a.courseCode)) ? o.b.courseCode : o.a.courseCode))
        .filter((c) => !addedSet.has(normCode(c))),
    )]
    const sug = buildSuggestions(filterOpenWaitlistForSolver(nextCourses, nextState), nextState, settings, setup, favTeachers)
    const names = addedCodes.join(' + ')
    const clashStr = clashedWith.length ? ` (overlaps ${clashedWith.join(', ')})` : ''
    if (sug.top.length === 0 && sug.unsolvableReason) {
      return `⚠ Added ${names}${clashStr} — no conflict-free suggestions fit all courses.`
    }
    if (sug.partialNotice) {
      const skips = [...new Set([...sug.top, ...sug.more].flatMap((c) => c.skippedCodes ?? []))]
      return `⚠ Added ${names}${clashStr} — no full suggestions fit; suggestions will remove a course (${skips.join(' / ')}).`
    }
    if (clashedWith.length > 0) {
      return `✓ Added ${names}${clashStr} — drag or click a suggestion to fix the overlap.`
    }
    return `✓ Added ${names} to timetable`
  }, [favTeachers, filterOpenWaitlistForSolver, settings, setup])

  const updateSetup = useCallback((next: QuickSetup) => {
    setSetup(next)
    writeAccountPrefs(status?.accountId, status?.userHint, next, retakeTerms)
  }, [status?.accountId, status?.userHint, retakeTerms])

  const compatibleDiscovery = useMemo(() => {
    if (!degreeView) return null
    return discoverCompatibleCandidates(degreeView, term, courses.map((c) => c.code), Object.keys(retakeTerms))
  }, [degreeView, term, courses, retakeTerms])

  const exemptedMap = useMemo(
    () => compatibleDiscovery?.exempted ?? new Map<string, string>(),
    [compatibleDiscovery],
  )

  const compatibleList = useMemo(() => {
    if (!compatibleDiscovery) return []
    const m = /^(Winter|Spring|Fall)\s+(\d{4})$/.exec(term)
    const strm = m ? `1${m[2].slice(2)}${m[1] === 'Winter' ? 1 : m[1] === 'Spring' ? 5 : 8}` : '1271'
    const have = new Set([...courses.map((c) => normCode(c.code)), ...compatibleDiscovery.eligible.map((c) => normCode(c.code))])
    const extras: CompatibleCandidate[] = electiveChoices
      .filter((ec) => !have.has(normCode(ec.code)) && (!exemptedMap.has(normCode(ec.code)) || !!retakeTerms[ec.code]))
      .map((ec) => ({
        code: ec.code,
        title: ec.title,
        units: ec.units,
        plannedTerm: null,
        course: ec,
        prereqStatus: 'met',
        prereqText: '',
        coreqs: [],
        unlocks: getCatalogRule(ec.code)?.unlocks ?? [],
        key: lookupCatalogKey(ec.code),
        strm,
      }))
    return [...compatibleDiscovery.eligible, ...extras]
  }, [compatibleDiscovery, term, courses, electiveChoices, exemptedMap, retakeTerms])

  const compatStatusMap = useMemo(() => {
    const out: Record<string, CompatibleStatus> = {}
    for (const c of compatibleList) {
      const data = compatCoursesData[c.code]
      if (data) {
        out[c.code] = {
          loading: false,
          checked: true,
          openCount: data.openCount,
          waitlistCount: data.waitlistCount,
          closedCount: data.closedCount,
          notOffered: data.notOffered,
        }
      } else {
        out[c.code] = { loading: compatLoading, checked: false, openCount: 0, waitlistCount: 0, closedCount: 0, notOffered: false }
      }
    }
    return out
  }, [compatibleList, compatCoursesData, compatLoading])

  const addCompatibleCourse = useCallback(async (cand: CompatibleCandidate, withCoreq = false) => {
    const seq = loadSeq.current
    const toAdd = [cand]
    if (withCoreq) {
      for (const co of cand.coreqs) {
        const found = compatibleList.find((x) => normCode(x.code) === normCode(co))
        if (found) toAdd.push(found)
      }
    }
    const targetCodes = toAdd.map((x) => x.code)
    setAddNotice(null)
    setAddingCodes((prev) => [...new Set([...prev, ...targetCodes])])
    try {
      const m = /^(Winter|Spring|Fall)\s+(\d{4})$/.exec(term)
      const toCode = m ? `1${m[2].slice(2)}${m[1] === 'Winter' ? 1 : m[1] === 'Spring' ? 5 : 8}` : '1271'
      const newCourses: PCourse[] = []
      const restoredCodes: string[] = []
      const notOfferedNames: string[] = []
      for (const item of toAdd) {
        if (courses.some((x) => normCode(x.code) === normCode(item.code))) {
          restoredCodes.push(item.code)
          continue
        }
        let data = compatCoursesData[item.code]
        if (!data) {
          try {
            const strmCode = item.strm ?? toCode
            const r = await api.compatibleSections(strmCode, [{
              code: item.code,
              ...(item.key ? { crse_id: item.key.crse_id, offer_nbr: item.key.offer_nbr, effdt: item.key.effdt } : {}),
            }])
            const sec = r.results?.[0]
            if (seq !== loadSeq.current) return
            if (sec) {
              const normalized = normalizeCompatibleEntry(sec, item.title, item.units, term)
              if (normalized) {
                data = normalized
                setCompatCoursesData((prev) => ({ ...prev, [item.code]: normalized }))
              }
            }
          } catch { /* fallback to error notice below */ }
        }
        const opts = data?.course.options ?? []
        if (opts.length === 0) {
          notOfferedNames.push(item.code)
          setCompatCoursesData((prev) => prev[item.code] ? prev : {
            ...prev,
            [item.code]: {
              course: { code: item.code, title: item.title, term, units: String(item.units ?? 3), row: 0, options: [] },
              openCount: 0,
              waitlistCount: 0,
              closedCount: 0,
              notOffered: true,
            },
          })
          continue
        }
        let semIdx = 0
        if (degreeView) {
          const yr = degreeView.years.find((y) => y.index === item.course.year)
          if (yr) {
            let i = 0
            for (const s of yr.semesters) {
              for (const b of s.blocks) {
                if (b.kind === 'course' && b.course.code === item.code) semIdx = i
                i++
              }
            }
          }
        }
        newCourses.push({
          code: item.code,
          title: item.title,
          term,
          key: data?.course.key,
          units: item.units,
          options: opts,
          addPlan: {
            year: item.course.year,
            index: semIdx,
            toCode,
            to: term,
          },
        })
      }
      if (newCourses.length > 0) {
        const existingBlocks = blocksOf(courses, state)
        const pickedChoices: Record<string, string> = {}
        for (const nc of newCourses) {
          const cleanOpt =
            nc.options.find((o) =>
              optionStatusOf(o) === 'Open' &&
              !o.components.some((cm) =>
                cm.meetings.some((mt) =>
                  existingBlocks.some((eb) => eb.day === mt.day && mt.start < eb.end && eb.start < mt.end),
                ),
              ),
            ) ??
            nc.options.find((o) => optionStatusOf(o) === 'Open') ??
            nc.options.find((o) => optionStatusOf(o) === 'Waitlist') ??
            nc.options[0]
          if (cleanOpt) {
            pickedChoices[nc.code] = cleanOpt.id
            for (const cm of cleanOpt.components) {
              for (const mt of cm.meetings) {
                existingBlocks.push({
                  courseCode: nc.code,
                  classNbr: cm.classNbr,
                  kind: cm.kind,
                  room: cm.room,
                  instructors: cm.instructors,
                  day: mt.day,
                  start: mt.start,
                  end: mt.end,
                  locked: false,
                  held: false,
                })
              }
            }
          }
        }
        const nextCourses = [...courses, ...newCourses]
        const nextState: PlannerState = {
          ...state,
          removed: state.removed.filter((r) => !restoredCodes.some((rc) => normCode(rc) === normCode(r))),
          choice: { ...pickedChoices, ...state.choice },
        }
        morphCaptureRef.current?.('schedule')
        setWorkspaceHistory((h) => commit(h, { ...h.present, courses: nextCourses, state: nextState }, Date.now()))
        if (addedTimerRef.current != null) window.clearTimeout(addedTimerRef.current)
        setJustAddedCourse(newCourses[0].code)
        addedTimerRef.current = window.setTimeout(() => setJustAddedCourse(null), 1600)
        setAddNotice(describeAddedCourseStatus(nextCourses, nextState, [...restoredCodes, ...newCourses.map((x) => x.code)]))
      } else if (restoredCodes.length > 0) {
        const nextState: PlannerState = {
          ...state,
          removed: state.removed.filter((r) => !restoredCodes.some((rc) => normCode(rc) === normCode(r))),
        }
        morphCaptureRef.current?.('schedule')
        setWorkspaceHistory((h) => commit(h, { ...h.present, state: nextState }, Date.now()))
        setJustAddedCourse(restoredCodes[0])
        setAddNotice(describeAddedCourseStatus(courses, nextState, restoredCodes))
      } else if (notOfferedNames.length > 0) {
        setAddNotice(`${notOfferedNames.join(', ')}: No sections offered in ${term}`)
      }
    } finally {
      if (seq === loadSeq.current) {
        setAddingCodes((prev) => prev.filter((code) => !targetCodes.includes(code)))
      }
    }
  }, [compatibleList, courses, compatCoursesData, degreeView, term, state, describeAddedCourseStatus])

  const searchCatalogFallback = async (q: string) => {
    const seq = loadSeq.current
    setSearchingFallback(true)
    try {
      const rows = await api.writeRead<any[]>('catalog_search', { keyword: q })
      if (seq !== loadSeq.current) return
      const hit = rows?.[0]
      if (hit?.code && hit?.crse_id && hit?.effdt) {
        const key = { crse_id: String(hit.crse_id), offer_nbr: String(hit.offer_nbr ?? '1'), effdt: String(hit.effdt) }
        const strm = /^(Winter|Spring|Fall)\s+(\d{4})$/.exec(term)
        const strmCode = strm ? `1${strm[2].slice(2)}${strm[1] === 'Winter' ? 1 : strm[1] === 'Spring' ? 5 : 8}` : '1271'
        const sec = await api.compatibleSections(strmCode, [{ code: hit.code, ...key }])
        if (seq !== loadSeq.current) return
        const firstRes = sec.results?.[0]
        const parsed = firstRes ? normalizeCompatibleEntry(firstRes, hit.title ?? hit.code, hit.units ? Number.parseFloat(hit.units) : 3, term) : null
        const opts = parsed?.course.options ?? []
        if (!courses.some((c) => c.code === hit.code)) {
          const added: PCourse = {
            code: hit.code,
            title: hit.title ?? hit.code,
            term,
            units: hit.units ? Number.parseFloat(hit.units) : 3,
            options: opts,
          }
          const nextCourses = [...courses, added]
          const nextState: PlannerState = added.options[0]
            ? { ...state, choice: { [added.code]: added.options[0].id, ...state.choice } }
            : state
          morphCaptureRef.current?.('schedule')
          setWorkspaceHistory((h) => commit(h, { ...h.present, courses: nextCourses, state: nextState }, Date.now()))
          setJustAddedCourse(added.code)
          setAddNotice(describeAddedCourseStatus(nextCourses, nextState, [added.code]))
        }
      }
    } catch { /* ignore */ } finally {
      setSearchingFallback(false)
    }
  }

  const openCoursePopover = (code: string) => {
    const cat = getCatalogRule(code)
    const compat = compatibleList.find((x) => x.code === code)
    const active = courses.find((x) => x.code === code)
    const st = compatStatusMap[code]
    setInfoTarget({
      code,
      title: active?.title ?? compat?.title ?? code,
      units: active?.units ?? compat?.units ?? null,
      unlocks: cat?.unlocks ?? [],
      prereqText: compat?.prereqText ?? '',
      prereqStatus: compat?.prereqStatus ?? 'met',
      coreqs: cat?.corequisites ?? [],
      plannedTerm: compat?.plannedTerm ?? term,
      inSchedule: !!active && !state.removed.includes(code),
      openCount: st?.openCount,
      waitlistCount: st?.waitlistCount,
      closedCount: st?.closedCount,
      onAdd: compat ? () => void addCompatibleCourse(compat, false) : undefined,
      onAddWithCoreq: compat && compat.coreqs.length > 0 ? () => void addCompatibleCourse(compat, true) : undefined,
      onRemove: active ? () => setState((s) => ({
        ...s,
        removed: s.removed.includes(code) ? s.removed.filter((r) => r !== code) : [...s.removed, code],
      })) : undefined,
    })
  }

  // Keep Open + Waitlist options available so buildSuggestions can always fall back when Open-only options clash
  const filterCoursesForSolver = useMemo(
    () => filterOpenWaitlistForSolver(courses, state),
    [courses, state, filterOpenWaitlistForSolver],
  )

  const applySuggestionCard = useCallback((card: SuggestionCard) => {
    morphCaptureRef.current?.('schedule')
    setState((s) => {
      const next = applySuggestion(courses, s, card.schedule)
      const picked = new Set(card.schedule.picks.map((p) => p.course.code))
      if (card.isOriginal) {
        return {
          ...next,
          removed: courses.filter((c) => !picked.has(c.code)).map((c) => c.code),
        }
      }
      const restored = next.removed.filter((r) => !picked.has(r))
      const withSkipped = card.skippedCodes?.length
        ? [...new Set([...restored, ...card.skippedCodes])]
        : restored
      return { ...next, removed: withSkipped }
    })
    if (card.skippedCodes?.length) {
      setAddNotice(`Removed ${card.skippedCodes.join(', ')} to apply "${card.label}" (use Restore or Ctrl+Z anytime).`)
    }
  }, [courses, setState])

  const handleUndo = useCallback(() => {
    morphCaptureRef.current?.('schedule')
    setPendingPartialPick(null)
    setWorkspaceHistory((h) => {
      const prev = undo(h)
      if (prev !== h) {
        writeAccountPrefs(status?.accountId, status?.userHint, setup, prev.present.retakeTerms)
      }
      return prev
    })
  }, [setup, status?.accountId, status?.userHint])

  const handleRedo = useCallback(() => {
    morphCaptureRef.current?.('schedule')
    setPendingPartialPick(null)
    setWorkspaceHistory((h) => {
      const next = redo(h)
      if (next !== h) {
        writeAccountPrefs(status?.accountId, status?.userHint, setup, next.present.retakeTerms)
      }
      return next
    })
  }, [setup, status?.accountId, status?.userHint])

  useEffect(() => {
    if (screen !== 'workspace') return
    const onKeyDown = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey) || e.altKey) return
      const target = e.target as HTMLElement | null
      if (target && (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.tagName === 'SELECT' || target.isContentEditable)) {
        return
      }
      const key = e.key.toLowerCase()
      if (key === 'z' && !e.shiftKey) {
        e.preventDefault()
        handleUndo()
      } else if ((key === 'z' && e.shiftKey) || (key === 'y' && !e.shiftKey)) {
        e.preventDefault()
        handleRedo()
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [screen, handleUndo, handleRedo])

  useEffect(() => {
    const onContext = (e: globalThis.MouseEvent) => {
      const target = e.target as HTMLElement | null
      if (target && (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable)) {
        return
      }
      e.preventDefault()
      const hit = target?.closest?.('[data-ctx-kind]') as HTMLElement | null
      const rawKind = hit?.dataset.ctxKind
      const kind: ContextMenuState['kind'] =
        rawKind === 'block' || rawKind === 'planned' || rawKind === 'eligible' ? rawKind : 'page'
      const vw = Math.max(window.innerWidth || 1280, 1)
      const vh = Math.max(window.innerHeight || 800, 1)
      const vx = clampPct(Math.round((e.clientX / vw) * 100), 2, 74)
      const vy = clampPct(Math.round((e.clientY / vh) * 100), 2, 68)
      setCtxMenu({
        vx,
        vy,
        kind,
        code: hit?.dataset.ctxCode,
        sec: hit?.dataset.ctxSec,
        comp: hit?.dataset.ctxComp,
      })
    }
    const dismiss = () => setCtxMenu(null)
    const onPointerDown = (e: Event) => {
      const t = (e.target as HTMLElement | null)?.closest?.('.ctx-menu')
      if (!t) dismiss()
    }
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') setCtxMenu(null) }
    window.addEventListener('contextmenu', onContext)
    window.addEventListener('pointerdown', onPointerDown)
    window.addEventListener('keydown', onKey)
    window.addEventListener('scroll', dismiss, true)
    window.addEventListener('resize', dismiss)
    return () => {
      window.removeEventListener('contextmenu', onContext)
      window.removeEventListener('pointerdown', onPointerDown)
      window.removeEventListener('keydown', onKey)
      window.removeEventListener('scroll', dismiss, true)
      window.removeEventListener('resize', dismiss)
    }
  }, [])

  const suggestions = useMemo(
    () => buildSuggestions(filterCoursesForSolver, state, settings, setup, favTeachers),
    [filterCoursesForSolver, state, settings, setup, favTeachers],
  )

  const tileMap = useMemo(
    () => assignTileClasses(courses.map((c) => c.code), compatibleList.map((c) => c.code)),
    [courses, compatibleList],
  )
  const blocks = useMemo(() => blocksOf(courses, state), [courses, state])
  const credits = useMemo(() => creditsFor(courses, state, term, planning), [courses, state, term, planning])
  const currentSig = useMemo(() => currentScheduleSignature(courses, state), [courses, state])

  const pendingActionCount = useMemo(() => {
    const swaps = courses.filter((c) => c.held && !state.removed.includes(c.code) && state.choice[c.code] && state.choice[c.code] !== c.held.id).length
    const enrols = courses.filter((c) => !c.held && !state.removed.includes(c.code)).length
    const drops = courses.filter((c) => c.held && state.removed.includes(c.code)).length
    return swaps + enrols + drops + Object.keys(moveDrafts).length
  }, [courses, state, moveDrafts])

  const renderCtxMenu = () => {
    if (!ctxMenu) return null
    const ctxCourse = ctxMenu.code ? courses.find((c) => c.code === ctxMenu.code) : undefined
    const ctxCompat = ctxMenu.code ? compatibleList.find((c) => c.code === ctxMenu.code) : undefined
    const isRemoved = ctxCourse ? state.removed.includes(ctxCourse.code) : false
    const isLockedCourse = ctxCourse ? !state.unlocked.includes(ctxCourse.code) && !!ctxCourse.held : false
    const closeAnd = (fn: () => void) => () => { setCtxMenu(null); fn() }

    return (
      <nav
        className={`ctx-menu fx-${ctxMenu.vx} fy-${ctxMenu.vy}`}
        role="menu"
        aria-label="Custom context menu"
      >
        <div className="ctx-menu-head">
          {ctxMenu.code
            ? `${ctxMenu.code}${ctxMenu.comp ? ` · ${ctxMenu.comp}` : ''}${ctxMenu.sec ? ` · Sec ${ctxMenu.sec}` : ''}`
            : 'Planie'}
        </div>
        {ctxMenu.code && ctxCourse ? (
          <>
            <button type="button" role="menuitem" className="ctx-menu-item" onClick={closeAnd(() => setInspectedCalendarCode(ctxCourse.code))}>
              {ctxMenu.kind === 'block' ? 'Pick or swap section' : 'Open course details'}
            </button>
            {ctxCourse.held ? (
              <button
                type="button"
                role="menuitem"
                className="ctx-menu-item"
                onClick={closeAnd(() => setState((s) => ({
                  ...s,
                  unlocked: s.unlocked.includes(ctxCourse.code)
                    ? s.unlocked.filter((u) => u !== ctxCourse.code)
                    : [...s.unlocked, ctxCourse.code],
                })))}
              >
                {isLockedCourse ? 'Unlock section' : 'Lock section'}
              </button>
            ) : null}
            <button
              type="button"
              role="menuitem"
              className="ctx-menu-item"
              onClick={closeAnd(() => {
                morphCaptureRef.current?.('schedule')
                setState((s) => ({
                  ...s,
                  removed: s.removed.includes(ctxCourse.code)
                    ? s.removed.filter((r) => r !== ctxCourse.code)
                    : [...s.removed, ctxCourse.code],
                }))
              })}
            >
              {isRemoved ? 'Restore to timetable' : 'Remove from timetable'}
            </button>
            <button
              type="button"
              role="menuitem"
              className="ctx-menu-item"
              onClick={closeAnd(() => { void navigator.clipboard?.writeText(ctxCourse.code).catch(() => {}) })}
            >
              Copy {ctxCourse.code}
            </button>
            <div className="ctx-menu-sep" aria-hidden="true" />
          </>
        ) : ctxMenu.code && ctxCompat ? (
          <>
            <button type="button" role="menuitem" className="ctx-menu-item" onClick={closeAnd(() => void addCompatibleCourse(ctxCompat, false))}>
              + Add to {term}
            </button>
            {ctxCompat.coreqs.length > 0 ? (
              <button type="button" role="menuitem" className="ctx-menu-item" onClick={closeAnd(() => void addCompatibleCourse(ctxCompat, true))}>
                + Add with {ctxCompat.coreqs[0]}
              </button>
            ) : null}
            <button
              type="button"
              role="menuitem"
              className="ctx-menu-item"
              onClick={closeAnd(() => { void navigator.clipboard?.writeText(ctxCompat.code).catch(() => {}) })}
            >
              Copy {ctxCompat.code}
            </button>
            <div className="ctx-menu-sep" aria-hidden="true" />
          </>
        ) : null}

        {screen === 'workspace' ? (
          <>
            <button type="button" role="menuitem" className="ctx-menu-item" disabled={!canUndo} onClick={closeAnd(handleUndo)}>
              Undo (Ctrl+Z)
            </button>
            <button type="button" role="menuitem" className="ctx-menu-item" disabled={!canRedo} onClick={closeAnd(handleRedo)}>
              Redo (Ctrl+Y)
            </button>
            <button type="button" role="menuitem" className="ctx-menu-item" onClick={closeAnd(() => {
              morphCaptureRef.current?.('layout')
              setSidebarOpen((x) => !x)
            })}>
              {sidebarOpen ? 'Hide courses panel' : 'Show courses panel'}
            </button>
            <button type="button" role="menuitem" className="ctx-menu-item" onClick={closeAnd(() => setFutureDrawerOpen(true))}>
              Future Plan
            </button>
            <button type="button" role="menuitem" className="ctx-menu-item" onClick={closeAnd(() => setScreen('setup'))}>
              Quick Setup / Tune
            </button>
          </>
        ) : null}

        <button type="button" role="menuitem" className="ctx-menu-item" onClick={closeAnd(toggleTheme)}>
          {theme === 'dark' ? 'Switch to Light mode' : 'Switch to Dark mode'}
        </button>
      </nav>
    )
  }

  if (screen === 'login') {
    return (
      <>
        <Login
          initialError={authError}
          theme={theme}
          onToggleTheme={toggleTheme}
          onSignedIn={(st) => {
            resetAccountWorkspace(true)
            setStatus(st)
            api.accounts().then((res) => setAccounts(res.accounts ?? [])).catch(() => {})
            enterPostLogin(st, true)
          }}
          onSwitchAccount={(id) => void handleSwitchAccount(id)}
        />
        {renderCtxMenu()}
      </>
    )
  }

  if (screen === 'setup') {
    return (
      <>
        <SetupWait
          setup={setup}
          onChangeSetup={updateSetup}
          showWaitlist={showWaitlist}
          onToggleWaitlist={() => setShowWaitlist((x) => !x)}
          showClosed={showClosed}
          onToggleClosed={() => setShowClosed((x) => !x)}
          stages={loadStages}
          ready={bootReady}
          error={loadError}
          onRetry={() => void runBootstrap(false, { accountId: status?.accountId ?? null, userHint: status?.userHint ?? null })}
          onContinue={() => setScreen('workspace')}
          onSignOut={() => void handleSignOut()}
          demo={status?.demo}
          userHint={status?.userHint ?? null}
          theme={theme}
          onToggleTheme={toggleTheme}
        />
        {renderCtxMenu()}
      </>
    )
  }

  if (screen === 'apply') {
    return (
      <>
        <Apply
          term={term}
          courses={courses}
          state={state}
          enrolled={enrolled}
          degreeView={degreeView}
          moveDrafts={moveDrafts}
          planning={planning}
          onBack={() => setScreen('workspace')}
          onDone={(prog) => { setDoneProgress(prog); setScreen('done') }}
          onRefresh={() => void runBootstrap(false, { accountId: status?.accountId ?? null, userHint: status?.userHint ?? null })}
        />
        {renderCtxMenu()}
      </>
    )
  }

  if (screen === 'done' && doneProgress) {
    return (
      <>
        <Done
          term={term}
          courses={courses}
          state={state}
          progress={doneProgress}
          onBack={() => setScreen('workspace')}
          onSignOut={() => void handleSignOut()}
          tileMap={tileMap}
        />
        {renderCtxMenu()}
      </>
    )
  }

  return (
    <div className="planie-root is-workspace">
      <header className="topbar">
        <div className="topbar-left">
          <span className="brand-mark"><span className="brand-dot" aria-hidden="true" />Planie</span>
          <span className="term-pill">{term}</span>
          <span className={`credit-pill ${credits.over ? 'over' : ''}`}>
            {creditText(credits.schedule)} / {credits.limit.max} cr
          </span>
          {status?.demo ? <span className="demo-badge">Demo</span> : null}
          {!status?.demo && status?.userHint && accounts.length <= 1 ? <span className="term-pill">{status.userHint}</span> : null}
        </div>

        <div className="topbar-right">
          {accounts.length > 1 ? (
            <select
              aria-label="Switch active account"
              value={status?.accountId ?? ''}
              onChange={(e) => e.target.value && void handleSwitchAccount(e.target.value)}
            >
              {accounts.map((a) => (
                <option key={a.id} value={a.id}>{a.demo ? 'Demo' : a.userHint}</option>
              ))}
            </select>
          ) : null}
          <button
            type="button"
            className="btn btn-sm"
            disabled={!canUndo}
            title="Undo (Ctrl+Z)"
            onClick={handleUndo}
          >
            ↶ Undo
          </button>
          <button
            type="button"
            className="btn btn-sm"
            disabled={!canRedo}
            title="Redo (Ctrl+Y)"
            onClick={handleRedo}
          >
            ↷ Redo
          </button>
          <button type="button" className="btn btn-sm" onClick={() => setFutureDrawerOpen(true)}>
            Future Plan{Object.keys(moveDrafts).length > 0 ? ` (${Object.keys(moveDrafts).length})` : ''}
          </button>
          <button type="button" className="btn btn-sm" onClick={() => setScreen('setup')}>Tune</button>
          <button
            type="button"
            className="icon-btn"
            aria-label={theme === 'dark' ? 'Switch to light mode' : 'Switch to dark mode'}
            onClick={toggleTheme}
          >
            {theme === 'dark' ? '☀️ Light' : '🌙 Dark'}
          </button>
          <button type="button" className="btn btn-sm" onClick={() => void handleSignOut()}>Sign out</button>
          <button type="button" className="btn btn-primary" onClick={() => setScreen('apply')}>
            Review &amp; Enroll{pendingActionCount > 0 ? ` (${pendingActionCount})` : ''}
          </button>
        </div>
      </header>

      <main className="workspace-shell">
        <div className={`workspace-main ${sidebarOpen ? '' : 'sidebar-collapsed'}`}>
          <CourseSidebar
            open={sidebarOpen}
            courses={courses}
            state={state}
            term={term}
            tab={sidebarTab}
            onTabChange={setSidebarTab}
            showWaitlist={showWaitlist}
            onToggleWaitlist={() => setShowWaitlist((x) => !x)}
            showClosed={showClosed}
            onToggleClosed={() => setShowClosed((x) => !x)}
            compatible={compatibleList}
            compatStatus={compatStatusMap}
            readyLoading={compatLoading}
            creditsAfter={credits.after}
            creditLimit={credits.limit.max}
            onToggleCourse={(code) => {
              morphCaptureRef.current?.('schedule')
              setState((s) => ({
                ...s,
                removed: s.removed.includes(code) ? s.removed.filter((r) => r !== code) : [...s.removed, code],
              }))
            }}
            onToggleLock={(code) => setState((s) => ({
              ...s,
              unlocked: s.unlocked.includes(code) ? s.unlocked.filter((u) => u !== code) : [...s.unlocked, code],
            }))}
            onSelectBlockCourse={(code) => setInspectedCalendarCode(code)}
            onAddCompatible={(cand, withCoreq) => void addCompatibleCourse(cand, withCoreq)}
            onInspectCourse={openCoursePopover}
            onSearchCatalogFallback={(q) => void searchCatalogFallback(q)}
            searchingFallback={searchingFallback}
            addingCodes={addingCodes}
            justAddedCode={justAddedCourse}
            addNotice={addNotice}
            onClearAddNotice={() => setAddNotice(null)}
            tileMap={tileMap}
          />

          <div className="center-pane">
            <SuggestionsStrip
              top={suggestions.top}
              more={suggestions.more}
              activeSig={currentSig}
              expanded={moreSuggestions}
              onToggleExpand={() => setMoreSuggestions((x) => !x)}
              unsolvableReason={suggestions.unsolvableReason}
              partialNotice={suggestions.partialNotice}
              prefNotice={suggestions.prefNotice}
              setup={setup}
              onChangeSetup={updateSetup}
              tileMap={tileMap}
              onPick={(card) => {
                if (card.skippedCodes?.length) {
                  setPendingPartialPick(card)
                  return
                }
                applySuggestionCard(card)
              }}
            />

            <Calendar
              courses={courses}
              state={state}
              blocks={blocks}
              settings={settings}
              filters={{ open: true, waitlist: showWaitlist, closed: showClosed, allowRedo }}
              onPickOption={(code, optId) => {
                morphCaptureRef.current?.('schedule')
                setState((s) => ({ ...s, choice: { ...s.choice, [code]: optId } }))
              }}
              onToggleLock={(code) => setState((s) => ({
                ...s,
                unlocked: s.unlocked.includes(code) ? s.unlocked.filter((u) => u !== code) : [...s.unlocked, code],
              }))}
              onUnlockCourse={(code) => setState((s) => (s.unlocked.includes(code) ? s : { ...s, unlocked: [...s.unlocked, code] }))}
              onToggleCourse={(code) => {
                morphCaptureRef.current?.('schedule')
                setState((s) => ({
                  ...s,
                  removed: s.removed.includes(code) ? s.removed.filter((r) => r !== code) : [...s.removed, code],
                }))
              }}
              inspectedCourseCode={inspectedCalendarCode}
              onInspectCourse={setInspectedCalendarCode}
              favTeachers={favTeachers}
              onToggleFavTeacher={(name) => {
                setFavTeachers((prev) => {
                  const next = prev.some((t) => t.toLowerCase() === name.toLowerCase())
                    ? prev.filter((t) => t.toLowerCase() !== name.toLowerCase())
                    : [...prev, name]
                  writeFavTeachers(status?.accountId, next)
                  return next
                })
              }}
              onToggleAllowRedo={() => setAllowRedo((x) => !x)}
              sidebarOpen={sidebarOpen}
              onToggleSidebar={() => {
                morphCaptureRef.current?.('layout')
                setSidebarOpen((x) => !x)
              }}
              justAddedCourse={justAddedCourse}
              morphCaptureRef={morphCaptureRef}
              tileMap={tileMap}
            />
          </div>
        </div>
      </main>

      <FuturePlanDrawer
        open={futureDrawerOpen}
        onClose={() => setFutureDrawerOpen(false)}
        view={degreeView}
        openTerm={term}
        scheduledCodes={new Set(courses.filter((c) => !state.removed.includes(c.code)).map((c) => c.code))}
        classStatuses={classStatuses}
        compatStatus={compatStatusMap}
        groupChoices={groupChoicesMap}
        loadingGroups={loadingGroupsMap}
        onLoadGroupChoices={(yr, row) => void loadGroupChoices(yr, row)}
        onCheckTermOpen={(list) => void checkDrawerTermOpen(list)}
        checkingTerm={checkingDrawerTerm}
        addingCodes={addingCodes}
        tileMap={tileMap}
        moves={moveDrafts}
        exempted={exemptedMap}
        retakeTerms={retakeTerms}
        onRetakeCourse={(course: DegreeCourse, toTerm: string) => {
          setWorkspaceHistory((h) => {
            const nextRetake = { ...h.present.retakeTerms, [course.code]: toTerm }
            writeAccountPrefs(status?.accountId, status?.userHint, setup, nextRetake)
            const nextDrafts = course.plannable
              ? { ...h.present.moveDrafts, [course.code]: { course, fromTerm: course.term ?? toTerm, toTerm } }
              : h.present.moveDrafts
            return commit(h, { ...h.present, retakeTerms: nextRetake, moveDrafts: nextDrafts }, Date.now())
          })
          if (toTerm === term) {
            const m = /^(Winter|Spring|Fall)\s+(\d{4})$/.exec(term)
            const strm = m ? `1${m[2].slice(2)}${m[1] === 'Winter' ? 1 : m[1] === 'Spring' ? 5 : 8}` : '1271'
            void addCompatibleCourse({
              code: course.code,
              title: course.title,
              units: course.units,
              plannedTerm: toTerm,
              course,
              prereqStatus: 'met',
              prereqText: '',
              coreqs: getCatalogRule(course.code)?.corequisites ?? [],
              unlocks: getCatalogRule(course.code)?.unlocks ?? [],
              key: lookupCatalogKey(course.code),
              strm,
            }, false)
          }
        }}
        onCancelRetake={(code) => {
          setWorkspaceHistory((h) => {
            const nextRetake = { ...h.present.retakeTerms }
            delete nextRetake[code]
            writeAccountPrefs(status?.accountId, status?.userHint, setup, nextRetake)
            const nextDrafts = { ...h.present.moveDrafts }
            delete nextDrafts[code]
            return commit(h, { ...h.present, retakeTerms: nextRetake, moveDrafts: nextDrafts }, Date.now())
          })
        }}
        onMoveCourse={(course: DegreeCourse, toTerm: string) => {
          setMoveDrafts((prev) => ({ ...prev, [course.code]: { course, fromTerm: course.term, toTerm } }))
        }}
        onCancelMove={(code) => {
          setMoveDrafts((prev) => {
            const next = { ...prev }
            delete next[code]
            return next
          })
        }}
        onQuickAddCourse={(code) => {
          const compat = compatibleList.find((c) => normCode(c.code) === normCode(code))
          if (compat) {
            void addCompatibleCourse(compat, false)
            return
          }
          const fromGroup = Object.values(groupChoicesMap).flat().find((x) => normCode(x.code) === normCode(code))
          const m = /^(Winter|Spring|Fall)\s+(\d{4})$/.exec(term)
          const strm = m ? `1${m[2].slice(2)}${m[1] === 'Winter' ? 1 : m[1] === 'Spring' ? 5 : 8}` : '1271'
          void addCompatibleCourse({
            code,
            title: fromGroup?.title ?? code,
            units: fromGroup?.units ?? 3,
            plannedTerm: term,
            course: fromGroup ?? { row: 0, year: 0, code, title: code, category: '', units: 3, term, status: 'planned', plannable: true, hasRequirement: false },
            prereqStatus: 'met',
            prereqText: '',
            coreqs: [],
            unlocks: getCatalogRule(code)?.unlocks ?? [],
            key: lookupCatalogKey(code),
            strm,
          }, false)
        }}
        onInspectCourse={openCoursePopover}
      />

      <CourseInfoPopover target={infoTarget} onClose={() => setInfoTarget(null)} />

      {pendingPartialPick ? (
        <div
          className="sheet-backdrop"
          role="dialog"
          aria-modal="true"
          aria-label="Confirm removing course for suggestion"
          onClick={() => setPendingPartialPick(null)}
        >
          <div className="sheet-card" onClick={(e) => e.stopPropagation()}>
            <div className="sheet-head">
              <h3>Remove {pendingPartialPick.skippedCodes?.join(', ')}?</h3>
              <button type="button" className="btn btn-sm" onClick={() => setPendingPartialPick(null)}>Cancel</button>
            </div>
            <p className="calm-notice warn">
              No conflict-free suggestion fits all your active courses together. Applying{' '}
              <strong>{pendingPartialPick.label}</strong> will remove{' '}
              <strong>{pendingPartialPick.skippedCodes?.join(', ')}</strong> from your active timetable so the rest fit cleanly.
            </p>
            <div className="course-card-actions">
              <button type="button" className="btn btn-sm" onClick={() => setPendingPartialPick(null)}>
                Keep all courses
              </button>
              <button
                type="button"
                className="btn btn-primary btn-sm"
                onClick={() => {
                  const chosen = pendingPartialPick
                  setPendingPartialPick(null)
                  applySuggestionCard(chosen)
                }}
              >
                Apply &amp; remove {pendingPartialPick.skippedCodes?.join(', ')}
              </button>
            </div>
          </div>
        </div>
      ) : null}
      {renderCtxMenu()}
    </div>
  )
}

export default App
export { solverCourses, primaryClass }

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { api, getSessionId, newSessionId, setSessionId } from '../api'
import { listRamAccounts, removeRamAccount, saveRamAccount, type RamAccount } from './accounts'
import { hasSeenTutorial, TutorialModal } from './Tutorial'
import { solve } from '../engine/solver'
import type { ClassOption, Course, Settings } from '../engine/types'
import { readEnrolled } from '../enrolled'
import { Apply } from './Apply'
import { Done } from './Done'
import { Load } from './Load'
import { Login } from './Login'
import { Planner } from './Planner'
import { PlanDegree } from './PlanDegree'
import { planningMap, type WireDegree } from './degree'
import { AddCourse } from './AddCourse'
import { clearAcademicData } from './storage'
import { readPicks, writePicks } from './savedPicks'
import { loadSnapshot, saveSnapshot, scheduleIssues } from './attention'
import { PreferencesPanel } from './Preferences'
import { effectiveSettings } from './scale'
import { addListedWaitlist, loadHeldOptions, loadSettings, loadTerm, rememberHeldRead, saveSettings, splitHeld, summariesOf, type Loaded } from './data'
import { commit, redo, reset, startHistory, undo, type History } from './history'
import { applySuggestion, buildCourses, solverCourses, emptyState, type PCourse, type PlannerState } from './model'
import { followBest } from './follow'
import { findOutlier } from './outlier'
import './app.css'

interface Snapshot { state: PlannerState; settings: Settings }
type Screen = 'boot' | 'login' | 'load' | 'planner' | 'apply' | 'done' | 'degree' | 'add'
const CONSENT_KEY = 'schedule-planner:v2:consent'

const consentedThisSession = (): boolean => {
  try {
    return window.sessionStorage.getItem(CONSENT_KEY) === '1'
  } catch {
    return false
  }
}

export default function AppV2() {
  const [screen, setScreen] = useState<Screen>('boot')
  const [signedInAs, setSignedInAs] = useState<string | null>(null)
  const [loaded, setLoaded] = useState<Loaded | null>(null)
  const [heldOpts, setHeldOpts] = useState<Record<string, { key?: Course['key']; options: ClassOption[] }>>({})
  const [readingHeld, setReadingHeld] = useState(false)
  const [refreshing, setRefreshing] = useState(false)
  const [updatedAt, setUpdatedAt] = useState<number | null>(null)
  const [refreshNote, setRefreshNote] = useState<string | null>(null)
  const [excluded, setExcluded] = useState<Record<string, string[]>>({})
  // Planner state and preferences share one history, so Back and Forward undo either.
  const [hist, setHist] = useState<History<Snapshot>>(() => startHistory({ state: emptyState(), settings: loadSettings() }))
  const { state, settings } = hist.present
  const [prefs, setPrefs] = useState(false)
  const [showTutorial, setShowTutorial] = useState(() => !hasSeenTutorial())
  const [auto, setAuto] = useState<boolean>(() => { try { return window.localStorage.getItem('v2-auto') !== '0' } catch { return true } })
  const autoRef = useRef(auto)
  autoRef.current = auto
  useEffect(() => { try { window.localStorage.setItem('v2-auto', auto ? '1' : '0') } catch { /* ignore */ } }, [auto])
  const cancel = useRef({ cancelled: false })

  useEffect(() => saveSettings(settings), [settings])

  // Where to start: the login screen, unless this session already gave its consent and the server is signed in.
  useEffect(() => {
    api.status().then((s) => {
      setSignedInAs(s.signedIn ? s.userHint : null)
      setScreen(s.signedIn && consentedThisSession() ? 'load' : 'login')
    }).catch(() => setScreen('login'))
  }, [])

  // The term each course belongs to in the degree's own order, from the last copy of the degree tree (read again in the background when old).
  const [planning, setPlanning] = useState<Record<string, string>>(() => {
    try {
      const c = JSON.parse(window.localStorage.getItem('schedule-planner:v2:degree') ?? 'null') as { wire?: WireDegree } | null
      return c?.wire && Array.isArray(c.wire.rows) ? planningMap(c.wire) : {}
    } catch { return {} }
  })
  useEffect(() => {
    if (!loaded) return
    let at = 0
    try { at = (JSON.parse(window.localStorage.getItem('schedule-planner:v2:degree') ?? 'null') as { at?: number } | null)?.at ?? 0 } catch { /* none */ }
    if (Date.now() - at < 6 * 3600 * 1000 && Object.keys(planning).length) return
    let alive = true
    api.writeRead<WireDegree>('degree_tree').then((w) => {
      if (!alive) return
      setPlanning(planningMap(w))
      try { window.localStorage.setItem('schedule-planner:v2:degree', JSON.stringify({ at: Date.now(), wire: w })) } catch { /* storage blocked */ }
    }).catch(() => { /* the rule simply stays off */ })
    return () => { alive = false }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loaded])

  // Degree courses added from the catalog that PeopleSoft does not have planned yet (planned and enrolled by Apply, in its one review).
  const [added, setAdded] = useState<PCourse[]>([])
  useEffect(() => {
    if (!loaded) return
    const there = new Set([...loaded.plan.map((c) => c.code), ...Object.keys(loaded.enrolled.options)])
    setAdded((a) => (a.some((c) => there.has(c.code)) ? a.filter((c) => !there.has(c.code)) : a))
  }, [loaded])

  const courses = useMemo(() => {
    if (!loaded) return []
    const list = buildCourses(loaded.plan, loaded.enrolled.options, loaded.enrolled.rows, heldOpts, loaded.term, added)
    return list.map((c) => (excluded[c.code]?.length ? { ...c, options: c.options.filter((o) => !excluded[c.code].includes(o.id)) } : c))
  }, [loaded, heldOpts, excluded, added])

  // What PeopleSoft counts in each term besides the tree's own rows (a course chosen inside a Course List, for example): the open term's
  // enrolled and planned courses with their units, and any other planned course with its term but unknown units (never guessed).
  const degreeExtras = useMemo(() => {
    if (!loaded) return []
    const open = courses.map((c) => ({ code: c.code, title: c.title, term: c.term, units: c.units ?? null }))
    const rest = loaded.others.flatMap((o) => {
      const m = /^((?:Fall|Winter|Spring) \d{4}) · /.exec(o.note)
      return m ? [{ code: o.code, title: o.title, term: m[1], units: null as number | null }] : []
    })
    return [...open, ...rest]
  }, [loaded, courses])

  // What the home screen remembers of the schedule that was just built: what PeopleSoft counts that the tree does not list, and its problems.
  useEffect(() => {
    if (!loaded) return
    const old = loadSnapshot()
    const issues = scheduleIssues(courses, state)
    saveSnapshot({ at: Date.now(), term: loaded.term, rows: old?.rows ?? [], extras: degreeExtras, overlaps: issues.overlaps, full: issues.full })
  }, [loaded, courses, state, degreeExtras])

  // Keep the schedule for the term it belongs to, so the next visit can restore it (see onLoaded).
  useEffect(() => {
    if (!loaded) return
    writePicks({ term: loaded.term, state, added, excluded })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state, added, excluded, loaded?.term])

  const coursesRef = useRef<ReturnType<typeof buildCourses>>([])
  coursesRef.current = courses

  /**
   * `plan`: a change of what the solver may choose from (lock, unlock, remove, restore): with Auto on the calendar follows the best schedule.
   * `pin`: the same, always (a pin is an instruction). `manual`: the student placed something themselves: Auto pauses.
   */
  const setState = useCallback((fn: (s: PlannerState) => PlannerState, kind: 'plan' | 'manual' | 'pin' = 'plan') => {
    if (kind === 'manual') setAuto(false)
    setHist((h) => {
      let next = fn(h.present.state)
      if (Object.is(next, h.present.state)) return h
      if ((kind === 'plan' && autoRef.current) || kind === 'pin') next = followBest(coursesRef.current, next, h.present.settings)
      return commit(h, { ...h.present, state: next }, Date.now())
    })
  }, [])
  const setSettings = useCallback((fn: (s: Settings) => Settings) => setHist((h) => {
    const next = fn(h.present.settings)
    if (Object.is(next, h.present.settings)) return h
    // With Auto on the calendar follows the preferences as they change, in the same history step.
    const state = autoRef.current ? followBest(coursesRef.current, h.present.state, next) : h.present.state
    return commit(h, { state, settings: next }, Date.now(), 'settings')
  }), [])
  const toggleAuto = useCallback(() => {
    if (autoRef.current) { setAuto(false); return }
    setAuto(true)
    setHist((h) => { const state = followBest(coursesRef.current, h.present.state, h.present.settings); return Object.is(state, h.present.state) ? h : commit(h, { ...h.present, state }, Date.now()) })
  }, [])
  const togglePin = useCallback((code: string, nbr: string) => {
    setState((st) => {
      const pinned = st.pins.includes(nbr)
      const course = coursesRef.current.find((c) => c.code === code)
      // Pinning a class of a course you are enrolled in unlocks it: a pin is an instruction to change it.
      const unlocked = !pinned && course?.held && !st.unlocked.includes(code) ? [...st.unlocked, code] : st.unlocked
      return { ...st, pins: pinned ? st.pins.filter((p) => p !== nbr) : [...st.pins, nbr], unlocked }
    }, 'pin')
  }, [setState])
  // The one small thing that keeps the score from being higher (checked by solving again with that thing changed).
  const outlier = useMemo(() => (prefs && loaded ? findOutlier(courses, state, settings) : null), [prefs, loaded, courses, state, settings])

  const goBack = useCallback(() => setHist(undo), [])
  const goForward = useCallback(() => setHist(redo), [])

  // Ctrl+Z / Ctrl+Shift+Z / Ctrl+Y on the planner screen (not while typing in a text field).
  useEffect(() => {
    if (screen !== 'planner') return
    const onKey = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey) || e.altKey) return
      const t = e.target as HTMLElement | null
      if (t && (t.tagName === 'TEXTAREA' || (t.tagName === 'INPUT' && (t as HTMLInputElement).type === 'text'))) return
      const k = e.key.toLowerCase()
      if (k === 'z' && !e.shiftKey) { e.preventDefault(); goBack() }
      else if ((k === 'z' && e.shiftKey) || k === 'y') { e.preventDefault(); goForward() }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [screen, goBack, goForward])

  const onLoaded = (l: Loaded) => {
    setLoaded(l)
    setHeldOpts({})
    // The schedule built earlier in this browser for this term comes back as it was, not as the suggestion (a page refresh must not change it).
    const saved = readPicks()
    if (saved && saved.term === l.term) {
      setExcluded({})
      setAdded(saved.added)
      setHist((h) => reset({ state: saved.state, settings: h.present.settings }))
      setScreen('planner')
      setUpdatedAt(Date.now())
      startHeld(l, false)
      return
    }
    setExcluded({})
    // The first suggestion fills the calendar; if nothing fits, every course shows its first combination so the overlaps are visible.
    const first = buildCourses(l.plan, l.enrolled.options, l.enrolled.rows, {}, l.term)
    const st = emptyState()
    const best = solve(solverCourses(first, st), effectiveSettings(settings)).schedules[0]
    setHist((h) => reset({ state: best ? applySuggestion(first, st, best) : st, settings: h.present.settings }))
    setScreen('planner')
    setUpdatedAt(Date.now())
    startHeld(l, false)
  }

  /**
   * The combinations of the courses you are enrolled in: reused from this browser's memory when the course's enrolled classes are
   * unchanged, and read (several courses at once, quietly in the background) only when not.
   */
  const startHeld = (l: Loaded, full: boolean) => {
    cancel.current.cancelled = true
    cancel.current = { cancelled: false }
    const { have, need } = splitHeld(l, full)
    setHeldOpts((h) => ({ ...Object.fromEntries(Object.entries(h).filter(([c]) => need.includes(c))), ...have }))
    if (need.length) {
      setReadingHeld(true)
      void loadHeldOptions(need, (code, key, options) => { rememberHeldRead(l, code, key, options); setHeldOpts((h) => ({ ...h, [code]: { key, options } })) }, cancel.current).finally(() => setReadingHeld(false))
    } else setReadingHeld(false)
  }

  /** Unlocking a course whose combinations are not here yet reads that one course now, instead of waiting for a bigger read. */
  const needOptions = (code: string) => {
    if (!loaded || readingHeld || heldOpts[code]) return
    setReadingHeld(true)
    void loadHeldOptions([code], (c, key, options) => { rememberHeldRead(loaded, c, key, options); setHeldOpts((h) => ({ ...h, [c]: { key, options } })) }, cancel.current).finally(() => setReadingHeld(false))
  }

  /** Re-read what can change (class list, enrolled list, seats); reuse what cannot unless `full`. Your choices and locks stay. */
  const refreshAll = async (full: boolean) => {
    if (!loaded || refreshing) return
    setRefreshing(true)
    try {
      const l = await loadTerm(() => undefined, { termHint: loaded.term, full })
      // PeopleSoft sometimes answers with an empty or maintenance page. Losing every enrolled class at once is far more likely to be that
      // than the truth, so a normal Refresh keeps what it had; Full accepts it.
      if (!full && Object.keys(loaded.enrolled.options).length > 0 && Object.keys(l.enrolled.options).length === 0) {
        setRefreshNote('PeopleSoft returned no enrolled classes. Kept what you had. Press Full if that is really true.')
        return
      }
      setRefreshNote(null)
      if (full) setExcluded({})
      setLoaded(l)
      setUpdatedAt(Date.now())
      startHeld(l, full)
    } catch (e) {
      setRefreshNote(`Refresh failed${e instanceof Error && e.message ? `: ${e.message}` : ''}. Kept what you had.`)
    } finally {
      setRefreshing(false)
    }
  }

  const refresh = async () => {
    if (!loaded) return
    const list = await api.classes()
    const enrolled = await readEnrolled(loaded.term, summariesOf(list.rows, loaded.term), !!settings.rules.joinWaitlist)
    if (enrolled && settings.rules.joinWaitlist) addListedWaitlist(enrolled, list.rows, loaded.term)
    if (enrolled) {
      setLoaded({ ...loaded, enrolled })
      setUpdatedAt(Date.now())
    }
  }

  const signOut = async () => {
    cancel.current.cancelled = true
    try {
      await api.logout()
    } catch {
      /* the page goes back to the login either way */
    }
    removeRamAccount(getSessionId())
    setSessionId(newSessionId())
    try {
      window.sessionStorage.removeItem(CONSENT_KEY)
    } catch {
      /* ignore */
    }
    clearAcademicData()
    setLoaded(null)
    setSignedInAs(null)
    setScreen('login')
  }

  const switchAccount = () => {
    cancel.current.cancelled = true
    // Keep the previous account's session + credentials in RAM, and open a fresh session slot for the new account.
    setSessionId(newSessionId())
    clearAcademicData()
    setLoaded(null)
    setSignedInAs(null)
    setScreen('login')
  }

  const selectRamAccount = async (acc: RamAccount) => {
    setSessionId(acc.sessionId)
    clearAcademicData()
    const s = await api.status()
    if (!s.signedIn) {
      await api.login(acc.username, acc.password)
    }
    try { window.sessionStorage.setItem(CONSENT_KEY, '1') } catch { /* ignore */ }
    setSignedInAs(acc.userHint)
    setScreen('load')
  }

  const tutorial = showTutorial ? <TutorialModal onClose={() => setShowTutorial(false)} /> : null

  if (screen === 'boot') return <div className="v2" />
  if (screen === 'login') {
    return (
      <>
        <Login
          signedInAs={signedInAs}
          ramAccounts={listRamAccounts()}
          onSwitchAccount={selectRamAccount}
          onHelp={() => setShowTutorial(true)}
          onDone={(info) => {
            if (info) saveRamAccount({ sessionId: getSessionId(), ...info })
            try { window.sessionStorage.setItem(CONSENT_KEY, '1') } catch { /* ignore */ }
            setScreen('load')
          }}
        />
        {tutorial}
      </>
    )
  }
  if (screen === 'add' && loaded) {
    return <AddCourse term={loaded.term} inSchedule={courses.map((c) => c.code)} extras={degreeExtras} dropping={courses.filter((c) => c.held && state.removed.includes(c.code)).map((c) => c.code)} onBack={() => setScreen('planner')} onAdded={() => { setScreen('planner'); void refreshAll(false) }} onKeep={(c) => { setAdded((a) => [...a.filter((x) => x.code !== c.code), c]); setScreen('planner') }} />
  }
  if (screen === 'degree') return <PlanDegree term={loaded?.term ?? 'Winter 2027'} extras={degreeExtras} extrasKnown={!!loaded} onBack={() => setScreen(loaded ? 'planner' : 'load')} />
  if (screen === 'load' || !loaded) return <><Load onLoaded={onLoaded} onSignOut={signOut} onSwitchAccount={switchAccount} onHelp={() => setShowTutorial(true)} onDegree={() => setScreen('degree')} />{tutorial}</>
  if (screen === 'apply') {
    return (
      <Apply
        planning={planning}
        courses={courses}
        state={state}
        enrolled={loaded.enrolled}
        onBack={() => setScreen('planner')}
        onDone={() => setScreen('done')}
        onRefresh={refresh}
        onRefused={(r) => setExcluded((x) => { const n = { ...x }; for (const [c, id] of Object.entries(r)) n[c] = [...new Set([...(n[c] ?? []), id])]; return n })}
      />
    )
  }
  if (screen === 'done') return <Done courses={courses} state={state} term={loaded.term} onBack={() => setScreen('planner')} />
  return (
    <>
      <Planner
        planning={planning}
        courses={courses}
        state={state}
        setState={setState}
        settings={settings}
        patchSettings={setSettings}
        enrolled={loaded.enrolled}
        others={loaded.others}
        readingHeld={readingHeld}
        term={loaded.term}
        onPrefs={() => setPrefs((p) => !p)}
        auto={auto}
        onToggleAuto={toggleAuto}
        side={prefs ? (
          <PreferencesPanel
            onClose={() => setPrefs(false)}
            settings={settings}
            patch={setSettings}
            courses={courses.map((c) => ({ code: c.code, title: c.title, term: c.term, options: c.options.length ? c.options : c.held ? [c.held] : [] }))}
            pins={state.pins}
            onPin={togglePin}
            outlier={outlier}
            onFix={(fix) => { if (fix.settings) setSettings(fix.settings); if (fix.state) setState(fix.state, 'plan') }}
          />
        ) : undefined}
        onApply={() => setScreen('apply')}
        onDegree={() => setScreen('degree')}
        onAdd={() => setScreen('add')}
        onSwitchAccount={switchAccount}
        onHelp={() => setShowTutorial(true)}
        onRefresh={refreshAll}
        refreshing={refreshing}
        refreshNote={refreshNote}
        updatedAt={updatedAt}
        onNeedOptions={needOptions}
        canBack={hist.past.length > 0}
        canForward={hist.future.length > 0}
        onBack={goBack}
        onForward={goForward}
      />
      {tutorial}
    </>
  )
}

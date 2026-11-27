import { useEffect, useMemo, useState } from 'react'
import { api, ApiError, type PlanProgress, type WriteStatus } from '../api'
import { enrolledOptions } from '../lock'
import { buildPlan } from '../plan'
import { blockerText, planSwapRounds } from '../swapRounds'
import { buildApplyRequest, wireProblems } from './applyRequest'
import { evalRuleTree, formatRuleTree, getCatalogRule } from './catalogRules'
import { completionFromView } from './compatible'
import { creditsFor, creditText } from './credits'
import type { DegreeView } from './degree'
import type { TermMoveDraft } from './FutureSemestersStrip'
import { optionStatusOf } from './guides'
import { asSchedule, courseOf, effective, isLocked, type AddPlan, type PCourse, type PlannerState } from './model'
import { WaitWidget } from './WaitWidget'

interface Props {
  term: string
  courses: PCourse[]
  state: PlannerState
  enrolled: { term: string; termRow: number; rows: any[]; options: Record<string, any>; unconfirmed: any[] }
  degreeView: DegreeView | null
  moveDrafts: Record<string, TermMoveDraft>
  planning: Record<string, string>
  onBack: () => void
  onDone: (progress: PlanProgress) => void
  onRefresh: () => void
}

export function Apply(props: Props) {
  const { term, courses, state, enrolled, degreeView, moveDrafts, planning, onBack, onDone } = props

  const [writeStatus, setWriteStatus] = useState<WriteStatus | null>(null)
  const [dropOn, setDropOn] = useState<Record<string, boolean>>({})
  const [overrideWaitlist, setOverrideWaitlist] = useState(false)
  const [overrideLimit, setOverrideLimit] = useState(false)
  const [phrase, setPhrase] = useState('')
  const [starting, setStarting] = useState(false)
  const [error, setError] = useState('')
  const [progress, setProgress] = useState<PlanProgress | null>(null)
  const [showRawById, setShowRawById] = useState<Record<string, boolean>>({})

  useEffect(() => {
    let alive = true
    api.writeStatus().then((ws) => { if (alive) setWriteStatus(ws) }).catch(() => { /* ignore */ })
    return () => { alive = false }
  }, [])

  // Build schedule & unified plan
  const activeCourses = useMemo(() => courses.filter((c) => !state.removed.includes(c.code)), [courses, state.removed])
  const schedule = useMemo(() => asSchedule(activeCourses, state), [activeCourses, state])
  const addPlans = useMemo(() => {
    const out: Record<string, AddPlan> = {}
    for (const c of activeCourses) {
      if (c.addPlan) out[c.code] = c.addPlan
    }
    return out
  }, [activeCourses])

  const liveSet = useMemo(() => new Set(courses.filter((c) => c.key || c.addPlan).map((c) => c.code)), [courses])
  const effectiveRemoved = useMemo(() => state.removed.filter((c) => dropOn[c] !== false), [state.removed, dropOn])
  const plan = useMemo(() => buildPlan(schedule, enrolled.options, enrolled.rows, {
    selected: activeCourses.map((c) => c.code),
    hidden: effectiveRemoved,
    courses: courses.map(courseOf),
    live: liveSet,
  }), [schedule, enrolled, activeCourses, effectiveRemoved, courses, liveSet])

  const allDrops = useMemo(() => plan.drops.filter((d) => dropOn[d.code] !== false), [plan.drops, dropOn])
  const heldOpts = useMemo(() => enrolledOptions(enrolled.rows), [enrolled.rows])
  const swapInputs = useMemo(() => plan.swaps.map((s) => ({
    code: s.course.code,
    now: s.now,
    next: s.next,
    seatsLeft: s.next.components[0]?.seatsOpen ?? null,
  })), [plan.swaps])
  const rp = useMemo(() => planSwapRounds(swapInputs, Object.values(heldOpts)), [swapInputs, heldOpts])

  const credits = useMemo(
    () => creditsFor(courses, { ...state, removed: effectiveRemoved }, term, planning, overrideWaitlist, overrideLimit),
    [courses, state, effectiveRemoved, term, planning, overrideWaitlist, overrideLimit],
  )

  const movePlans = useMemo(() => {
    if (!degreeView) return []
    const out: { course: string; year: number; index: number; termCode: string; credits: { term: string; limit: number; after: number; unknown: string[]; overrideLimit?: boolean } }[] = []
    for (const mv of Object.values(moveDrafts)) {
      const m = /^(Winter|Spring|Fall)\s+(\d{4})$/.exec(mv.toTerm.trim())
      if (!m) continue
      const termCode = `1${m[2].slice(2)}${m[1] === 'Winter' ? 1 : m[1] === 'Spring' ? 5 : 8}`
      let semRowIdx = 0
      const yr = degreeView.years.find((y) => y.index === mv.course.year)
      if (yr) {
        let idx = 0
        for (const s of yr.semesters) {
          for (const b of s.blocks) {
            if (b.kind === 'course' && b.course.code === mv.course.code) semRowIdx = idx
            idx++
          }
        }
      }
      out.push({
        course: mv.course.code,
        year: mv.course.year,
        index: semRowIdx,
        termCode,
        credits: {
          term: mv.toTerm,
          limit: creditsFor([], state, mv.toTerm).limit.max,
          after: credits.after,
          unknown: credits.unknown,
        },
      })
    }
    return out
  }, [moveDrafts, degreeView, state, credits])

  // Check if any picked section is Closed (browse-only)
  const closedPicks = useMemo(() => {
    return activeCourses.filter((c) => {
      if (isLocked(c, state)) return false
      const opt = effective(c, state)
      return opt && optionStatusOf(opt) === 'Closed'
    }).map((c) => c.code)
  }, [activeCourses, state])

  const waitlistPicks = useMemo(() => {
    return activeCourses.filter((c) => {
      if (c.held) return false
      const opt = effective(c, state)
      return opt && optionStatusOf(opt) === 'Waitlist'
    }).map((c) => c.code)
  }, [activeCourses, state])

  // Real prerequisite check using strictly completed courses (`completionFromView`, NEVER enrolled courses!)
  const prereqIssues = useMemo(() => {
    if (!degreeView) return []
    const { doneCodes, earnedCredits } = completionFromView(degreeView)
    const msgs: string[] = []
    for (const e of plan.enrols) {
      const cat = getCatalogRule(e.course.code)
      if (cat?.prerequisites && evalRuleTree(cat.prerequisites, doneCodes, earnedCredits) === false) {
        msgs.push(`${e.course.code} requires ${formatRuleTree(cat.prerequisites)}`)
      }
    }
    return msgs
  }, [degreeView, plan.enrols])

  const filteredBlockers = useMemo(() => {
    return plan.blockers.filter((b) => {
      if (b.includes('live row or key is missing')) {
        const code = b.split(':')[0]?.trim()
        if (code && addPlans[code]) return false
      }
      return true
    })
  }, [plan.blockers, addPlans])

  const actionCount = plan.enrols.length + plan.swaps.length + allDrops.length + movePlans.length
  const cannotConfirm =
    actionCount === 0 ||
    closedPicks.length > 0 ||
    credits.blocked ||
    filteredBlockers.length > 0 ||
    (writeStatus?.confirmMode === 'phrase' && phrase.trim().length === 0)

  // Poll `/api/write/plan/progress` once started
  useEffect(() => {
    if (!progress || progress.finished) return
    const timer = window.setInterval(async () => {
      try {
        const p = await api.planProgress()
        setProgress(p)
      } catch { /* keep polling */ }
    }, 900)
    return () => window.clearInterval(timer)
  }, [progress])

  const startApply = async () => {
    if (cannotConfirm || starting) return
    setStarting(true)
    setError('')
    try {
      const ws = writeStatus ?? (await api.writeStatus())
      if (!ws.armed) {
        await api.writeArm(ws.consent.statements.map((s) => s.id), ws.consent.version)
      }
      const req = buildApplyRequest(plan, rp, enrolled, allDrops, credits, addPlans, movePlans)
      const problems = wireProblems(req)
      if (problems.length > 0) {
        setError(problems[0])
        setStarting(false)
        return
      }
      await api.planApply(req)
      const p = await api.planProgress()
      setProgress(p)
    } catch (err) {
      setError(err instanceof ApiError ? err.message : 'Could not start enrollment.')
    } finally {
      setStarting(false)
    }
  }

  const allSucceeded = !!progress?.finished && progress.items.length > 0 && progress.items.every((it) => it.status === 'done')

  return (
    <div className="planie-root">
      <header className="topbar">
        <div className="topbar-left">
          <span className="brand-mark"><span className="brand-dot" aria-hidden="true" />Planie</span>
          <span className="term-pill">{term}</span>
          <span className={`credit-pill ${credits.over ? 'over' : ''}`}>
            {creditText(credits.after)} / {credits.limit.max} cr
          </span>
        </div>
        <div className="topbar-right">
          {!progress || progress.finished ? (
            <button type="button" className="btn btn-sm" onClick={onBack}>← Back to workspace</button>
          ) : null}
        </div>
      </header>

      <main className="apply-shell">
        {!progress ? (
          <>
            <div>
              <h2>Review &amp; Enroll</h2>
              <p className="login-sub">Check every change below. Nothing touches PeopleSoft until you press Confirm &amp; Enroll.</p>
            </div>

            {actionCount === 0 ? (
              <div className="calm-notice">No changes pending. Pick a different section, add a compatible course, or move a semester plan first.</div>
            ) : (
              <div className="action-card-grid">
                {allDrops.map((d) => (
                  <div key={`drop-${d.code}`} className="action-card kind-drop">
                    <span className="status-badge bad">Drop</span>
                    <strong>{d.code}</strong>
                    <span className="suggest-sub">Class #{d.classNbr}</span>
                    <button type="button" className="btn btn-sm" onClick={() => setDropOn((p) => ({ ...p, [d.code]: false }))}>Keep instead</button>
                  </div>
                ))}

                {plan.swaps.map((sw) => {
                  const isRedo = sw.redo
                  const msg = rp.waits[sw.course.code]?.[0]
                  return (
                    <div key={`swap-${sw.course.code}`} className="action-card kind-swap">
                      <span className={`status-badge ${isRedo ? 'warn' : 'accent'}`}>{isRedo ? 'Drop + Re-enrol' : 'Swap'}</span>
                      <strong>{sw.course.code}</strong>
                      <span className="suggest-sub">
                        Sec {sw.now.components[0]?.section ?? sw.now.optionNo} → Sec {sw.next.components[0]?.section ?? sw.next.optionNo}
                      </span>
                      {isRedo ? <span className="status-badge warn">Seat risk: lab-only change</span> : null}
                      {msg ? <span className="suggest-sub">{blockerText(msg)}</span> : null}
                    </div>
                  )
                })}

                {plan.enrols.map((e) => {
                  const isPlanAdd = !!addPlans[e.course.code]
                  const isWl = !!e.next.waitlist
                  return (
                    <div key={`enrol-${e.course.code}`} className={`action-card ${isPlanAdd ? 'kind-plan' : 'kind-enrol'}`}>
                      <span className={`status-badge ${isWl ? 'warn' : 'ok'}`}>
                        {isPlanAdd ? 'Plan + Enroll' : isWl ? 'Join Waitlist' : 'Enroll'}
                      </span>
                      <strong>{e.course.code}</strong>
                      <span className="suggest-sub">
                        {e.next.components.map((cm) => `${cm.kind} Sec ${cm.section.replace(/^sec\s*/i, '')}`).join(' + ')}
                      </span>
                      {isWl ? <span className="suggest-sub">Seat not guaranteed if full.</span> : null}
                    </div>
                  )
                })}

                {movePlans.map((m) => (
                  <div key={`move-${m.course}`} className="action-card kind-plan">
                    <span className="status-badge warn">Move term</span>
                    <strong>{m.course}</strong>
                    <span className="suggest-sub">Moved to {m.credits.term}</span>
                  </div>
                ))}
              </div>
            )}

            {closedPicks.length > 0 ? (
              <div className="status-badge bad">
                Full section selected for {closedPicks.join(', ')} — swap to an Open or Waitlist section before enrolling.
              </div>
            ) : null}

            {waitlistPicks.length > 0 ? (
              <div className="calm-notice">
                Waitlisted courses ({waitlistPicks.join(', ')}): PeopleSoft may enrol you later automatically without asking.
              </div>
            ) : null}

            {prereqIssues.length > 0 ? (
              <div className="status-badge warn">Prerequisite note: {prereqIssues.join('; ')}</div>
            ) : null}

            {filteredBlockers.map((b, i) => (
              <div key={i} className="status-badge bad">{b}</div>
            ))}

            {credits.over ? (
              <div className="setup-card">
                <div className="course-meta-row">
                  <span>
                    <strong>{creditText(credits.after)} credits</strong> exceeds normal limit ({credits.limit.max} for {term}).
                  </span>
                  {credits.waitlistOnly ? (
                    <label className="filter-chip">
                      <input type="checkbox" checked={overrideWaitlist} onChange={() => setOverrideWaitlist((x) => !x)} />
                      <span>Exclude waitlisted units from cap</span>
                    </label>
                  ) : null}
                  <label className="filter-chip">
                    <input type="checkbox" checked={overrideLimit} onChange={() => setOverrideLimit((x) => !x)} />
                    <span>Override credit limit (max 24)</span>
                  </label>
                </div>
              </div>
            ) : null}

            {writeStatus?.confirmMode === 'phrase' ? (
              <label className="field-label">
                Type <code>CONFIRM</code> to authorise live PeopleSoft changes
                <input type="text" value={phrase} onChange={(e) => setPhrase(e.target.value)} placeholder="CONFIRM" />
              </label>
            ) : null}

            {error ? <p className="status-badge bad" role="alert">{error}</p> : null}

            <div className="pill-row">
              <button type="button" className="btn btn-primary" disabled={cannotConfirm || starting} onClick={startApply}>
                {starting ? 'Starting…' : `Confirm & Enroll (${actionCount})`}
              </button>
              <button type="button" className="btn" onClick={onBack}>Back</button>
            </div>
          </>
        ) : (
          <section className="setup-shell">
            <div className="setup-card">
              <div className="suggestions-head">
                <h2>{progress.finished ? (allSucceeded ? 'All set!' : 'Finished with notes') : 'Applying changes…'}</h2>
                <span className="status-badge accent">{progress.phase}</span>
              </div>

              <div className="course-list">
                {progress.items.map((it) => {
                  const hasDetails = it.status === 'error' && Boolean(it.note)
                  const rawOpen = !!showRawById[it.id]
                  return (
                    <div key={it.id} className="course-card">
                      <div className="course-card-top">
                        <strong>{it.course} · {it.kind}</strong>
                        <span className={`status-badge ${it.status === 'done' ? 'ok' : it.status === 'error' ? 'bad' : 'accent'}`}>
                          {it.status}
                        </span>
                      </div>
                      {it.status === 'done' && it.note ? <p className="suggest-sub">{it.note}</p> : null}
                      {hasDetails ? (
                        <div>
                          <button
                            type="button"
                            className="btn btn-sm"
                            onClick={() => setShowRawById((p) => ({ ...p, [it.id]: !p[it.id] }))}
                          >
                            {rawOpen ? 'Hide PeopleSoft message' : 'Show PeopleSoft message'}
                          </button>
                          {rawOpen ? <p className="calm-notice">{it.note}</p> : null}
                        </div>
                      ) : null}
                    </div>
                  )
                })}
              </div>

              {progress.error ? <p className="status-badge bad">{progress.error}</p> : null}

              {progress.finished ? (
                <div className="pill-row">
                  {allSucceeded ? (
                    <button type="button" className="btn btn-primary" onClick={() => onDone(progress)}>
                      See confirmed schedule →
                    </button>
                  ) : (
                    <button type="button" className="btn btn-primary" onClick={onBack}>
                      Return to workspace
                    </button>
                  )}
                </div>
              ) : null}
            </div>

            <WaitWidget title={progress.finished ? 'Want to finish your round?' : 'Sending safely to PeopleSoft…'} />
          </section>
        )}
      </main>
    </div>
  )
}

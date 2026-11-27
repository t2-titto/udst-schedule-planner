import { useEffect, useMemo, useRef, useState } from 'react'
import { getCatalogRule } from './catalogRules'
import { completionFromView, evaluateCoursePrereqs, inferExemptedCourses } from './compatible'
import type { CompatibleStatus } from './CourseSidebar'
import { creditText, limitFor } from './credits'
import type { DegreeCourse, DegreeSemester, DegreeView } from './degree'
import type { TermMoveDraft } from './FutureSemestersStrip'
import { normCode } from './prereq'
import { tileClassFor } from './SuggestionsStrip'

interface Props {
  open: boolean
  onClose: () => void
  view: DegreeView | null
  openTerm: string
  scheduledCodes?: Set<string>
  classStatuses?: Record<string, { term: string; status: string }>
  compatStatus?: Record<string, CompatibleStatus>
  groupChoices?: Record<string, DegreeCourse[]>
  loadingGroups?: Record<string, boolean>
  onLoadGroupChoices?: (year: number, groupRow: number) => void
  onCheckTermOpen?: (courses: DegreeCourse[]) => void
  checkingTerm?: boolean
  addingCodes?: string[]
  moves: Record<string, TermMoveDraft>
  onMoveCourse: (course: DegreeCourse, toTerm: string) => void
  onCancelMove: (code: string) => void
  onQuickAddCourse?: (code: string, fromTerm: string) => void
  onInspectCourse?: (code: string) => void
  exempted?: ReadonlyMap<string, string>
  retakeTerms?: Record<string, string>
  onRetakeCourse?: (course: DegreeCourse, toTerm: string) => void
  onCancelRetake?: (code: string) => void
  tileMap?: ReadonlyMap<string, string>
}

function semesterTermLabel(s: DegreeSemester): string {
  const withPlan = s.blocks.find((b) => b.kind === 'course' && (b.course.planningTerm || b.course.term))
  if (withPlan && withPlan.kind === 'course') {
    return withPlan.course.planningTerm ?? withPlan.course.term ?? s.name
  }
  return s.name
}

function collectTermOptions(view: DegreeView | null, openTerm: string): string[] {
  const terms = new Set<string>([openTerm])
  if (!view) return [...terms]
  for (const y of view.years) {
    for (const s of y.semesters) {
      for (const b of s.blocks) {
        if (b.kind === 'course') {
          if (b.course.planningTerm) terms.add(b.course.planningTerm)
          if (b.course.term) terms.add(b.course.term)
        }
      }
    }
  }
  return [...terms].filter(Boolean)
}

function computeCreditsByTerm(view: DegreeView | null, moves: Record<string, TermMoveDraft>, termChoices: string[]): Map<string, number> {
  const map = new Map<string, number>()
  for (const t of termChoices) map.set(t, 0)
  if (!view) return map
  for (const y of view.years) {
    for (const s of y.semesters) {
      for (const b of s.blocks) {
        if (b.kind !== 'course' || b.course.dropping) continue
        const target = moves[b.course.code]?.toTerm ?? b.course.term
        if (!target) continue
        map.set(target, (map.get(target) ?? 0) + (b.course.units ?? 0))
      }
    }
  }
  for (const ex of view.extras) {
    if (ex.dropping || !ex.term) continue
    map.set(ex.term, (map.get(ex.term) ?? 0) + (ex.units ?? 0))
  }
  return map
}

export function FuturePlanDrawer(props: Props) {
  const {
    open,
    onClose,
    view,
    openTerm,
    scheduledCodes,
    classStatuses = {},
    compatStatus = {},
    groupChoices = {},
    loadingGroups = {},
    onLoadGroupChoices,
    onCheckTermOpen,
    checkingTerm,
    addingCodes = [],
    moves,
    onMoveCourse,
    onCancelMove,
    onQuickAddCourse,
    exempted,
    retakeTerms = {},
    onRetakeCourse,
    onCancelRetake,
    tileMap,
  } = props

  const [activeSemName, setActiveSemName] = useState<string | null>(null)
  const [retakeMenuCode, setRetakeMenuCode] = useState<string | null>(null)
  const activePillRef = useRef<HTMLButtonElement | null>(null)
  const requestedGroupsRef = useRef<Set<string>>(new Set())
  const requestedCheckSemsRef = useRef<Set<string>>(new Set())

  const semesters: DegreeSemester[] = useMemo(() => {
    if (!view) return []
    return view.years.flatMap((y) => y.semesters)
  }, [view])

  const termChoices = useMemo(() => collectTermOptions(view, openTerm), [view, openTerm])
  const termCreditsMap = useMemo(() => computeCreditsByTerm(view, moves, termChoices), [view, moves, termChoices])
  const { doneCodes: rawDoneCodes, earnedCredits } = useMemo(
    () => (view ? completionFromView(view, openTerm) : { doneCodes: new Set<string>(), earnedCredits: 0 }),
    [view, openTerm],
  )
  const exemptedMap = useMemo(
    () => exempted ?? inferExemptedCourses(rawDoneCodes, scheduledCodes),
    [exempted, rawDoneCodes, scheduledCodes],
  )
  const doneCodes = useMemo(
    () => new Set<string>([...rawDoneCodes, ...exemptedMap.keys()]),
    [rawDoneCodes, exemptedMap],
  )

  const currentSem = useMemo(() => {
    if (!semesters.length) return null
    return (
      semesters.find((s) => s.name === activeSemName) ??
      semesters.find((s) => s.blocks.some((b) => b.kind === 'course' && b.course.status !== 'completed')) ??
      semesters[0]
    )
  }, [semesters, activeSemName])

  useEffect(() => {
    if (!open) return
    const t = window.setTimeout(() => {
      activePillRef.current?.scrollIntoView({ inline: 'center', block: 'nearest' })
    }, 30)
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => {
      window.clearTimeout(t)
      window.removeEventListener('keydown', onKey)
    }
  }, [open, onClose, currentSem?.name])

  // Reset per-view request tracking cleanly when switching accounts / degree data
  useEffect(() => {
    requestedGroupsRef.current.clear()
    requestedCheckSemsRef.current.clear()
  }, [view])

  // Auto-load CourseGroup electives AND automatically check open-term availability in background when active semester opens
  useEffect(() => {
    if (!open || !currentSem) return
    if (onLoadGroupChoices) {
      for (const b of currentSem.blocks) {
        if (b.kind === 'group' && b.group.hasSelect) {
          const key = `${b.group.year}:${b.group.row}`
          if (!groupChoices[key] && !loadingGroups[key] && !requestedGroupsRef.current.has(key)) {
            requestedGroupsRef.current.add(key)
            onLoadGroupChoices(b.group.year, b.group.row)
          }
        }
      }
    }
    if (onCheckTermOpen && !checkingTerm && !requestedCheckSemsRef.current.has(currentSem.name)) {
      const unchecked = currentSem.blocks
        .filter((b): b is { kind: 'course'; course: DegreeCourse } => b.kind === 'course')
        .map((b) => b.course)
        .filter((c) => !compatStatus[c.code]?.checked && !compatStatus[normCode(c.code)]?.checked && !compatStatus[c.code]?.loading)
      if (unchecked.length > 0) {
        requestedCheckSemsRef.current.add(currentSem.name)
        onCheckTermOpen(unchecked)
      }
    }
  }, [open, currentSem, groupChoices, loadingGroups, onLoadGroupChoices, onCheckTermOpen, checkingTerm, compatStatus])

  if (!open) return null

  const moveCount = Object.keys(moves).length
  const semCoursesAll: DegreeCourse[] = currentSem
    ? currentSem.blocks.filter((b): b is { kind: 'course'; course: DegreeCourse } => b.kind === 'course').map((b) => b.course)
    : []
  const checkingAnyInSem = checkingTerm || semCoursesAll.some((c) => compatStatus[c.code]?.loading || compatStatus[normCode(c.code)]?.loading)

  const renderOpenBadge = (code: string) => {
    const st = compatStatus[code] ?? compatStatus[normCode(code)]
    const ps = classStatuses[code] ?? classStatuses[normCode(code)]
    if (st?.loading) return <span className="status-badge">Checking {openTerm}…</span>
    if (st?.checked) {
      if (st.notOffered) return <span className="status-badge warn">Not offered in {openTerm}</span>
      if (st.openCount > 0) return <span className="status-badge ok">{st.openCount} open · {openTerm}</span>
      if (st.waitlistCount > 0) return <span className="status-badge warn">{st.waitlistCount} waitlist · {openTerm}</span>
      return <span className="status-badge bad">Full in {openTerm}</span>
    }
    if (ps?.term === openTerm) {
      if (/^enrolled$/i.test(ps.status)) return <span className="status-badge ok">Enrolled</span>
      if (/^waitlisted$/i.test(ps.status)) return <span className="status-badge warn">Waitlisted</span>
      if (/^classes available$/i.test(ps.status)) return <span className="status-badge ok">Open in {openTerm}</span>
      if (/no classes/i.test(ps.status)) return <span className="status-badge warn">Not offered · {openTerm}</span>
    }
    return null
  }

  const canQuickAdd = (code: string) => {
    if (!onQuickAddCourse || scheduledCodes?.has(code)) return false
    const st = compatStatus[code] ?? compatStatus[normCode(code)]
    if (st?.checked && st.notOffered) return false
    const ps = classStatuses[code] ?? classStatuses[normCode(code)]
    return (st?.checked && (st.openCount > 0 || st.waitlistCount > 0)) || (ps?.term === openTerm && /classes available/i.test(ps.status))
  }

  return (
    <div className="drawer-backdrop" role="dialog" aria-modal="true" aria-label="Future Semesters & Degree Plan" onClick={onClose}>
      <aside className="drawer-panel" onClick={(e) => e.stopPropagation()}>
        <div className="sheet-head">
          <div>
            <h3>Future Semesters &amp; Degree Plan</h3>
            <p className="suggest-sub">
              {moveCount > 0
                ? `${moveCount} term move${moveCount === 1 ? '' : 's'} queued for Review & Enroll`
                : 'Credits, prerequisites, unlocks, electives & open sections inline'}
            </p>
          </div>
          <button type="button" className="btn btn-sm" onClick={onClose}>Done</button>
        </div>

        {!view || semesters.length === 0 ? (
          <p className="calm-notice">No degree plan rows loaded yet.</p>
        ) : (
          <>
            <div className="semester-pills-row">
              {semesters.map((sem) => {
                const semCourses = sem.blocks.filter((b): b is { kind: 'course'; course: DegreeCourse } => b.kind === 'course').map((b) => b.course)
                const termLabel = semesterTermLabel(sem)
                const cr = semCourses.reduce((a, c) => a + (c.units ?? 0), 0)
                const lim = limitFor(termLabel).max
                const active = currentSem?.name === sem.name

                return (
                  <button
                    key={sem.name}
                    ref={active ? activePillRef : undefined}
                    type="button"
                    className={`sem-pill ${active ? 'active' : ''}`}
                    onClick={() => setActiveSemName(sem.name)}
                  >
                    <span className="sem-pill-title">{termLabel}</span>
                    <span className="sem-pill-meta num">{semCourses.length} courses · {creditText(cr)}/{lim} cr</span>
                  </button>
                )
              })}
            </div>

            {currentSem ? (
              <>
                {checkingAnyInSem ? (
                  <div className="course-meta-row">
                    <span className="suggest-sub">{semesterTermLabel(currentSem)}</span>
                    <span className="status-badge">
                      <span className="btn-spin" aria-hidden="true" /> Checking {openTerm}…
                    </span>
                  </div>
                ) : null}

                <div className="drawer-body">
                  {currentSem.blocks.map((b, i) => {
                    if (b.kind === 'group') {
                      const gKey = `${b.group.year}:${b.group.row}`
                      const choices = groupChoices[gKey]
                      const loading = !!loadingGroups[gKey]
                      return (
                        <div key={`g-${i}`} className="course-card">
                          <div className="course-card-top">
                            <div>
                              <div className="course-code">{b.group.name}</div>
                              <div className="course-title">
                                {choices ? `${choices.length} elective option${choices.length === 1 ? '' : 's'}` : 'Elective / Choice list'}
                              </div>
                            </div>
                            {b.group.hasSelect && !choices ? (
                              <span className="status-badge">
                                <span className="btn-spin" aria-hidden="true" /> Loading options…
                              </span>
                            ) : null}
                          </div>

                          {loading && !choices ? (
                            <p className="suggest-sub">Loading elective choices automatically…</p>
                          ) : choices && choices.length > 0 ? (
                            <div className="elective-choice-list">
                              {choices.map((ch) => {
                                const pr = evaluateCoursePrereqs(ch.code, doneCodes, earnedCredits)
                                const unlocks = getCatalogRule(ch.code)?.unlocks ?? []
                                const isAdding = addingCodes.includes(ch.code)
                                const inSched = !!scheduledCodes?.has(ch.code)
                                const openBadge = renderOpenBadge(ch.code)
                                return (
                                  <div key={ch.code} className="elective-choice-row">
                                    <div className="course-card-top">
                                      <div>
                                        <div className="course-code">
                                          <span className={`course-color-dot ${tileClassFor(ch.code, tileMap)}`} aria-hidden="true" />
                                          {ch.code}{ch.units != null ? ` · ${ch.units} cr` : ''}
                                        </div>
                                        <div className="course-title">{ch.title}</div>
                                      </div>
                                      {inSched ? (
                                        <span className="status-badge ok">In schedule</span>
                                      ) : onQuickAddCourse ? (
                                        <button
                                          type="button"
                                          className="btn btn-sm btn-primary"
                                          disabled={isAdding || !!(compatStatus[ch.code]?.notOffered)}
                                          onClick={() => onQuickAddCourse(ch.code, openTerm)}
                                        >
                                          {isAdding ? <><span className="btn-spin" aria-hidden="true" /> Adding…</> : `+ Add`}
                                        </button>
                                      ) : null}
                                    </div>
                                    <div className="pill-row">
                                      {openBadge}
                                      <span className={`status-badge ${pr.status === 'unmet' ? 'warn' : 'ok'}`}>
                                        {pr.status === 'met' && !pr.text ? 'No prereqs' : pr.status === 'met' ? `Met: ${pr.text}` : `Needs: ${pr.text}`}
                                      </span>
                                      {unlocks.length > 0 ? <span className="status-badge">Unlocks: {unlocks.join(', ')}</span> : null}
                                    </div>
                                  </div>
                                )
                              })}
                            </div>
                          ) : choices && choices.length === 0 ? (
                            <p className="suggest-sub">No elective choices returned for this group.</p>
                          ) : null}
                        </div>
                      )
                    }

                    const c = b.course
                    const nc = normCode(c.code)
                    const pendingMove = moves[c.code]
                    const retakeTerm = retakeTerms[c.code]
                    const exemptedBy = exemptedMap.get(nc)
                    const isExempt = !!exemptedBy && c.status !== 'completed' && !retakeTerm
                    const effectiveTerm = retakeTerm ?? (pendingMove ? pendingMove.toTerm : c.term)
                    const pr = evaluateCoursePrereqs(c.code, doneCodes, earnedCredits)
                    const unlocks = getCatalogRule(c.code)?.unlocks ?? []
                    const openBadge = renderOpenBadge(c.code)
                    const inSched = !!scheduledCodes?.has(c.code)
                    const isAdding = addingCodes.includes(c.code)
                    const menuOpen = retakeMenuCode === c.code

                    return (
                      <div
                        key={c.code}
                        className={`course-card ${inSched ? `has-tile ${tileClassFor(c.code, tileMap)}` : ''}`}
                        onContextMenu={(e) => {
                          if (!onRetakeCourse) return
                          e.preventDefault()
                          setRetakeMenuCode((cur) => (cur === c.code ? null : c.code))
                        }}
                      >
                        <div className="course-card-top">
                          <div>
                            <div className="course-code">
                              {inSched ? <span className="course-color-dot" aria-hidden="true" /> : null}
                              {c.code}{c.units != null ? ` · ${c.units} cr` : ''}
                            </div>
                            <div className="course-title">{c.title}</div>
                          </div>
                          <div className="pill-row">
                            <span className={`status-badge ${retakeTerm ? 'accent' : c.status === 'completed' || isExempt ? 'ok' : pendingMove ? 'accent' : ''}`}>
                              {retakeTerm
                                ? `Retake · ${retakeTerm}`
                                : c.status === 'completed'
                                  ? 'Done'
                                  : isExempt
                                    ? `Exempt (via ${exemptedBy})`
                                    : effectiveTerm ?? 'Unplanned'}
                            </span>
                            {!isExempt ? openBadge : null}
                          </div>
                        </div>

                        <div className="drawer-inline-meta">
                          <div>
                            <strong>Needs:</strong>{' '}
                            {pr.text ? (
                              <span className={pr.status === 'unmet' ? 'status-badge warn' : ''}>
                                {pr.text}{pr.status === 'met' ? ' (✓ met)' : pr.status === 'unmet' ? ' (unmet)' : ' (check)'}
                              </span>
                            ) : (
                              'None'
                            )}
                          </div>
                          <div>
                            <strong>Unlocks:</strong>{' '}
                            {unlocks.length > 0 ? unlocks.join(', ') : 'None in catalog'}
                          </div>
                        </div>

                        <div className="course-card-actions">
                          {retakeTerm && onCancelRetake ? (
                            <button type="button" className="btn btn-sm" onClick={() => onCancelRetake(c.code)}>
                              Cancel retake
                            </button>
                          ) : null}
                          {pendingMove ? (
                            <button type="button" className="btn btn-sm" onClick={() => onCancelMove(c.code)}>Undo move</button>
                          ) : null}
                          {inSched ? (
                            <span className="status-badge ok">In schedule</span>
                          ) : !isExempt && (canQuickAdd(c.code) || (c.plannable && effectiveTerm === openTerm && onQuickAddCourse) || (retakeTerm === openTerm && onQuickAddCourse)) ? (
                            <button
                              type="button"
                              className="btn btn-sm btn-primary"
                              disabled={isAdding || !!(compatStatus[c.code]?.notOffered)}
                              onClick={() => onQuickAddCourse?.(c.code, openTerm)}
                            >
                              {isAdding ? <><span className="btn-spin" aria-hidden="true" /> Adding…</> : `+ Add to ${openTerm}`}
                            </button>
                          ) : null}
                          {c.plannable && c.status !== 'completed' && !isExempt ? (
                            <label className="term-select-wrap">
                              <span className="suggest-sub">Term</span>
                              <select
                                aria-label={`Move ${c.code} to term`}
                                value={effectiveTerm ?? ''}
                                onChange={(e) => { if (e.target.value) onMoveCourse(c, e.target.value) }}
                              >
                                {termChoices.map((t) => {
                                  const cr = termCreditsMap.get(t) ?? 0
                                  const lim = limitFor(t).max
                                  return (
                                    <option key={t} value={t}>
                                      {t} ({creditText(cr)}/{lim} cr)
                                    </option>
                                  )
                                })}
                              </select>
                            </label>
                          ) : null}
                          {(c.status === 'completed' || isExempt) && onRetakeCourse ? (
                            <button
                              type="button"
                              className="btn btn-sm"
                              onClick={() => setRetakeMenuCode((cur) => (cur === c.code ? null : c.code))}
                            >
                              Retake…
                            </button>
                          ) : null}
                        </div>

                        {menuOpen && onRetakeCourse ? (
                          <div className="retake-menu" role="menu" aria-label={`Retake semester for ${c.code}`}>
                            <div className="retake-menu-head">
                              <span className="suggest-sub">Retake {c.code} in:</span>
                              <button type="button" className="btn btn-sm" onClick={() => setRetakeMenuCode(null)}>Close</button>
                            </div>
                            <div className="pill-row">
                              {termChoices.map((t) => (
                                <button
                                  key={t}
                                  type="button"
                                  role="menuitem"
                                  className={`btn btn-sm ${t === openTerm ? 'btn-primary' : ''}`}
                                  onClick={() => {
                                    onRetakeCourse(c, t)
                                    setRetakeMenuCode(null)
                                  }}
                                >
                                  {t}
                                </button>
                              ))}
                            </div>
                          </div>
                        ) : null}
                      </div>
                    )
                  })}
                </div>
              </>
            ) : null}
          </>
        )}
      </aside>
    </div>
  )
}

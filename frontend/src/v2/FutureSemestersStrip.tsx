import { useMemo, useState } from 'react'
import type { DegreeCourse, DegreeSemester, DegreeView } from './degree'
import { creditsFor, creditText, limitFor } from './credits'
import type { PCourse, PlannerState } from './model'

export interface TermMoveDraft {
  course: DegreeCourse
  fromTerm: string | null
  toTerm: string
}

interface Props {
  view: DegreeView | null
  openTerm: string
  courses: PCourse[]
  state: PlannerState
  moves: Record<string, TermMoveDraft>
  onMoveCourse: (course: DegreeCourse, toTerm: string) => void
  onCancelMove: (code: string) => void
  onQuickAddCourse?: (code: string, fromTerm: string) => void
  onInspectCourse: (code: string) => void
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

export function FutureSemestersStrip(props: Props) {
  const { view, openTerm, moves, onMoveCourse, onCancelMove, onQuickAddCourse, onInspectCourse } = props
  const [expanded, setExpanded] = useState(false)
  const [activeSemName, setActiveSemName] = useState<string | null>(null)
  const [movingCourse, setMovingCourse] = useState<DegreeCourse | null>(null)

  const semesters: DegreeSemester[] = useMemo(() => {
    if (!view) return []
    return view.years.flatMap((y) => y.semesters)
  }, [view])

  const termChoices = useMemo(() => collectTermOptions(view, openTerm), [view, openTerm])

  const currentSem = useMemo(() => {
    if (!semesters.length) return null
    return semesters.find((s) => s.name === activeSemName) ?? semesters.find((s) => s.blocks.some((b) => b.kind === 'course' && b.course.status !== 'completed')) ?? semesters[0]
  }, [semesters, activeSemName])

  const moveCount = Object.keys(moves).length
  if (!view || semesters.length === 0) return null

  return (
    <section className="future-strip" aria-label="Future semesters timeline">
      <div className="future-header">
        <div className="pill-row">
          <h3>Future Semesters &amp; Degree Plan</h3>
          {moveCount > 0 ? <span className="status-badge accent">{moveCount} pending move{moveCount === 1 ? '' : 's'}</span> : null}
        </div>
        <button type="button" className="btn btn-sm" onClick={() => setExpanded((x) => !x)}>
          {expanded ? 'Collapse plan' : 'Expand future semesters'}
        </button>
      </div>

      <div className="semester-pills-row">
        {semesters.map((sem) => {
          const semCourses = sem.blocks.filter((b): b is { kind: 'course'; course: DegreeCourse } => b.kind === 'course').map((b) => b.course)
          const termLabel = semesterTermLabel(sem)
          const cr = semCourses.reduce((a, c) => a + (c.units ?? 0), 0)
          const lim = limitFor(termLabel).max
          const active = expanded && currentSem?.name === sem.name

          return (
            <button
              key={sem.name}
              type="button"
              className={`sem-pill ${active ? 'active' : ''}`}
              onClick={() => { setActiveSemName(sem.name); setExpanded(true) }}
            >
              <span className="sem-pill-title">{termLabel}</span>
              <span className="sem-pill-meta num">{semCourses.length} courses · {creditText(cr)} / {lim} cr</span>
            </button>
          )
        })}
      </div>

      {expanded && currentSem ? (
        <div className="future-sem-grid">
          {currentSem.blocks.map((b, i) => {
            if (b.kind === 'group') {
              return (
                <div key={`g-${i}`} className="course-card">
                  <div className="course-code">{b.group.name}</div>
                  <div className="course-title">Requirement group · pick from Ready to Add</div>
                </div>
              )
            }
            const c = b.course
            const pendingMove = moves[c.code]
            const effectiveTerm = pendingMove ? pendingMove.toTerm : c.term
            return (
              <div key={c.code} className="course-card">
                <div className="course-card-top">
                  <div>
                    <div className="course-code">{c.code}{c.units != null ? ` · ${c.units} cr` : ''}</div>
                    <div className="course-title">{c.title}</div>
                  </div>
                  <button type="button" className="icon-btn" aria-label={`Inspect ${c.code}`} onClick={() => onInspectCourse(c.code)}>
                    ⓘ
                  </button>
                </div>

                <div className="course-meta-row">
                  <div className="pill-row">
                    <span className={`status-badge ${c.status === 'completed' ? 'ok' : pendingMove ? 'accent' : ''}`}>
                      {c.status === 'completed' ? 'Done' : effectiveTerm ?? 'Unplanned'}
                    </span>
                    {pendingMove ? (
                      <button type="button" className="btn btn-sm" onClick={() => onCancelMove(c.code)}>Undo move</button>
                    ) : null}
                  </div>
                  {c.plannable && c.status !== 'completed' ? (
                    <div className="pill-row">
                      {effectiveTerm === openTerm && onQuickAddCourse ? (
                        <button type="button" className="btn btn-sm" onClick={() => onQuickAddCourse(c.code, openTerm)}>
                          Schedule
                        </button>
                      ) : null}
                      <button type="button" className="btn btn-sm" onClick={() => setMovingCourse(c)}>
                        Move term
                      </button>
                    </div>
                  ) : null}
                </div>
              </div>
            )
          })}
        </div>
      ) : null}

      {/* Touch-friendly destination semester picker */}
      {movingCourse ? (
        <div className="sheet-backdrop" role="dialog" aria-modal="true" aria-label={`Move ${movingCourse.code}`} onClick={() => setMovingCourse(null)}>
          <div className="sheet-card" onClick={(e) => e.stopPropagation()}>
            <div className="sheet-head">
              <div>
                <h3>Move {movingCourse.code}</h3>
                <p className="suggest-sub">Queued together when you press Review &amp; Enroll.</p>
              </div>
              <button type="button" className="btn btn-sm" onClick={() => setMovingCourse(null)}>Close</button>
            </div>
            <div className="pill-row">
              {termChoices.map((t) => (
                <button
                  key={t}
                  type="button"
                  className={`choice-pill ${(moves[movingCourse.code]?.toTerm ?? movingCourse.term) === t ? 'active' : ''}`}
                  onClick={() => {
                    onMoveCourse(movingCourse, t)
                    setMovingCourse(null)
                  }}
                >
                  {t}
                </button>
              ))}
            </div>
          </div>
        </div>
      ) : null}
    </section>
  )
}

export { creditsFor }

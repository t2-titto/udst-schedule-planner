import { useDeferredValue, useEffect, useMemo, useState, type ReactNode } from 'react'
import { fmtCompact } from '../engine/normalize'
import { activeDays, scoreSelection, solve, type Schedule } from '../engine/solver'
import { DAY_SHORT, NEUTRAL, instructorKey, type ClassOption, type Settings } from '../engine/types'
import type { EnrolledState } from '../enrolled'
import { codeOf } from '../lock'
import { MiniWeek } from './MiniWeek'
import { buildPlan } from '../plan'
import { Calendar } from './Calendar'
import { suggestFixes } from './fixes'
import { redoSuggestions } from './redoSuggest'
import { CreditPanel } from './CreditPanel'
import { creditsFor, creditText } from './credits'
import { guidesFor, type Guide } from './guides'
import { CenteredSlider } from './CenteredSlider'
import type { OtherRow } from './data'
import { effectiveSettings, freeDaysMatter } from './scale'
import { hasClass, solverCourses, applySuggestion, asSchedule, blocksOf, courseOf, effective, fixesFor, isLocked, overlapsOf, redoChange, type PCourse, type PlannerState } from './model'

interface Props {
  courses: PCourse[]
  state: PlannerState
  setState: (fn: (s: PlannerState) => PlannerState, kind?: 'plan' | 'manual' | 'pin') => void
  settings: Settings
  patchSettings: (fn: (s: Settings) => Settings) => void
  enrolled: EnrolledState
  others: OtherRow[]
  /** Enrolled courses whose other combinations are still being read. */
  readingHeld: boolean
  term: string
  onPrefs: () => void
  /** The docked preferences panel, when open. */
  side?: ReactNode
  onApply: () => void
  auto: boolean
  onToggleAuto: () => void
  /** Course code -> the term it belongs to in the degree's order (see `planningMap`). */
  planning: Record<string, string>
  onDegree: () => void
  onAdd: () => void
  onSwitchAccount?: () => void
  onHelp?: () => void
  onRefresh: (full: boolean) => void
  refreshing: boolean
  refreshNote: string | null
  updatedAt: number | null
  /** An unlocked course needs its other combinations now. */
  onNeedOptions: (code: string) => void
  canBack: boolean
  canForward: boolean
  onBack: () => void
  onForward: () => void
}

/** "just now", "3 min ago", "2 h ago". */
export function ago(at: number, now: number): string {
  const s = Math.max(0, Math.round((now - at) / 1000))
  if (s < 45) return 'just now'
  const m = Math.round(s / 60)
  return m < 60 ? `${m} min ago` : `${Math.round(m / 60)} h ago`
}

const when = (o: ClassOption) =>
  [...new Set(o.components.flatMap((c) => c.meetings.map((m) => `${DAY_SHORT[m.day]} ${fmtCompact(m.start, m.end)}`)))].join(', ')
const teachers = (o: ClassOption) => [...new Set(o.components.flatMap((c) => c.instructors))].join(', ')
const seatsOf = (o: ClassOption) =>
  o.status.toLowerCase() === 'closed' && o.components.every((c) => c.seatsOpen === null)
    ? 'Closed'
    : o.components.map((c) => (c.seatsOpen === null ? '·' : String(c.seatsOpen))).join(' / ')

export function Planner({ courses, state, setState, settings, patchSettings, enrolled, others, readingHeld, term, onPrefs, side, onApply, auto, onToggleAuto, planning, onDegree, onAdd, onSwitchAccount, onHelp, onRefresh, refreshing, refreshNote, updatedAt, onNeedOptions, canBack, canForward, onBack, onForward }: Props) {
  const [active, setActive] = useState<string | null>(null)
  const [showOthers, setShowOthers] = useState(false)
  const [preview, setPreview] = useState<Guide | null>(null)
  const [showCredits, setShowCredits] = useState(false)
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const t = window.setInterval(() => setNow(Date.now()), 30000)
    return () => window.clearInterval(t)
  }, [])
  const deferred = useDeferredValue(settings)
  const colorOf = (code: string) => Math.max(0, courses.findIndex((c) => c.code === code)) % 8

  const result = useMemo(() => solve(solverCourses(courses, state), effectiveSettings(deferred)), [courses, state, deferred])
  const blocks = useMemo(() => blocksOf(courses, state), [courses, state])
  const overlaps = useMemo(() => overlapsOf(blocks), [blocks])
  const plan = useMemo(
    () => buildPlan(asSchedule(courses, state), enrolled.options, enrolled.rows, { selected: [], hidden: [], courses: [], live: new Set(courses.map((c) => c.code)) }),
    [courses, state, enrolled],
  )
  // The score of whatever is on the calendar now, with the same rules and week the ranked list uses.
  const score = useMemo(() => {
    const eff = effectiveSettings(deferred)
    const sc = solverCourses(courses, state)
    const picks = courses.flatMap((c) => { const o = effective(c, state); return o ? [{ course: courseOf(c), option: o }] : [] })
    return picks.length ? scoreSelection(picks, activeDays(sc.length ? sc : picks.map((p) => p.course)), eff) : null
  }, [courses, state, deferred])
  const changes = plan.swaps.length + plan.enrols.length + plan.drops.length
  const credits = useMemo(() => creditsFor(courses, state, term, planning), [courses, state, term, planning])
  const canApply = overlaps.length === 0 && plan.blockers.length === 0 && changes > 0 && !credits.blocked

  const suggestions = result.schedules
  // Schedules that exist only if you accept a drop and re-enrol; empty unless one really beats the ordinary best.
  const redo = useMemo(() => redoSuggestions(courses, state, deferred), [courses, state, deferred])
  const nothing = useMemo(() => (result.schedules.length ? { reasons: [], fixes: [] } : suggestFixes(courses, state, deferred)), [result, courses, state, deferred])
  const range: [number, number] = [8 * 60, 22 * 60]
  const days = [0, 1, 2, 3, 4]
  const activeCourse = courses.find((c) => c.code === active) ?? null

  const pick = (s: Schedule) => setState((st) => applySuggestion(courses, st, s), 'manual')
  // Choosing a combination by hand also drops the pins of that course that the chosen combination does not contain.
  const choose = (code: string, id: string) =>
    setState((st) => {
      const c = courses.find((x) => x.code === code)
      const o = c?.options.find((x) => x.id === id)
      const pins = c && o ? st.pins.filter((p) => !c.options.some((x) => hasClass(x, p)) || hasClass(o, p)) : st.pins
      return { ...st, choice: { ...st.choice, [code]: id }, pins }
    }, 'manual')
  const remove = (code: string) => setState((st) => ({ ...st, removed: [...new Set([...st.removed, code])] }))
  const restore = (code: string) => setState((st) => ({ ...st, removed: st.removed.filter((c) => c !== code) }))
  const unlock = (code: string) => {
    setState((st) => ({ ...st, unlocked: [...new Set([...st.unlocked, code])] }))
    onNeedOptions(code)
  }
  const lock = (code: string) => setState((st) => ({ ...st, unlocked: st.unlocked.filter((c) => c !== code), choice: Object.fromEntries(Object.entries(st.choice).filter(([k]) => k !== code)) }))

  return (
    <div className="v2">
      <header className="v2-bar">
        <h1>Schedule planner</h1>
        <span className="v2-status">{term}{readingHeld ? ' · reading your courses’ other times…' : ''}{refreshing ? ' · refreshing…' : updatedAt ? ` · updated ${ago(updatedAt, now)}` : ''}</span>
        <span className="v2-spacer" />
        {score ? (
          <span className="v2-score" aria-live="polite" title={`Instructors ${score.instructor.toFixed(1)} · Time ${score.timing.toFixed(1)} · Free days ${score.freeDay.toFixed(1)} (0 to 10)`}>
            {preview && !preview.current ? (
              <>Score <strong>{preview.score.toFixed(1)}</strong><span className={`v2-score__delta${preview.score - score.total < 0 ? ' is-down' : ''}`}>{preview.score - score.total >= 0 ? '+' : ''}{(preview.score - score.total).toFixed(1)} if dropped here</span></>
            ) : (
              <>Score <strong>{score.total.toFixed(1)}</strong>
                <span className="v2-score__parts">Instructors {score.instructor.toFixed(1)} · Time {score.timing.toFixed(1)}{freeDaysMatter(deferred.weights.freeDay) ? ` · Free days ${score.freeDay.toFixed(1)}` : ''}</span></>
            )}
          </span>
        ) : null}
        <button type="button" className={`v2-meter${credits.over ? ' is-over' : credits.unknown.length ? ' is-unknown' : ''}`} aria-expanded={showCredits} onClick={() => setShowCredits((v) => !v)} title="Credits this term: your schedule, and what PeopleSoft will count against the limit">
          {creditText(credits.after)}{credits.unknown.length ? '+' : ''} / {credits.limit.max} credits
        </button>
        <button type="button" className="v2-linkbtn" onClick={onToggleAuto} aria-pressed={auto} title={auto ? 'The calendar follows your preferences as you change them. Placing a class yourself pauses this.' : 'Paused because you placed a class yourself. Click to rebuild the calendar from the best schedule and follow your preferences again.'}>{auto ? 'Auto: on' : 'Auto: paused, follow best'}</button>
        <button type="button" className="btn btn-quiet" disabled={refreshing} onClick={() => onRefresh(false)} title="Read your enrolled classes, the class list and seats again. Course times are reused if nothing changed.">{refreshing ? 'Refreshing…' : '↻ Refresh'}</button>
        <button type="button" className="v2-linkbtn" disabled={refreshing} onClick={() => onRefresh(true)} title="Read everything again, including every course's times and combinations (about 30 s)">Full</button>
        <button type="button" className="btn btn-quiet" disabled={!canBack} onClick={onBack} title="Undo (Ctrl+Z)" aria-label="Back, undo the last change">↶ Back</button>
        <button type="button" className="btn btn-quiet" disabled={!canForward} onClick={onForward} title="Redo (Ctrl+Shift+Z)" aria-label="Forward, redo">Forward ↷</button>
        <button type="button" className="btn btn-quiet" onClick={onAdd}>Add a course</button>
        <button type="button" className="btn btn-quiet" onClick={onDegree}>Degree</button>
        <button type="button" className="btn btn-quiet" onClick={onPrefs}>Preferences</button>
        {onHelp ? <button type="button" className="v2-linkbtn" onClick={onHelp}>Tutorial</button> : null}
        {onSwitchAccount ? <button type="button" className="v2-linkbtn" onClick={onSwitchAccount}>Switch account</button> : null}
        <button type="button" className="btn" disabled={!canApply} onClick={onApply} title={credits.over ? `Over the credit limit: ${creditText(credits.after)} / ${credits.limit.max}` : credits.unknown.length ? 'Some credits are unknown' : overlaps.length ? 'Fix the overlaps first' : changes === 0 ? 'Nothing to change' : plan.blockers[0]}>Apply</button>
      </header>
      {showCredits || credits.over ? <CreditPanel credits={credits} onRemoveEnrolled={remove} /> : null}


      {refreshNote ? <p className="v2-alert" role="status">{refreshNote}</p> : null}

      <div className="v2-planner" data-side={side ? '' : undefined}>
        <aside className="v2-courses" aria-label="Courses">
          <h2>Courses</h2>
          {courses.map((c) => {
            const removed = state.removed.includes(c.code)
            const locked = isLocked(c, state)
            const eff = effective(c, state)
            const movable = !removed && !locked && c.options.length > 0
            return (
              <div key={c.code} className={`v2-crow c${colorOf(c.code)}${locked ? ' is-locked' : ''}${removed ? ' is-removed' : ''}${active === c.code ? ' is-active' : ''}`}>
                <div className="top">
                  <input type="checkbox" aria-label={`Keep ${c.code} in the schedule`} checked={!removed} onChange={(e) => (e.target.checked ? restore(c.code) : remove(c.code))} />
                  <span className="swatch" aria-hidden="true" />
                  <button type="button" className="v2-linkbtn code" style={{ color: 'inherit' }} onClick={() => setActive(c.code)}>{c.code}<span className="ctitle">{c.title ? ` ${c.title}` : ''}</span></button>
                  {c.held ? (
                    locked ? (
                      <button type="button" className="v2-iconbtn" aria-label={`Unlock ${c.code}`} title="Unlock to change it" onClick={() => unlock(c.code)}>🔒</button>
                    ) : (
                      <button type="button" className="v2-iconbtn" aria-label={`Lock ${c.code} again`} title="Lock it back to what you have" onClick={() => lock(c.code)}>🔓</button>
                    )
                  ) : null}
                  <button type="button" className="v2-iconbtn" aria-label={`Remove ${c.code}`} title={c.held ? 'Remove (drops it when you apply)' : 'Remove from the schedule'} onClick={() => remove(c.code)}>✕</button>
                </div>
                <div className="sub">{c.held ? (locked ? 'Enrolled' : 'Enrolled · unlocked') : 'New'}</div>
                {movable ? (
                  <select aria-label={`Combination for ${c.code}`} value={eff?.id ?? ''} onChange={(e) => choose(c.code, e.target.value)}>
                    {c.options.map((o) => <option key={o.id} value={o.id}>{`${o.optionNo}: ${when(o)}${o.status.toLowerCase() === 'closed' && !c.held?.components.every((h) => o.components.some((x) => x.classNbr === h.classNbr)) ? ' (Closed)' : ''}${redoChange(c, o) ? ' (lab only: drop and enrol again)' : ''}`}</option>)}
                  </select>
                ) : null}
                {!removed && !locked && c.held && c.options.length === 0 ? <div className="sub">{readingHeld ? 'Reading its other times…' : 'Its other times could not be read.'}</div> : null}
                {!removed && !c.held && c.options.length === 0 ? <div className="sub">Its combination was hidden after PeopleSoft refused it (click Full to show again).</div> : null}
              </div>
            )
          })}
          {others.length ? (
            <>
              <button type="button" className="v2-linkbtn" onClick={() => setShowOthers((v) => !v)}>{showOthers ? 'Hide others' : 'Show others'}</button>
              {showOthers ? <ul className="v2-muted" style={{ margin: 0, paddingLeft: '1.1rem' }}>{others.map((o) => <li key={`${o.code}${o.note}`}>{o.code} · {o.note}</li>)}</ul> : null}
            </>
          ) : null}
          {enrolled.unconfirmed.length ? (
            <p className="v2-muted v2-unconfirmed" style={{ fontSize: 'var(--text-xs)', margin: 0 }}>
              Ignored (Plan and Enrol is the source of truth): {enrolled.unconfirmed.map((r) => `${codeOf(r.course)} ${r.component.toLowerCase()} ${r.class_nbr}`).join(', ')}. PeopleSoft's Drop page lists {enrolled.unconfirmed.length > 1 ? 'them' : 'it'}; the Plan page does not.
            </p>
          ) : null}
        </aside>

        <main className="v2-main">
          {suggestions.length ? (
            <div className="v2-thumbs" role="list" aria-label="Ranked suggestions">
              {suggestions.map((s, i) => (
                <button key={i} type="button" role="listitem" className="v2-thumb" aria-pressed={s.picks.every((p) => effective(courses.find((c) => c.code === p.course.code)!, state)?.id === p.option.id)} onClick={() => pick(s)}>
                  <span className="t"><span>#{i + 1}</span><span className="num">{s.scores.total.toFixed(1)}</span></span>
                  <MiniWeek schedule={s} days={days} range={range} colorOf={colorOf} />
                </button>
              ))}
            </div>
          ) : (
            <div className="v2-nosug" role="status">
              <strong>No suggestions</strong>
              {courses.length === 0 ? (
                <p className="v2-muted">No courses for this term were found in your plan or enrolment.</p>
              ) : courses.every((c) => state.removed.includes(c.code)) ? (
                <p className="v2-muted">Every course is removed. Tick a course on the left to bring it back.</p>
              ) : (
                <>
                  <ul>{nothing.reasons.map((r) => <li key={r}>{r}</li>)}{nothing.reasons.length === 0 ? <li>Locked classes and the other courses leave no conflict-free combination.</li> : null}</ul>
                  <div className="btnrow">
                    {nothing.fixes.map((f) => (
                      <button key={f.label} type="button" className="btn btn-quiet btn-sm" onClick={() => { if (f.settings) patchSettings(f.settings); if (f.state) setState(f.state) }}>{f.label}</button>
                    ))}
                  </div>
                </>
              )}
            </div>
          )}

          {redo.length ? (
            <div className="v2-redo" role="group" aria-label="Suggestions that need a drop and re-enrol">
              <div className="v2-redo__head"><strong>↻ If you accept a drop and re-enrol</strong> <span>You would be dropped from the course and enrolled again, so you could lose the seat. Shown because they score higher.</span></div>
              <div className="v2-thumbs v2-thumbs--redo" role="list">
                {redo.map((r, i) => (
                  <button key={i} type="button" role="listitem" className="v2-thumb is-redo" title={`Needs a drop and re-enrol of ${r.redoCourses.join(', ')}`} onClick={() => pick(r.schedule)}>
                    <span className="t"><span>↻ {r.redoCourses.join(', ')}</span><span className="num">{r.schedule.scores.total.toFixed(1)}{r.gain > 0 && suggestions.length ? ` (+${r.gain.toFixed(1)})` : ''}</span></span>
                    <MiniWeek schedule={r.schedule} days={days} range={range} colorOf={colorOf} />
                  </button>
                ))}
              </div>
            </div>
          ) : null}

          <Calendar blocks={blocks} overlaps={overlaps} colorOf={colorOf} active={active} onPick={setActive} guidesFor={(b) => guidesFor(courses, state, b, settings)} onDropGuide={(b, g) => choose(b.courseCode, g.optionId)} onPreview={setPreview} baseScore={score?.total ?? null} />

          {overlaps.length ? (
            <div className="v2-overlaps" role="alert">
              {overlaps.map((o, i) => {
                const movers = [o.a.courseCode, o.b.courseCode].map((code) => courses.find((c) => c.code === code)!).filter((c) => !isLocked(c, state) && c.options.length > 0)
                const mover = movers[0]
                const fix = mover ? fixesFor(mover, courses.filter((c) => !state.removed.includes(c.code)), state, 1)[0] : undefined
                return (
                  <div key={i} className="v2-overlap">
                    <span>{o.a.courseCode} {o.a.kind.toLowerCase()} overlaps {o.b.courseCode} {o.b.kind.toLowerCase()}, {DAY_SHORT[o.day]} {fmtCompact(o.start, o.end)}</span>
                    {mover && fix ? <button type="button" className="btn" onClick={() => choose(mover.code, fix.id)}>Move {mover.code} to {when(fix)}</button> : <span>{movers.length ? 'No combination of it avoids this.' : 'Unlock one of them to move it.'}</span>}
                  </div>
                )
              })}
            </div>
          ) : null}
        </main>
        {side}
      </div>

      {activeCourse ? (
        <aside className="v2-panel" aria-label={`${activeCourse.code} combinations`}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-sm)' }}>
            <h3 style={{ flex: 1 }}>{activeCourse.code}</h3>
            <button type="button" className="v2-iconbtn" aria-label="Close" onClick={() => setActive(null)}>✕</button>
          </div>
          <div className="v2-muted">{activeCourse.title}</div>
          <div className="btnrow">
            {activeCourse.held ? (isLocked(activeCourse, state)
              ? <button type="button" className="btn btn-quiet btn-sm" onClick={() => unlock(activeCourse.code)}>🔒 Unlock</button>
              : <button type="button" className="btn btn-quiet btn-sm" onClick={() => lock(activeCourse.code)}>🔓 Lock</button>) : null}
            <button type="button" className="btn btn-quiet btn-sm" onClick={() => { remove(activeCourse.code); setActive(null) }}>✕ Remove</button>
          </div>
          {(() => {
            const names = [...new Set([...(activeCourse.held ? [activeCourse.held] : []), ...activeCourse.options].flatMap((o) => o.components.flatMap((c) => c.instructors)))]
            return names.length ? (
              <div className="v2-scores">
                <strong>Instructor scores</strong>
                {names.map((n) => {
                  const key = instructorKey(activeCourse.code, n)
                  return <CenteredSlider key={key} id={`sc-${key}`} label={n} value={(settings.instructorScores[key] ?? NEUTRAL) - NEUTRAL} onChange={(v) => patchSettings((st) => ({ ...st, instructorScores: { ...st.instructorScores, [key]: v + NEUTRAL } }))} />
                })}
              </div>
            ) : null
          })()}
          {isLocked(activeCourse, state)
            ? <p className="v2-muted">Locked to what you are enrolled in. Unlock it to see its other combinations.</p>
            : activeCourse.options.length === 0
              ? <p className="v2-muted">{readingHeld ? 'Reading its other times…' : 'Its other combinations could not be read.'}</p>
              : activeCourse.options.map((o) => {
                  const cur = effective(activeCourse, state)
                  const clash = overlapsOf([...blocksOf(courses.filter((c) => c.code !== activeCourse.code), state), ...o.components.flatMap((comp) => comp.meetings.map((m) => ({ courseCode: activeCourse.code, classNbr: comp.classNbr, kind: comp.kind, room: comp.room, instructors: comp.instructors, day: m.day, start: m.start, end: m.end, locked: false, held: false })))]).length > 0
                  return (
                    <button key={o.id} type="button" className={`v2-combo${redoChange(activeCourse, o) ? ' is-redo' : ''}`} aria-pressed={cur?.id === o.id} onClick={() => choose(activeCourse.code, o.id)}>
                      <strong>{o.optionNo}: {when(o)}</strong>
                      <span className="s">{teachers(o)} · seats {seatsOf(o)}{clash ? ' · overlaps something' : ''}{redoChange(activeCourse, o) ? ' · changes only the lab: needs a drop and a new enrolment (you could lose the seat)' : ''}</span>
                    </button>
                  )
                })}
        </aside>
      ) : null}
    </div>
  )
}

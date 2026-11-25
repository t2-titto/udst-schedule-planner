import { useMemo, useState } from 'react'
import type { DegreeCourse, DegreeView, WireDegree } from './degree'
import { creditText } from './credits'
import { termOrder } from './data'
import { academicYearLabel, academicYearOf, allowedTerms, changeCount, coursesOf, loadOf, movable, moveCourse, REVIEW_NOTE, suggestTerm, termLoads, termOf, termsOfAcademicYear, warningsFor, type Draft } from './draft'
import type { History } from './history'
import type { Prerequisite } from './prereq'
import { coreqMap, loadPrereqs } from './prereqCache'

interface Props {
  /** False until the schedule is loaded (the credit totals would miss Course-List courses). */
  canSave?: boolean
  /** Why Save is off, for the line under the draft bar: the schedule is not loaded, or PeopleSoft's current plan is still being read. */
  saveBlockedWhy?: 'schedule' | 'reading' | null
  view: DegreeView
  wire: WireDegree
  openTerm: string
  prereqs: Map<string, Prerequisite>
  coreqs?: Map<string, string[]>
  history: History<Draft>
  onChange: (next: Draft, tag?: string) => void
  onUndo: () => void
  onRedo: () => void
  onDiscard: () => void
  onSave: () => void
}

/** A board of terms for one academic year at a time. Moves only change the local draft; nothing is sent to PeopleSoft. */
export function PlanBoard({ canSave = true, saveBlockedWhy = null, view, wire, openTerm, prereqs, coreqs: coreqsProp, history, onChange, onUndo, onRedo, onDiscard, onSave }: Props) {
  const draft = history.present
  const courses = useMemo(() => coursesOf(view), [view])
  const optionsByRow = useMemo(() => new Map(wire.rows.filter((r) => r.term_choice).map((r) => [r.row, r.term_choice!.options.map((o) => ({ term: o.label, code: o.code }))])), [wire])
  const allowedOf = (c: DegreeCourse) => allowedTerms(c, optionsByRow.get(c.row) ?? [], openTerm)
  const years = useMemo(() => {
    const set = new Set<number>()
    for (const c of courses) {
      const y = c.term ? academicYearOf(c.term) : null
      if (y !== null) set.add(y)
      for (const t of allowedTerms(c, optionsByRow.get(c.row) ?? [], openTerm)) { const a = academicYearOf(t); if (a !== null) set.add(a) }
    }
    return [...set].sort((a, b) => a - b)
  }, [courses, optionsByRow, openTerm])
  const [year, setYear] = useState(() => academicYearOf(openTerm) ?? years[0] ?? 2026)
  const [dragRow, setDragRow] = useState<number | null>(null)
  const [overTerm, setOverTerm] = useState<string | null>(null)

  const loads = useMemo(() => termLoads(courses, draft), [courses, draft])
  const coreqs = useMemo(() => coreqsProp ?? coreqMap(loadPrereqs(), Date.now()), [coreqsProp, prereqs])
  const warnings = useMemo(() => warningsFor(courses, draft, prereqs, openTerm, coreqs), [courses, draft, prereqs, openTerm, coreqs])
  const unplanned = courses.filter((c) => termOf(c, draft) === null)
  const byRow = new Map(courses.map((c) => [c.row, c]))

  const move = (c: DegreeCourse, term: string) => onChange(moveCourse(draft, c, term, allowedOf(c)))
  const drop = (term: string) => {
    const c = dragRow === null ? undefined : byRow.get(dragRow)
    setDragRow(null)
    setOverTerm(null)
    if (c) move(c, term)
  }

  // For a term over its limit: offer to move one movable course (one the student just moved first) to the nearest term with the fewest credits.
  const suggestions = [...loads.values()].filter((l) => l.over).flatMap((l) => {
    const cands = l.courses.filter(movable)
    const pick = cands.find((c) => c.row in draft) ?? cands[cands.length - 1]
    if (!pick) return []
    const to = suggestTerm(pick, draft, courses, allowedOf(pick), l.term)
    return to ? [{ course: pick, from: l.term, to }] : []
  })

  const card = (c: DegreeCourse) => {
    const canMove = movable(c)
    const moved = c.row in draft
    const opts = allowedOf(c)
    return (
      <div
        key={c.row}
        className={`v2-coursecard is-${c.status}${moved ? ' is-moved' : ''}${dragRow === c.row ? ' is-dragging' : ''}`}
        draggable={canMove}
        onDragStart={(e) => { setDragRow(c.row); e.dataTransfer.effectAllowed = 'move'; e.dataTransfer.setData('text/plain', c.code) }}
        onDragEnd={() => { setDragRow(null); setOverTerm(null) }}
      >
        <div><strong>{c.code}</strong> {c.title}</div>
        <div className="v2-muted">{c.units === null ? 'units unknown' : `${creditText(c.units)} cr`} · {c.category || '—'}{canMove ? '' : c.status === 'completed' ? ' · completed' : ' · in progress'}{moved ? ' · moved in draft' : ''}</div>
        {canMove ? (
          <label className="v2-moveto">
            <span className="v2-muted">Move to</span>
            <select value={termOf(c, draft) ?? ''} onChange={(e) => e.target.value && move(c, e.target.value)} aria-label={`Move ${c.code} to`}>
              {termOf(c, draft) === null ? <option value="">Choose a term…</option> : null}
              {opts.map((t) => <option key={t} value={t}>{t}</option>)}
            </select>
          </label>
        ) : null}
      </div>
    )
  }

  return (
    <section className="v2-board" aria-label="Term board (draft)">
      <div className="v2-draftbar" role="status">
        <strong>Draft</strong> – {changeCount(draft) === 0 ? 'no changes yet.' : `${changeCount(draft)} course${changeCount(draft) === 1 ? '' : 's'} moved.`} Nothing is sent to PeopleSoft.
        <span className="v2-spacer" />
        <button type="button" className="btn btn-quiet btn-sm" disabled={history.past.length === 0} onClick={onUndo}>Back</button>
        <button type="button" className="btn btn-quiet btn-sm" disabled={history.future.length === 0} onClick={onRedo}>Forward</button>
        <button type="button" className="btn btn-quiet btn-sm" disabled={changeCount(draft) === 0} onClick={onDiscard}>Discard all</button>
        <button type="button" className="btn btn-sm" disabled={!canSave || changeCount(draft) === 0 || warnings.some((w) => w.kind === 'credits')} title={!canSave ? (saveBlockedWhy === 'reading' ? 'Waiting for PeopleSoft\'s current plan' : 'Open this term\'s schedule first, so every course in each term is counted') : warnings.some((w) => w.kind === 'credits') ? 'A term is over its credit limit' : undefined} onClick={onSave}>Save to PeopleSoft…</button>
      </div>
      <label className="v2-yearpick">Year{' '}
        <select value={year} onChange={(e) => setYear(Number(e.target.value))} aria-label="Academic year">
          {years.map((y) => <option key={y} value={y}>{academicYearLabel(y)}</option>)}
        </select>
      </label>
      {!canSave && saveBlockedWhy !== 'reading' ? <p className="v2-muted">To save, open this term's schedule first (Back, then Build this term's schedule): the credits of courses chosen inside Course Lists are only known from there.</p> : null}
      {!canSave && saveBlockedWhy === 'reading' ? <p className="v2-muted">Waiting for PeopleSoft's current plan. Saving needs it, because the copy on this device may be out of date.</p> : null}
      {warnings.length ? (
        <ul className="v2-warnings" role="alert">{warnings.map((w, i) => {
          const peer = w.kind === 'corequisite' && w.paired ? courses.find((c) => c.code === w.paired) : undefined
          const canAdd = !!peer && movable(peer) && allowedOf(peer).includes(w.term)
          const load = canAdd ? loadOf(loads, w.term) : null
          const fits = !!peer && !!load && peer.units !== null && load.credits + peer.units <= load.limit.max
          return (
            <li key={i}>
              {w.text}{' '}
              {canAdd && peer && load ? (
                <>
                  <button type="button" className="btn btn-sm" disabled={!fits} onClick={() => move(peer, w.term)}>Add {peer.code} to {w.term}</button>
                  {!fits ? <span className="v2-muted"> (would exceed {load.limit.max} credits)</span> : null}
                </>
              ) : null}
            </li>
          )
        })}</ul>
      ) : null}
      {suggestions.map((s) => (
        <p key={s.course.row} className="v2-suggest">
          {s.from} is over its limit. {s.course.code} fits in {s.to}, the nearest term with the fewest credits.{' '}
          <button type="button" className="btn btn-sm" onClick={() => move(s.course, s.to)}>Move it to {s.to}</button>{' '}
          <span className="v2-muted">{REVIEW_NOTE}</span>
        </p>
      ))}
      <div className="v2-degree__sems">
        {termsOfAcademicYear(year).map((t) => {
          const l = loadOf(loads, t)
          const inTerm = courses.filter((c) => termOf(c, draft) === t)
          const dragged = dragRow === null ? undefined : byRow.get(dragRow)
          const droppable = !!dragged && allowedOf(dragged).includes(t)
          return (
            <div
              key={t}
              className={`v2-card v2-sem v2-term${l.over ? ' is-over' : ''}${overTerm === t && droppable ? ' is-droptarget' : ''}${termOrder(t) < termOrder(openTerm) ? ' is-past' : ''}`}
              onDragOver={(e) => { if (droppable) { e.preventDefault(); setOverTerm(t) } }}
              onDragLeave={() => setOverTerm((o) => (o === t ? null : o))}
              onDrop={(e) => { e.preventDefault(); drop(t) }}
              aria-label={t}
            >
              <h3>{t} <span className={l.over ? 'v2-over' : 'v2-muted'}>{creditText(l.credits)} / {l.limit.max} credits{l.over ? ' – over' : ''}{l.unknown.length ? ` (at least: ${l.unknown.map((c) => c.code).join(', ')} has no credits listed)` : ''}{l.limit.confirmed ? '' : ' (limit not confirmed)'}</span></h3>
              {l.deferred.length ? <p className="v2-muted">Not counted here (over the limit otherwise): {l.deferred.map((d) => `${d.code}, which belongs to ${d.planningTerm}`).join('; ')}.</p> : null}
              <div className="v2-blocks">{inTerm.length ? inTerm.map(card) : <p className="v2-muted">Nothing here{droppable ? ' – drop a course to put it here' : ''}.</p>}</div>
            </div>
          )
        })}
      </div>
      {unplanned.length ? (
        <div className="v2-card">
          <h3>Not planned <span className="v2-muted">{unplanned.length} courses</span></h3>
          <div className="v2-blocks v2-blocks--wide">{unplanned.map(card)}</div>
        </div>
      ) : null}
      <p className="v2-muted">Credits count every course in the term, taken or planned, but not courses inside Course Lists you have not opened. Prerequisite warnings appear for courses whose "Prerequisite" you have opened in University planning.</p>
    </section>
  )
}

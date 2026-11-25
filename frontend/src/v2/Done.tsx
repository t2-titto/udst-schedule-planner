import { useMemo, useState } from 'react'
import { buildIcs, icsFileName, termRange } from '../engine/ics'
import { Calendar } from './Calendar'
import { asSchedule, blocksOf, overlapsOf, type PCourse, type PlannerState } from './model'

interface Props {
  courses: PCourse[]
  state: PlannerState
  term: string
  onBack: () => void
}

/** Only reached when everything is enrolled: the final week and the calendar file. */
export function Done({ courses, state, term, onBack }: Props) {
  const schedule = useMemo(() => asSchedule(courses, state), [courses, state])
  const blocks = useMemo(() => blocksOf(courses, state), [courses, state])
  // The term's first and last day come from PeopleSoft's own dates on the classes (any course carries them), not from a guess.
  const found = useMemo(() => {
    const all = termRange({ ...schedule, picks: courses.map((c, i) => ({ course: { code: c.code, title: c.title, term: c.term, options: c.options }, courseIndex: i, option: c.options[0] ?? c.held!, instructorScore: 0, limiting: [] })).filter((p) => p.option) })
    return all.start && all.end ? all : termRange(schedule)
  }, [courses, schedule])
  const [start, setStart] = useState(found.start ?? '')
  const [end, setEnd] = useState(found.end ?? '')
  const valid = start !== '' && end !== '' && start <= end
  const colorOf = (code: string) => Math.max(0, courses.findIndex((c) => c.code === code)) % 8

  const download = () => {
    const text = buildIcs(schedule, { termStart: start, termEnd: end, reminderMin: 30, calendarName: term ? `UDST ${term}` : 'UDST classes' })
    const url = URL.createObjectURL(new Blob([text], { type: 'text/calendar;charset=utf-8' }))
    const a = document.createElement('a')
    a.href = url
    a.download = icsFileName(term)
    document.body.appendChild(a)
    a.click()
    a.remove()
    window.setTimeout(() => URL.revokeObjectURL(url), 10000)
  }

  return (
    <div className="v2">
      <header className="v2-bar"><h1>Schedule planner</h1></header>
      <div className="v2-center">
        <div className="v2-card wide">
          <div style={{ textAlign: 'center' }}>
            <h2><span className="v2-tick" style={{ display: 'inline-grid', verticalAlign: 'middle', marginRight: '0.5rem' }}>✓</span>Everything is enrolled</h2>
            <p className="v2-muted">{term} · {schedule.picks.length} courses · matches PeopleSoft</p>
          </div>
          <Calendar blocks={blocks} overlaps={overlapsOf(blocks)} colorOf={colorOf} quiet />
          <div className="v2-card" style={{ width: '100%', boxShadow: 'none', border: '1px solid var(--color-rule-2)' }}>
            <h2 style={{ fontSize: 'var(--text-xl)' }}>Add to Google Calendar</h2>
            {valid ? <p className="v2-muted">Weekly events from {start} to {end}, read from PeopleSoft’s class dates.</p> : (
              <div className="v2-field"><label htmlFor="v2-start">First day</label><input id="v2-start" type="date" value={start} onChange={(e) => setStart(e.target.value)} /><label htmlFor="v2-end">Last day</label><input id="v2-end" type="date" value={end} onChange={(e) => setEnd(e.target.value)} /></div>
            )}
            <div className="v2-actions" style={{ justifyContent: 'flex-start' }}>
              <button type="button" className="btn" disabled={!valid} onClick={download}>Download calendar file (.ics)</button>
              <button type="button" className="v2-linkbtn" onClick={onBack}>Back to planner</button>
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}

import type { PlanProgress } from '../api'
import { DAY_SHORT } from '../engine/types'
import { downloadIcs } from './ics'
import { blocksOf, type PCourse, type PlannerState } from './model'
import { tileClassFor } from './SuggestionsStrip'

interface Props {
  term: string
  courses: PCourse[]
  state: PlannerState
  progress: PlanProgress
  onBack: () => void
  onSignOut: () => void
  tileMap?: ReadonlyMap<string, string>
}

const fmtClock = (min: number) => {
  const h = Math.floor(min / 60)
  const m = min % 60
  return `${h % 12 === 0 ? 12 : h % 12}:${String(m).padStart(2, '0')} ${h >= 12 ? 'PM' : 'AM'}`
}

export function Done({ term, courses, state, progress, onBack, onSignOut, tileMap }: Props) {
  const activeBlocks = blocksOf(courses, state)
  const doneItems = progress.items.filter((x) => x.status === 'done')

  return (
    <div className="planie-root">
      <header className="topbar">
        <div className="topbar-left">
          <span className="brand-mark"><span className="brand-dot" aria-hidden="true" />Planie</span>
          <span className="term-pill">{term}</span>
        </div>
        <div className="topbar-right">
          <button type="button" className="btn btn-sm" onClick={onBack}>Back to workspace</button>
          <button type="button" className="btn btn-sm" onClick={onSignOut}>Sign out</button>
        </div>
      </header>

      <main className="apply-shell done-shell">
        <section className="setup-card">
          <div className="suggestions-head">
            <div>
              <h2>You&apos;re enrolled!</h2>
              <p className="login-sub">
                {doneItems.length} update{doneItems.length === 1 ? '' : 's'} confirmed for {term}.
              </p>
            </div>
            <button type="button" className="btn btn-primary" onClick={() => downloadIcs(courses, state, term)}>
              Export to Google Calendar (.ics)
            </button>
          </div>

          <div className="done-week-grid">
            {[0, 1, 2, 3, 4].map((day) => {
              const dayClasses = activeBlocks.filter((b) => b.day === day).sort((a, b) => a.start - b.start)
              return (
                <div key={day} className="action-card">
                  <strong>{DAY_SHORT[day]}</strong>
                  {dayClasses.length === 0 ? (
                    <span className="suggest-sub">Free day</span>
                  ) : (
                    dayClasses.map((b, i) => (
                      <div key={i} className={`course-card ${tileClassFor(b.courseCode, tileMap)}`}>
                        <div className="course-code">{b.courseCode}</div>
                        <div className="suggest-sub num">{fmtClock(b.start)}–{fmtClock(b.end)}</div>
                        <div className="suggest-sub">{b.room || 'Campus'}</div>
                      </div>
                    ))
                  )}
                </div>
              )
            })}
          </div>
        </section>
      </main>
    </div>
  )
}

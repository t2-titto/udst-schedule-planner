import { useState } from 'react'
import { ApiError } from '../api'
import { loadCache } from './cache'
import { loadTerm, type Loaded, type LoadStep } from './data'
import { HomePanel } from './HomePanel'

interface Props {
  onLoaded: (l: Loaded) => void
  onSignOut: () => void
  onSwitchAccount?: () => void
  onHelp?: () => void
  onDegree: () => void
}

type StepState = 'idle' | 'working' | 'done'
const STEPS: { id: LoadStep; label: string }[] = [
  { id: 'plan', label: 'Courses in your plan' },
  { id: 'enrolled', label: 'Your enrolled classes' },
  { id: 'times', label: 'Class times' },
]

/** Term (always the open one), one button, and a short list of what is being read. */
export function Load({ onLoaded, onSignOut, onSwitchAccount, onHelp, onDegree }: Props) {
  const [started, setStarted] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [done, setDone] = useState<Record<string, string>>({})

  const go = async () => {
    setStarted(true)
    setError(null)
    setDone({})
    try {
      const loaded = await loadTerm((s, detail) => setDone((d) => ({ ...d, [s]: detail ?? '' })), { termHint: loadCache()?.term })
      onLoaded(loaded)
    } catch (e) {
      setError(e instanceof ApiError ? e.message : 'Could not read your courses. Try again.')
      setStarted(false)
    }
  }

  const stateOf = (id: LoadStep): StepState => (id in done ? 'done' : started ? (STEPS.findIndex((s) => !(s.id in done)) === STEPS.findIndex((s) => s.id === id) ? 'working' : 'idle') : 'idle')

  return (
    <div className="v2">
      <header className="v2-bar">
        <h1>Schedule planner</h1>
        <span className="v2-spacer" />
        {onHelp ? <button type="button" className="v2-linkbtn" onClick={onHelp}>Tutorial</button> : null}
        {onSwitchAccount ? <button type="button" className="v2-linkbtn" onClick={onSwitchAccount}>Switch account</button> : null}
        <button type="button" className="v2-linkbtn" onClick={onSignOut}>Sign out</button>
      </header>
      <div className="v2-center">
        <div className="v2-card">
          <div>
            <span className="v2-muted">open for enrolment</span>
            <h2>The open term</h2>
          </div>
          <button type="button" className="btn" disabled={started} onClick={go}>Build this term's schedule</button>
          <button type="button" className="btn btn-quiet" disabled={started} onClick={onDegree}>Plan my degree</button>
          {error ? <p className="v2-alert" role="alert">{error}</p> : null}
          {started ? (
            <ul className="v2-steps" aria-live="polite">
              {STEPS.map((s) => {
                const st = stateOf(s.id)
                return (
                  <li key={s.id}>
                    {st === 'done' ? <span className="v2-tick" role="img" aria-label="done">✓</span> : st === 'working' ? <span className="v2-spin" role="img" aria-label="working" /> : <span className="v2-idle" role="img" aria-label="waiting" />}
                    <span>{s.label}</span>
                    {done[s.id] ? <span className="muted">{done[s.id]}</span> : null}
                  </li>
                )
              })}
            </ul>
          ) : null}
        </div>
        <HomePanel paused={started} />
      </div>
    </div>
  )
}

import type { QuickSetup, SetupFreeDay, SetupTimePref } from './suggestions'
import { WaitWidget } from './WaitWidget'
import type { ThemeMode } from './theme'

export interface LoadStage {
  id: 'boot' | 'sections' | 'compatible'
  label: string
  status: 'pending' | 'working' | 'done' | 'error'
  detail?: string
}

interface Props {
  setup: QuickSetup
  onChangeSetup: (next: QuickSetup) => void
  showWaitlist: boolean
  onToggleWaitlist: () => void
  showClosed: boolean
  onToggleClosed: () => void
  stages: LoadStage[]
  ready: boolean
  error: string | null
  onRetry: () => void
  onContinue: () => void
  onSignOut: () => void
  demo?: boolean
  userHint: string | null
  theme: ThemeMode
  onToggleTheme: () => void
}

const FREE_DAYS: { val: SetupFreeDay; label: string }[] = [
  { val: 'any', label: 'Any' },
  { val: 0, label: 'Sun' },
  { val: 1, label: 'Mon' },
  { val: 2, label: 'Tue' },
  { val: 3, label: 'Wed' },
  { val: 4, label: 'Thu' },
]

const TIMES: { val: SetupTimePref; label: string }[] = [
  { val: 'any', label: 'Any time' },
  { val: 'morning', label: 'Morning' },
  { val: 'afternoon', label: 'Afternoon' },
  { val: 'evening', label: 'Evening' },
]

export function SetupWait(props: Props) {
  const {
    setup,
    onChangeSetup,
    showWaitlist,
    onToggleWaitlist,
    showClosed,
    onToggleClosed,
    stages,
    ready,
    error,
    onRetry,
    onContinue,
    onSignOut,
    demo,
    userHint,
    theme,
    onToggleTheme,
  } = props

  return (
    <div className="planie-root">
      <header className="topbar">
        <div className="topbar-left">
          <span className="brand-mark"><span className="brand-dot" aria-hidden="true" />Planie</span>
          {demo ? <span className="demo-badge">Demo</span> : userHint ? <span className="term-pill">{userHint}</span> : null}
        </div>
        <div className="topbar-right">
          <button type="button" className="icon-btn" onClick={onToggleTheme}>
            {theme === 'dark' ? '☀️ Light' : '🌙 Dark'}
          </button>
          <button type="button" className="btn btn-sm" onClick={onSignOut}>Sign out</button>
        </div>
      </header>

      <main className="setup-shell">
        <section className="setup-card">
          <div>
            <h2>Quick setup</h2>
            <p className="login-sub">Set your vibe while Planie pulls your schedule and eligible degree courses.</p>
          </div>

          <div>
            <div className="pill-group-label">Prefer a free day?</div>
            <div className="pill-row">
              {FREE_DAYS.map((d) => (
                <button
                  key={String(d.val)}
                  type="button"
                  className={`choice-pill ${setup.freeDay === d.val ? 'active' : ''}`}
                  onClick={() => onChangeSetup({ ...setup, freeDay: d.val, freeDays: typeof d.val === 'number' ? [d.val] : [] })}
                >
                  {d.label}
                </button>
              ))}
            </div>
          </div>

          <div>
            <div className="pill-group-label">Best time of day</div>
            <div className="pill-row">
              {TIMES.map((t) => (
                <button
                  key={t.val}
                  type="button"
                  className={`choice-pill ${setup.timePref === t.val ? 'active' : ''}`}
                  onClick={() => onChangeSetup({ ...setup, timePref: t.val })}
                >
                  {t.label}
                </button>
              ))}
            </div>
          </div>

          <div>
            <div className="pill-group-label">Include sections</div>
            <div className="pill-row">
              <span className="choice-pill locked-on">✓ Open</span>
              <button type="button" className={`choice-pill ${showWaitlist ? 'active' : ''}`} onClick={onToggleWaitlist}>
                Waitlisted
              </button>
              <button type="button" className={`choice-pill ${showClosed ? 'active' : ''}`} onClick={onToggleClosed}>
                Closed (browse)
              </button>
            </div>
          </div>

          <div className="stage-list" aria-live="polite">
            {stages.map((st) => (
              <div key={st.id} className={`stage-item ${st.status}`}>
                <span className="stage-left">
                  <span className="stage-dot" aria-hidden="true" />
                  <span>{st.label}</span>
                </span>
                <span className="suggest-sub">
                  {st.status === 'done' ? st.detail || 'Ready' : st.status === 'working' ? 'Fetching…' : st.status === 'error' ? st.detail || 'Skipped' : 'Queued'}
                </span>
              </div>
            ))}
          </div>

          {error ? (
            <div className="course-meta-row">
              <span className="status-badge bad">{error}</span>
              <button type="button" className="btn btn-sm" onClick={onRetry}>Retry</button>
            </div>
          ) : null}

          <button type="button" className="btn btn-primary" disabled={!ready} onClick={onContinue}>
            {ready ? 'Open my workspace →' : 'Fetching your timetable…'}
          </button>
        </section>

        <WaitWidget title="Relax while we fetch" />
      </main>
    </div>
  )
}

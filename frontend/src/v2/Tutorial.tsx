import { useState } from 'react'

export const TUTORIAL_KEY = 'schedule-planner:v2:tutorial-seen'

export function hasSeenTutorial(): boolean {
  try { return window.localStorage.getItem(TUTORIAL_KEY) === '1' } catch { return true }
}

export function markTutorialSeen(): void {
  try { window.localStorage.setItem(TUTORIAL_KEY, '1') } catch { /* ignore */ }
}

const STEPS = [
  {
    title: '1. Private & Safe by Default',
    body: 'This planner runs 100% locally on your computer (localhost:6767). Your UDST PeopleSoft credentials stay in memory only and are never saved to disk or sent to any third party. Writes to PeopleSoft stay OFF until you explicitly review and press Apply.',
  },
  {
    title: '2. Sign In & Load Your Term',
    body: 'Sign in with your UDST student number and PeopleSoft password, accept the safety statements, then click "Build this term\'s schedule". The planner reads your current enrolled classes, available sections, and live seat counts.',
  },
  {
    title: '3. Customize Your Ideal Week',
    body: 'Click any course on the left to inspect all its sections (Open and Closed), lock (🔒) classes you want to keep, or click "Preferences" to weight free days, time windows, and instructors. Turn Auto on to follow the highest-scoring schedule automatically.',
  },
  {
    title: '4. Plan Your Degree & Prerequisites',
    body: 'Click "Degree" to plan future semesters on a drag-and-drop board with live 18-credit / 12-credit checks, prerequisite warnings, and co-requisite alerts (with a one-click button to pair lecture + lab in the same term).',
  },
  {
    title: '5. Review Before Applying & Switch Accounts',
    body: 'Press "Apply" to see every swap, enrolment, and drop on one review screen with live seat checks before anything is sent to PeopleSoft. Need another account? Click "Switch account" or open http://localhost:6767 in a private window for a fresh session.',
  },
]

interface Props {
  onClose: () => void
}

export function TutorialModal({ onClose }: Props) {
  const [step, setStep] = useState(0)
  const cur = STEPS[step]
  const last = step === STEPS.length - 1

  const finish = () => {
    markTutorialSeen()
    onClose()
  }

  return (
    <div className="v2-modal-backdrop" role="dialog" aria-modal="true" aria-labelledby="v2-tut-title">
      <div className="v2-card v2-modal">
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', gap: '0.5rem' }}>
          <span className="v2-muted">Quick Guide ({step + 1} / {STEPS.length})</span>
          <button type="button" className="v2-linkbtn" onClick={finish}>Skip</button>
        </div>
        <h2 id="v2-tut-title" style={{ margin: '0.25rem 0' }}>{cur.title}</h2>
        <p style={{ lineHeight: 1.55, margin: '0.25rem 0 0.75rem' }}>{cur.body}</p>
        <div className="v2-actions" style={{ justifyContent: 'space-between' }}>
          <button type="button" className="btn btn-quiet" disabled={step === 0} onClick={() => setStep((s) => s - 1)}>Back</button>
          <div style={{ display: 'flex', gap: '0.5rem' }}>
            {!last ? (
              <button type="button" className="btn" onClick={() => setStep((s) => s + 1)}>Next</button>
            ) : (
              <button type="button" className="btn" onClick={finish}>Got it, let's start</button>
            )}
          </div>
        </div>
      </div>
    </div>
  )
}

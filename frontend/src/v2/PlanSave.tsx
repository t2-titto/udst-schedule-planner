import { useMemo, useState } from 'react'
import { api } from '../api'
import { executeMoves, msg, type Step } from './runMoves'
import type { DegreeView, WireDegree } from './degree'
import type { Draft } from './draft'
import { compareTrees, moveProblems, planSave } from './savePlan'
import { creditText } from './credits'

interface Props {
  view: DegreeView
  wire: WireDegree
  openTerm: string
  draft: Draft
  /** `fresh` is the plan read back from PeopleSoft after saving (null when nothing was read). `clean` is true when every change was saved and proved. */
  onDone: (fresh: WireDegree | null, clean: boolean) => void
  onCancel: () => void
}


/** The one review before anything is sent, then the saves one by one (course planning only: it never enrols, drops or swaps). */
export function PlanSave({ view, wire, openTerm, draft, onDone, onCancel }: Props) {
  const plan = useMemo(() => planSave(view, wire, openTerm, draft), [view, wire, openTerm, draft])
  const problems = useMemo(() => plan.moves.flatMap(moveProblems), [plan])
  const [phase, setPhase] = useState<'review' | 'running' | 'finished'>('review')
  const [steps, setSteps] = useState<Step[]>(() => plan.moves.map(() => 'waiting'))
  const [error, setError] = useState<string | null>(null)
  const [after, setAfter] = useState<{ fresh: WireDegree | null; problems: string[] } | null>(null)
  const set = (i: number, s: Step) => setSteps((x) => x.map((v, j) => (j === i ? s : v)))
  const blocked = plan.stuck !== null || problems.length > 0 || plan.moves.length === 0

  const run = async () => {
    setPhase('running')
    setError(null)
    const { done, error: failed } = await executeMoves(plan.moves, set)
    if (failed) setError(failed)
    const stopped = failed !== null
    // The whole plan is read back from PeopleSoft and compared.
    let fresh: WireDegree | null = null
    let diff: string[] = []
    try {
      fresh = await api.writeRead<WireDegree>('degree_tree')
      diff = compareTrees(wire, done, fresh)
    } catch (e) {
      diff = [`Could not read the plan back from PeopleSoft (${msg(e)}). Check Plan and Enrol by hand.`]
    }
    setAfter({ fresh, problems: diff })
    setPhase('finished')
    if (!stopped && diff.length === 0) onDone(fresh, true)
  }

  return (
    <section className="v2-card v2-save" aria-label="Save to PeopleSoft">
      <h2>{phase === 'review' ? 'Save to PeopleSoft' : phase === 'running' ? 'Saving…' : 'Saved'}</h2>
      <p className="v2-muted">This changes your <strong>course planning</strong> only. It does not enrol you in anything. PeopleSoft saves the plan without checking the credit limit, so this app checks it first.</p>
      <ul className="v2-savelist">
        {plan.moves.map((m, i) => (
          <li key={m.course.row}>
            <strong>{m.course.code}</strong> {m.from ?? 'not planned'} → <strong>{m.to}</strong>
            <span className="v2-muted"> · {m.to} will have {creditText(m.after)} credits</span>
            {phase !== 'review' ? <span className={`v2-step is-${steps[i].replace(' ', '-')}`}> · {steps[i]}</span> : null}
          </li>
        ))}
      </ul>
      {plan.stuck ? <p className="v2-alert" role="alert">{plan.stuck} Change the draft first.</p> : null}
      {problems.map((p) => <p key={p} className="v2-alert" role="alert">{p}</p>)}
      {error ? <p className="v2-alert" role="alert">{error} The changes after it were not sent.</p> : null}
      {after ? (
        after.problems.length === 0
          ? <p>PeopleSoft's plan was read back and matches: only the courses above changed.</p>
          : <div className="v2-alert" role="alert"><p>PeopleSoft's plan does not match what was expected. Check Plan and Enrol by hand:</p><ul>{after.problems.map((p) => <li key={p}>{p}</li>)}</ul></div>
      ) : null}
      <div className="v2-actions">
        {phase === 'review' ? (
          <>
            <button type="button" className="btn" disabled={blocked} onClick={() => void run()}>Save {plan.moves.length} change{plan.moves.length === 1 ? '' : 's'} to PeopleSoft</button>
            <button type="button" className="btn btn-quiet" onClick={onCancel}>Keep as draft</button>
            <span className="v2-muted">About {Math.max(1, plan.moves.length) * 45} seconds (PeopleSoft is read again before each change).</span>
          </>
        ) : null}
        {phase === 'finished' ? (
          <button type="button" className="btn" onClick={() => onDone(after?.fresh ?? null, !error && (after?.problems.length ?? 1) === 0)}>Close</button>
        ) : null}
      </div>
    </section>
  )
}

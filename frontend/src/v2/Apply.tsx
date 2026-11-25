import { useEffect, useMemo, useRef, useState } from 'react'
import { api, ApiError, type PlanProgress } from '../api'
import { fmtCompact } from '../engine/normalize'
import { DAY_SHORT, type ClassOption } from '../engine/types'
import type { EnrolledState } from '../enrolled'
import { buildPlan } from '../plan'
import { blockerText, planSwapRounds, type RoundSwap } from '../swapRounds'
import { asSchedule, type PCourse, type PlannerState } from './model'
import { buildApplyRequest, wireProblems } from './applyRequest'
import { CreditPanel } from './CreditPanel'
import { creditsFor, creditText } from './credits'
import { loadSettings } from './data'
import { buildDegree, type WireDegree } from './degree'
import { evalRuleTree, formatRuleTree, getCatalogRule } from './catalogRules'
import { isMet, normCode } from './prereq'
import { coreqMap, loadPrereqs, prereqMap } from './prereqCache'
import { planChanged, planDiff, type PlanDiff } from './verifyPlan'
import { keptDropClashes, verifyStale } from './applyGuard'

interface Props {
  courses: PCourse[]
  /** Course code -> the term it belongs to in the degree's order. */
  planning: Record<string, string>
  state: PlannerState
  enrolled: EnrolledState
  onBack: () => void
  onDone: () => void
  /** PeopleSoft's real state changed: read it again. */
  onRefresh: () => Promise<void>
  /** Course code -> the combination PeopleSoft refused, so the planner stops offering it. */
  onRefused: (refused: Record<string, string>) => void
}

const when = (o: ClassOption) => [...new Set(o.components.flatMap((c) => c.meetings.map((m) => `${DAY_SHORT[m.day]} ${fmtCompact(m.start, m.end)}`)))].join(', ')
const teachers = (o: ClassOption) => [...new Set(o.components.flatMap((c) => c.instructors))].join(', ')
const seats = (o: ClassOption) =>
  o.status.toLowerCase() === 'closed' && o.components.every((c) => c.seatsOpen === null)
    ? 'Closed'
    : o.components.map((c) => (c.seatsOpen === null ? '—' : c.seatsOpen)).join(' / ')
const msg = (e: unknown) => (e instanceof ApiError ? e.message : 'Something went wrong. Nothing was sent.')

function Icon({ status }: { status: PlanProgress['items'][number]['status'] }) {
  if (status === 'done') return <span className="v2-tick" role="img" aria-label="done">✓</span>
  if (status === 'error') return <span className="v2-cross" role="img" aria-label="failed">✕</span>
  if (status === 'working') return <span className="v2-spin" role="img" aria-label="working" />
  return <span className="v2-idle" role="img" aria-label="waiting" />
}

/** Review, then live progress, on one screen. */
export function Apply({ courses, planning, state, enrolled, onBack, onDone, onRefresh, onRefused }: Props) {
  const addPlans = Object.fromEntries(courses.filter((c) => c.addPlan).map((c) => [c.code, c.addPlan!]))
  const plan = buildPlan(asSchedule(courses, state), enrolled.options, enrolled.rows, { selected: [], hidden: [], courses: [], live: new Set(courses.map((c) => c.code)) })
  const rp = useMemo(
    () =>
      planSwapRounds(
        plan.swaps.map((s): RoundSwap => ({ code: s.course.code, now: s.now, next: s.next, seatsLeft: s.next.components.reduce<number | null>((m, c) => (c.seatsOpen === null ? m : m === null ? c.seatsOpen : Math.min(m, c.seatsOpen)), null) })),
        plan.keeps.map((k) => k.now),
        plan.swaps.filter((s) => s.redo).map((s) => s.course.code),
      ),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [plan.swaps.length, plan.keeps.length, courses, state],
  )
  const roundOf = (code: string) => rp.rounds.findIndex((r) => r.some((s) => s.code === code)) + 1
  const hasLoop = rp.converted.length > 0
  const [ack, setAck] = useState(false)
  const [dropOn, setDropOn] = useState<Record<string, boolean>>({})
  // Waitlists (UNVERIFIED): joining one means you are NOT enrolled; leaving or swapping one gives up the place.
  const [waitAck, setWaitAck] = useState(false)
  const [reqAck, setReqAck] = useState(false)
  const [overrideCredits, setOverrideCredits] = useState(false)
  const joins = plan.enrols.filter((e) => e.next.waitlist).map((e) => e.course.code)
  const leaves = [...plan.swaps.filter((x) => x.now.waitlisted).map((x) => x.course.code), ...plan.drops.filter((d) => enrolled.options[d.code]?.waitlisted).map((d) => d.code)]
  const needsWaitAck = joins.length > 0 || leaves.length > 0
  // An enrolled course whose drop was unticked on this screen stays enrolled, so it still counts.
  const credits = creditsFor(courses, { ...state, removed: state.removed.filter((c) => dropOn[c] !== false) }, enrolled.term, planning, overrideCredits)
  // PeopleSoft is read once more right now: the credit total must come from what it holds this minute, not from when the page loaded.
  const [verify, setVerify] = useState<{ status: 'checking' | 'ok' | 'changed' | 'failed'; diff?: PlanDiff }>({ status: 'checking' })
  const [recheck, setRecheck] = useState(0)
  const [verifiedAt, setVerifiedAt] = useState<number | null>(null)
  useEffect(() => {
    let live = true
    setVerify({ status: 'checking' })
    setVerifiedAt(null)
    api.refresh(enrolled.term).then((r) => {
      if (!live) return
      const diff = planDiff(r.classes.rows, enrolled.term, courses, !!loadSettings().rules.joinWaitlist)
      setVerify(planChanged(diff) ? { status: 'changed', diff } : { status: 'ok' })
      setVerifiedAt(Date.now())
    }).catch(() => { if (live) setVerify({ status: 'failed' }) })
    return () => { live = false }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [recheck])
  const [error, setError] = useState<string | null>(null)
  const [progress, setProgress] = useState<PlanProgress | null>(null)
  // The check "right now" is only good for two minutes: a student who leaves the review open gets it run again before Apply can be pressed.
  useEffect(() => {
    const t = window.setInterval(() => {
      if (verify.status === 'ok' && verifyStale(verifiedAt, Date.now()) && !progress) setRecheck((n) => n + 1)
    }, 15000)
    return () => window.clearInterval(t)
  }, [verify.status, verifiedAt, progress])
  const timer = useRef<number | undefined>(undefined)
  const sent = useRef<{ swaps: Record<string, string>; enrols: Record<string, string> }>({ swaps: {}, enrols: {} })
  useEffect(() => () => window.clearTimeout(timer.current), [])

  const drops = plan.drops.filter((d) => dropOn[d.code] !== false)
  const kept = keptDropClashes(plan, enrolled.options, dropOn)
  // A waitlisted course the Drop page did not list has no class number: it cannot be dropped from here.
  const noNumber = drops.filter((d) => !d.classNbr).map((d) => d.code)
  const closedTargets = [
    ...plan.swaps.filter((s) => s.next.status.toLowerCase() === 'closed' || s.next.components.some((c) => c.seatsOpen === 0)).map((s) => s.course.code),
    ...plan.enrols.filter((e) => !e.next.waitlist && (e.next.status.toLowerCase() === 'closed' || e.next.components.some((c) => c.seatsOpen === 0))).map((e) => e.course.code),
  ]
  const reqWarnings = useMemo(() => {
    const cache = loadPrereqs()
    const now = Date.now()
    const livePrereqs = prereqMap(cache, now)
    const liveCoreqs = coreqMap(cache, now)
    const droppedCodes = new Set(drops.map((d) => normCode(d.code)))
    const activeEnrols = new Set(plan.enrols.filter((e) => !e.next.waitlist).map((e) => normCode(e.course.code)))
    const heldEnrolled = new Set(
      Object.entries(enrolled.options)
        .filter(([c, o]) => !o.waitlisted && !droppedCodes.has(normCode(c)))
        .map(([c]) => normCode(c)),
    )
    // Completed courses from cached degree_tree (when present), so satisfied prerequisites are not flagged.
    let done: Set<string> | null = null
    try {
      const deg = JSON.parse(window.localStorage.getItem('schedule-planner:v2:degree') ?? 'null') as { wire?: WireDegree } | null
      if (deg?.wire && Array.isArray(deg.wire.rows)) {
        const view = buildDegree(deg.wire, enrolled.term)
        done = new Set<string>(heldEnrolled)
        for (const y of view.years) {
          for (const s of y.semesters) {
            for (const b of s.blocks) {
              if (b.kind === 'course' && b.course.status === 'completed') done.add(normCode(b.course.code))
            }
          }
        }
      }
    } catch { /* ignore */ }

    const out: string[] = []
    for (const code of [...activeEnrols, ...plan.swaps.map((s) => normCode(s.course.code))]) {
      const cat = getCatalogRule(code)
      const cos = [...new Set([...(liveCoreqs.get(code) ?? []), ...(cat?.corequisites ?? [])])]
      for (const co of cos) {
        if (heldEnrolled.has(co) || activeEnrols.has(co)) continue
        const waitlisted = enrolled.options[co]?.waitlisted
        out.push(waitlisted ? `${code} requires co-requisite ${co}, which is currently waitlisted (not enrolled).` : `${code} requires co-requisite ${co} in the same term.`)
      }
      if (done) {
        const live = livePrereqs.get(code)
        if (live) {
          if (isMet(live, done) !== true) {
            out.push(`${code} prerequisite: ${live.raw}`)
          }
        } else if (cat?.prerequisites) {
          if (evalRuleTree(cat.prerequisites, done) !== true) {
            out.push(`${code} prerequisite: ${formatRuleTree(cat.prerequisites)}`)
          }
        } else if (cat?.cohort_rules?.length) {
          if (!cat.cohort_rules.some((cr) => evalRuleTree(cr.prerequisites, done!) === true)) {
            out.push(`${code} prerequisite (cohort-dependent): ${cat.cohort_rules.map((cr) => `${cr.condition}: ${formatRuleTree(cr.prerequisites)}`).join('; ')}`)
          }
        }
      }
      if (cat?.notes?.length) {
        out.push(`${code} requirement note: ${cat.notes.join('; ')}`)
      }
    }
    return out
  }, [drops, plan.enrols, plan.swaps, enrolled.options, enrolled.term])
  const blockedByChecks = plan.blockers.length > 0 || kept.length > 0 || noNumber.length > 0 || closedTargets.length > 0
  const planned = plan.swaps.length + plan.enrols.length + drops.length
  const running = progress !== null && !progress.finished
  const failed = progress?.finished && progress.items.some((i) => i.status === 'error')

  const poll = async () => {
    try {
      const p = await api.planProgress()
      setProgress(p)
      if (!p.finished) {
        timer.current = window.setTimeout(() => void poll(), 1200)
        return
      }
      // The run is over: switch writes off again (a new Apply switches them on itself), so nothing stays armed after it.
      try { await api.writeDisarm() } catch { /* the idle timer switches them off anyway */ }
      const refused: Record<string, string> = {}
      for (const it of p.items) {
        if (it.status !== 'error') continue
        if (/^not sent\b|not sent\.|waits for/i.test(it.note)) continue
        const id = it.kind === 'swap' ? sent.current.swaps[it.course] : it.kind === 'enrol' ? sent.current.enrols[it.course] : undefined
        if (id) refused[it.course] = id
      }
      if (Object.keys(refused).length) onRefused(refused)
      await onRefresh()
      if (p.items.length && p.items.every((i) => i.status === 'done')) onDone()
    } catch (e) {
      setError(msg(e))
    }
  }

  const go = async () => {
    if (credits.blocked || verify.status !== 'ok' || blockedByChecks || (needsWaitAck && !waitAck) || (reqWarnings.length > 0 && !reqAck)) return
    if (verifyStale(verifiedAt, Date.now())) { setRecheck((n) => n + 1); return }
    setError(null)
    sent.current = {
      swaps: Object.fromEntries(plan.swaps.map((s) => [s.course.code, s.next.id])),
      enrols: Object.fromEntries([...plan.enrols.map((e) => [e.course.code, e.next.id]), ...rp.converted.map((c) => [c.code, c.next.id])]),
    }
    try {
      // The one consent was given at sign-in; writes are switched on now, only for this.
      const ws = await api.writeStatus()
      if (!ws.armed) await api.writeArm(ws.consent.statements.map((s) => s.id), ws.consent.version)
      const request = buildApplyRequest(plan, rp, enrolled, drops, credits, addPlans)
      const problems = wireProblems(request)
      if (problems.length) throw new ApiError(`The plan could not be put into a request (${problems[0]}). Nothing was sent.`, 0)
      await api.planApply(request)
      setProgress({ id: 0, phase: 'starting', finished: false, error: null, items: [] })
      void poll()
    } catch (e) {
      setError(msg(e))
      // Nothing is running (the request failed before the job started), so do not leave writes switched on.
      try { await api.writeDisarm() } catch { /* the idle timer switches them off anyway */ }
    }
  }

  return (
    <div className="v2">
      <header className="v2-bar"><h1>Schedule planner</h1></header>
      <div className="v2-center">
        {!progress ? (
          <div className="v2-card wide">
            <div>
              <h2>Review before you apply</h2>
              <p className="v2-muted">{[`${plan.keeps.length} kept`, `${plan.swaps.length} swap${plan.swaps.length === 1 ? '' : 's'}`, `${plan.enrols.length} new enrolment${plan.enrols.length === 1 ? '' : 's'}`, `${plan.drops.length} drop${plan.drops.length === 1 ? '' : 's'}`].join(', ')}</p>
            </div>
            <table className="v2-table">
              <thead><tr><th>Action</th><th>Course</th><th>Now</th><th>Becomes</th><th>Seats left</th><th>Instructor</th></tr></thead>
              <tbody>
                {plan.keeps.map((k) => <tr key={`k${k.code}`}><td>Keep</td><td><strong>{k.code}</strong></td><td colSpan={4}>{when(k.now)}</td></tr>)}
                {plan.swaps.filter((s) => !rp.converted.some((c) => c.code === s.course.code)).map((s) => {
                  const waits = rp.waits[s.course.code] ?? []
                  return (
                    <tr key={`s${s.course.code}`}>
                      <td><strong>Swap</strong>{rp.rounds.length > 1 ? <div className="v2-muted">Round {roundOf(s.course.code)}</div> : null}</td>
                      <td><strong>{s.course.code}</strong></td>
                      <td>{when(s.now)}</td>
                      <td>{when(s.next)}{waits.length ? <div className="v2-muted">{blockerText(waits[0])}</div> : null}</td>
                      <td>{seats(s.next)}</td>
                      <td>{teachers(s.now) === teachers(s.next) ? `${teachers(s.next)} (same)` : `${teachers(s.now)} → ${teachers(s.next)}`}</td>
                    </tr>
                  )
                })}
                {rp.converted.map((c) => (
                  <tr key={`l${c.code}`}>
                    <td><strong>Drop, then enrol again</strong>{rp.forced.includes(c.code) ? <div className="v2-muted">Only the lab changes</div> : null}</td>
                    <td><strong>{c.code}</strong></td>
                    <td>{when(c.now)}</td>
                    <td>{when(c.next)}<div className="v2-muted">{rp.forced.includes(c.code) ? 'PeopleSoft refuses a swap that changes only the lab, so this course is dropped and then enrolled again.' : 'PeopleSoft cannot swap this one: it waits for another swap that waits for it.'}</div></td>
                    <td>{c.seatsLeft ?? (c.next.status.toLowerCase() === 'closed' ? 'Closed' : '—')}</td>
                    <td>{teachers(c.next)}</td>
                  </tr>
                ))}
                {plan.enrols.map((e) => <tr key={`e${e.course.code}`}><td><strong>{e.next.waitlist ? 'Join waitlist' : addPlans[e.course.code] ? 'Plan, then enrol' : 'Enrol'}</strong>{e.next.waitlist ? <div className="v2-muted">not enrolled</div> : null}</td><td><strong>{e.course.code}</strong></td><td>—</td><td>{when(e.next)}</td><td>{e.next.waitlist ? e.next.components.map((c) => (c.waitPlaces == null ? '—' : `${c.waitPlaces} on waitlist`)).join(' / ') : seats(e.next)}</td><td>{teachers(e.next)}</td></tr>)}
                {plan.drops.map((d) => (
                  <tr key={`d${d.code}`}>
                    <td><strong>Drop</strong></td><td><strong>{d.code}</strong></td><td colSpan={3}>{d.classes.join(', ')}</td>
                    <td><label><input type="checkbox" checked={dropOn[d.code] !== false} onChange={(e) => setDropOn({ ...dropOn, [d.code]: e.target.checked })} /> drop it</label></td>
                  </tr>
                ))}
              </tbody>
            </table>
            {rp.forced.length ? (
              <div className="v2-alert" role="alert">
                <strong>{rp.forced.join(', ')}: only the lab changes.</strong>{' '}
                PeopleSoft refuses that as a swap, so {rp.forced.length > 1 ? 'these courses are' : 'this course is'} dropped and enrolled again in the same run (the drop first, the enrolment after the swaps).
                Between the two someone else could take the seat.
              </div>
            ) : null}
            {rp.loops.length ? (
              <div className="v2-alert" role="alert">
                <strong>These swaps wait for each other: {rp.loops.map((l) => l.join(' and ')).join('; ')}.</strong>{' '}
                PeopleSoft checks each swap against what you hold today, so it cannot do both. {rp.converted.filter((c) => !rp.forced.includes(c.code)).map((c) => c.code).join(', ')} (the one with the most seats left)
                will be dropped and enrolled again after the swaps. Between the drop and the new enrolment someone else could take the seat.
              </div>
            ) : null}
            {hasLoop ? (
              <label style={{ display: 'block', marginTop: 'var(--space-xs)' }}><input type="checkbox" checked={ack} onChange={(e) => setAck(e.target.checked)} /> I understand I could lose {rp.converted.length > 1 ? 'these seats' : 'this seat'}</label>
            ) : null}
            {needsWaitAck ? (
              <div className="v2-alert" role="alert">
                <strong>Waitlists are unverified here.</strong>{' '}
                {joins.length ? <>Joining the waitlist of {joins.join(', ')} does <strong>not</strong> enrol you: you are not in the class, and PeopleSoft may enrol you later without asking you. </> : null}
                {leaves.length ? <>Dropping or swapping {leaves.join(', ')} gives up your place on its waitlist; you may not get it back. </> : null}
                This has not been tried on a real waitlist yet, so PeopleSoft's own answer is shown after each step.
                <label style={{ display: 'block', marginTop: 'var(--space-xs)' }}><input type="checkbox" checked={waitAck} onChange={(e) => setWaitAck(e.target.checked)} /> I understand</label>
              </div>
            ) : null}
            {credits.waitlistUnits > 0 ? <p className="v2-muted">{creditText(credits.waitlistUnits)} of these credits are waitlisted classes: PeopleSoft may or may not count them.</p> : null}
            {credits.waitlistOnly ? (
              <label style={{ display: 'block' }}><input type="checkbox" checked={overrideCredits} onChange={(e) => setOverrideCredits(e.target.checked)} /> Go on anyway: the total is over {credits.limit.max} only if PeopleSoft counts my waitlisted classes</label>
            ) : null}
            {noNumber.map((c) => <p key={`nn${c}`} className="v2-alert" role="alert">{c} cannot be dropped from here: PeopleSoft's Drop page does not list its class. Leave its waitlist in PeopleSoft, then refresh.</p>)}
            <p className="v2-muted">Credits: {creditText(credits.after)}{credits.unknown.length ? '+' : ''} / {credits.limit.max} counted by PeopleSoft for {enrolled.term}{credits.limit.confirmed ? '' : ` (${credits.limit.note})`}.</p>
            {credits.deferred.length ? <p className="v2-muted">Not counted in {enrolled.term} because it would be over the limit otherwise: {credits.deferred.map((d) => `${d.code} (it belongs to ${d.to})`).join(', ')}. PeopleSoft may still count it and refuse the enrolment; if it does, its refusal is shown here.</p> : null}
            {credits.blocked ? <CreditPanel credits={credits} onRemoveEnrolled={() => onBack()} removeLabel="Back to the planner to take it out" /> : null}
            {verify.status === 'checking' ? <p className="v2-muted" role="status">Checking PeopleSoft's plan once more before you apply (about 20 seconds)…</p> : null}
            {verify.status === 'failed' ? <p className="v2-alert" role="alert">Could not re-check PeopleSoft, so Apply is off. <button type="button" className="btn btn-quiet btn-sm" onClick={() => setRecheck((n) => n + 1)}>Try again</button></p> : null}
            {verify.status === 'changed' && verify.diff ? (
              <p className="v2-alert" role="alert">
                PeopleSoft's plan changed since this page loaded ({[...verify.diff.plannedNew.map((c) => `${c} is now planned`), ...verify.diff.plannedGone.map((c) => `${c} is no longer planned`), ...verify.diff.enrolledNow.map((c) => `${c} is now enrolled`), ...verify.diff.enrolledGone.map((c) => `${c} is no longer enrolled`)].join(', ')}). The credit total may be wrong, so Apply is off. Go back and refresh.
              </p>
            ) : null}
            {plan.blockers.map((b, i) => <p key={i} className="v2-alert" role="alert">{b}</p>)}
            {closedTargets.length ? <p className="v2-alert" role="alert">Closed or full: {closedTargets.join(', ')} has no open seats. Pick another option before applying.</p> : null}
            {reqWarnings.length ? (
              <div className="v2-alert" role="alert">
                <strong>Check requirements before applying:</strong>
                <ul style={{ margin: '0.25rem 0 0', paddingLeft: '1.2rem' }}>
                  {reqWarnings.map((m, i) => <li key={i}>{m}</li>)}
                </ul>
                <label style={{ display: 'block', marginTop: 'var(--space-xs)' }}>
                  <input type="checkbox" checked={reqAck} onChange={(e) => setReqAck(e.target.checked)} /> I meet these requirements or have an override — continue anyway
                </label>
              </div>
            ) : null}
            {kept.map((b, i) => <p key={`kept${i}`} className="v2-alert" role="alert">{b}</p>)}
            {plan.blockers.length === 0 && kept.length === 0 ? <p className="v2-muted">✓ The final schedule has no overlaps. Swaps are ordered so none overlaps a class that is still held when it is sent.</p> : null}
            {error ? <p className="v2-alert" role="alert">{error}</p> : null}
            <div className="v2-actions">
              <button type="button" className="btn btn-quiet" onClick={onBack}>Back to planner</button>
              <button type="button" className="btn" disabled={blockedByChecks || (needsWaitAck && !waitAck) || (reqWarnings.length > 0 && !reqAck) || planned === 0 || (hasLoop && !ack) || credits.blocked || verify.status !== 'ok'} onClick={go}>{verify.status === 'checking' ? 'Checking PeopleSoft…' : 'Apply'}</button>
            </div>
          </div>
        ) : (
          <div className="v2-card">
            <h2>Applying to PeopleSoft</h2>
            <ul className="v2-progress" aria-live="polite">
              {progress.items.map((it) => (
                <li key={it.id}>
                  <Icon status={it.status} />
                  <div><div className="what">{it.kind === 'drop' ? (enrolled.options[it.course]?.waitlisted ? 'Leave waitlist' : 'Drop') : it.kind === 'swap' ? 'Swap' : joins.includes(it.course) ? 'Join waitlist' : 'Enrol'} {it.course}{it.round && it.round > 1 ? ` (round ${it.round})` : ''}</div><div className="note">{it.note}</div></div>
                </li>
              ))}
            </ul>
            {progress.error ? <p className="v2-alert" role="alert">{progress.error}</p> : null}
            {error ? <p className="v2-alert" role="alert">{error}</p> : null}
            {running ? <p className="v2-muted">Keep this tab open.</p> : null}
            {failed ? (
              <div className="v2-actions">
                <button type="button" className="btn btn-quiet" onClick={onBack}>Back to planner</button>
                <button type="button" className="btn" onClick={() => { setProgress(null); setError(null) }}>Fix and retry</button>
              </div>
            ) : null}
          </div>
        )}
      </div>
    </div>
  )
}

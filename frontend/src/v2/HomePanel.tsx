import { useEffect, useMemo, useRef, useState } from 'react'
import { api } from '../api'
import { attentionList, countsFor, loadSnapshot, OLD_AFTER_MS, openTermOf, saveSnapshot, waitTransitions, type Snapshot } from './attention'
import { creditText } from './credits'
import { loadSettings, termOrder } from './data'
import { buildDegree, type WireDegree } from './degree'
import { coursesOf, termLoads } from './draft'
import { nextToRead, loadPrereqs, prereqMap, savePrereqs, type PrereqCache, type ToRead } from './prereqCache'
import { ago } from './Planner'

const DEGREE_KEY = 'schedule-planner:v2:degree'
const DEGREE_OLD_MS = 3600 * 1000

const readDegree = (): { at: number; wire: WireDegree } | null => {
  try {
    const c = JSON.parse(window.localStorage.getItem(DEGREE_KEY) ?? 'null') as { at?: number; wire?: WireDegree } | null
    return c?.wire && Array.isArray(c.wire.rows) ? { at: c.at ?? 0, wire: c.wire } : null
  } catch { return null }
}

/**
 * The home screen's facts: what was last read (shown at once, with its age), counts, what needs attention and credits per term.
 * It then refreshes in the background, one read at a time and never in the student's way: the class list, the degree tree, and the
 * prerequisite text of each remaining course (kept for a day). It sends nothing to PeopleSoft that changes anything.
 */
/** Minutes between two background reads while a class is waitlisted and this page is open. */
export const WATCH_MIN = 10

export function HomePanel({ paused = false }: { paused?: boolean }) {
  const [snap, setSnap] = useState<Snapshot | null>(loadSnapshot)
  const [deg, setDeg] = useState(readDegree)
  const [cache, setCache] = useState<PrereqCache>(loadPrereqs)
  const [now, setNow] = useState(() => Date.now())
  const [note, setNote] = useState<string | null>('Reading PeopleSoft in the background…')
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null)
  const alive = useRef(true)
  // The student asked for something (building the schedule): the background reads stop before their next request, so they never queue in front of it.
  const pausedRef = useRef(paused)
  pausedRef.current = paused

  useEffect(() => {
    alive.current = true
    const tick = window.setInterval(() => setNow(Date.now()), 30000)
    void (async () => {
      let rows = snap?.rows ?? []
      try {
        const l = await api.classes()
        if (!alive.current) return
        rows = l.rows.map((r) => ({ course_label: r.course_label, term: r.term, status: r.status }))
        const moved = waitTransitions(snap?.rows ?? [], rows)
        const next: Snapshot = { at: Date.now(), term: openTermOf(rows) || snap?.term || '', rows, extras: snap?.extras ?? [], overlaps: snap?.overlaps ?? [], full: snap?.full ?? [], waitMoved: moved.length ? moved : snap?.waitMoved ?? [] }
        setSnap(next)
        saveSnapshot(next)
      } catch { setNote('Could not read the class list just now; showing what was last read.') }
      let wire = deg?.wire ?? null
      if (pausedRef.current) { if (alive.current) setNote('Paused while your schedule is being built.'); return }
      if (!wire || Date.now() - (deg?.at ?? 0) > DEGREE_OLD_MS) {
        try {
          const w = await api.writeRead<WireDegree>('degree_tree')
          if (!alive.current) return
          wire = w
          const d = { at: Date.now(), wire: w }
          setDeg(d)
          try { window.localStorage.setItem(DEGREE_KEY, JSON.stringify(d)) } catch { /* storage blocked */ }
        } catch { /* the last copy stays */ }
      }
      if (wire) {
        const todo: ToRead[] = []
        for (const y of new Set(wire.rows.map((r) => r.year))) {
          wire.rows.filter((r) => r.year === y).forEach((r, index) => {
            if (r.code && r.term_choice && r.has_requirement) todo.push({ code: r.code, year: y, index })
          })
        }
        const total = todo.length
        let c = loadPrereqs()
        const tried = new Set<string>()
        for (let n = nextToRead(todo, c, Date.now(), tried); n && alive.current && !pausedRef.current; n = nextToRead(todo, c, Date.now(), tried)) {
          tried.add(n.code)
          setProgress({ done: todo.filter((t) => c[t.code]).length, total })
          try {
            const r = await api.writeRead<{ course: string; text: string }>('degree_requirement', { year: n.year, index: n.index })
            // The page addresses a course by position in a tree that may be old: only text PeopleSoft says is this course's is kept.
            if (r.course.replace(/\s+/g, '') !== n.code.replace(/\s+/g, '')) continue
            c = { ...c, [n.code]: { text: r.text, at: Date.now() } }
            savePrereqs(c)
            if (alive.current) setCache(c)
          } catch { /* left for the next visit */ }
        }
        if (alive.current) setProgress(null)
      }
      if (alive.current) setNote(null)
    })()
    return () => { alive.current = false; window.clearInterval(tick) }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const waitlistOn = !!loadSettings().rules.joinWaitlist
  // Watch a waitlist: while the page is open and a class is waitlisted, read the class list again now and then (never while the schedule is being built).
  const watching = waitlistOn && countsFor(snap?.rows ?? [], snap?.term || '').waitlisted > 0
  useEffect(() => {
    if (!watching) return
    const t = window.setInterval(() => {
      if (pausedRef.current || !alive.current) return
      api.classes().then((l) => {
        if (!alive.current) return
        const rows = l.rows.map((r) => ({ course_label: r.course_label, term: r.term, status: r.status }))
        setSnap((cur) => {
          const moved = waitTransitions(cur?.rows ?? [], rows)
          const next: Snapshot = { at: Date.now(), term: openTermOf(rows) || cur?.term || '', rows, extras: cur?.extras ?? [], overlaps: cur?.overlaps ?? [], full: cur?.full ?? [], waitMoved: moved.length ? moved : cur?.waitMoved ?? [] }
          saveSnapshot(next)
          return next
        })
      }).catch(() => { /* the last read stays, with its age */ })
    }, WATCH_MIN * 60000)
    return () => window.clearInterval(t)
  }, [watching])

  const term = snap?.term || ''
  const view = useMemo(() => (deg ? buildDegree(deg.wire, term || 'Winter 2027', snap?.extras ?? []) : null), [deg, term, snap?.extras])
  const prereqs = useMemo(() => prereqMap(cache, now), [cache, now])
  const items = useMemo(() => attentionList({ term, rows: snap?.rows ?? [], view, prereqs, overlaps: snap?.overlaps ?? [], full: snap?.full ?? [], waitlistOn, waitMoved: snap?.waitMoved }), [term, snap, view, prereqs, waitlistOn])
  const counts = countsFor(snap?.rows ?? [], term)
  const loads = useMemo(() => (view ? [...termLoads(coursesOf(view), {}).values()].sort((a, b) => termOrder(a.term) - termOrder(b.term)) : []), [view])
  const age = snap ? now - snap.at : 0

  if (!snap) return <section className="v2-card" aria-label="What is known"><p className="v2-muted">{note ?? 'Nothing read yet.'}</p></section>

  return (
    <section className="v2-card v2-home" aria-label="What is known">
      <p className={age > OLD_AFTER_MS ? 'v2-alert' : 'v2-muted'}>
        {term || 'Your term'} · last read from PeopleSoft {ago(snap.at, now)}{age > OLD_AFTER_MS ? ' (old, refreshing)' : ''}{note ? ` · ${note}` : ''}
      </p>
      <p>
        <strong>{counts.enrolled}</strong> enrolled · <strong>{counts.planned}</strong> planned with classes
        {counts.noClasses ? <> · <strong>{counts.noClasses}</strong> planned without classes</> : null}
      </p>
      <h2>Needs your attention <span className="v2-muted">{items.length}</span></h2>
      {items.length === 0 ? <p className="v2-muted">Nothing found{progress ? ' so far' : ''}.</p> : (
        <ul className="v2-attention">{items.map((i, n) => <li key={n} className={`is-${i.kind}`}>{i.text}</li>)}</ul>
      )}
      {progress ? <p className="v2-muted">Checking prerequisites: {progress.done} of {progress.total}…</p> : null}
      {waitlistOn && counts.waitlisted > 0 ? <p className="v2-muted">Waitlist last checked {ago(snap.at, now)}. It is read again every {WATCH_MIN} minutes while this page is open.</p> : null}
      {loads.length ? (
        <details>
          <summary>Credits by term</summary>
          <table className="v2-credits__table">
            <thead><tr><th>Term</th><th>Credits</th><th>Limit</th></tr></thead>
            <tbody>
              {loads.map((l) => (
                <tr key={l.term} className={l.over ? 'is-over' : undefined}>
                  <td>{l.term}</td>
                  <td>{creditText(l.credits)}{l.deferred.length ? ` (${l.deferred.map((d) => d.code).join(', ')} set aside)` : ''}</td>
                  <td>{l.limit.max}{l.limit.confirmed ? '' : ' (not confirmed)'}{l.over ? ' – over' : ''}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </details>
      ) : null}
    </section>
  )
}

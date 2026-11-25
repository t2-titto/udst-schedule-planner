import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { api } from '../api'
import { buildDegree, type ExtraCourse, type WireDegree } from './degree'
import { creditText } from './credits'
import { formatRuleTree, getCatalogRule } from './catalogRules'
import { loadOf, termLoads, coursesOf } from './draft'
import { executeMoves, msg } from './runMoves'
import { normalizeCourses } from '../engine/normalize'
import type { PCourse } from './model'
import { parseUnits } from './model'
import { planSave } from './savePlan'
import {
  availabilityOf, candidates, CATALOG_KEY, loadCatalog, nextToCheck, normCodeKey, remember, saveCatalog, searchCandidates, termCode,
  type Candidate, type CatalogCache,
} from './ready'

interface Props {
  /** The open term ("Winter 2027"). */
  term: string
  /** Codes already in the schedule. */
  inSchedule: string[]
  /** What PeopleSoft counts in the open term besides the tree's rows (see App). */
  extras: ExtraCourse[]
  /** Enrolled courses the schedule is dropping: they no longer count towards the term's credits. */
  dropping?: string[]
  /** Called after a course was saved into PeopleSoft's plan: the schedule must be read again. */
  onAdded: () => void
  /** The course goes into the schedule only; Apply plans it in PeopleSoft and then enrols it. */
  onKeep: (c: PCourse) => void
  onBack: () => void
}

const DEGREE_KEY = 'schedule-planner:v2:degree'
const readDegreeCache = (): WireDegree | null => {
  try {
    const c = JSON.parse(window.localStorage.getItem(DEGREE_KEY) ?? 'null') as { wire?: WireDegree } | null
    return c?.wire && Array.isArray(c.wire.rows) ? c.wire : null
  } catch { return null }
}

/** Degree courses with classes this term that are not in the schedule yet: a ready list (checked in the background) and a search box. */
export function AddCourse({ term, inSchedule, extras, dropping = [], onAdded, onKeep, onBack }: Props) {
  const [wire, setWire] = useState<WireDegree | null>(readDegreeCache)
  const [cache, setCache] = useState<CatalogCache>(loadCatalog)
  // Adding is only offered on a degree tree PeopleSoft gave THIS visit (the copy in the browser's memory has no age limit).
  const [treeFresh, setTreeFresh] = useState(false)
  const [checking, setChecking] = useState<string | null>(null)
  const [failed, setFailed] = useState<Record<string, string>>({})
  const [text, setText] = useState('')
  const [pick, setPick] = useState<Candidate | null>(null)
  const [saving, setSaving] = useState<'idle' | 'working' | 'done'>('idle')
  const [error, setError] = useState<string | null>(null)
  const [now, setNow] = useState(() => Date.now())
  const [sections, setSections] = useState<{ options: { option_no: string; status: string; components: { kind: string; schedule_text: string; seats_text: string; instructor: string }[] }[] } | 'loading' | 'failed' | null>(null)
  const cacheRef = useRef(cache)
  cacheRef.current = cache
  const stop = useRef(false)

  useEffect(() => {
    stop.current = false
    return () => { stop.current = true }
  }, [])

  // The degree tree: from the last copy at once, then read again in the background.
  useEffect(() => {
    let alive = true
    api.writeRead<WireDegree>('degree_tree').then((w) => {
      if (!alive) return
      setWire(w)
      setTreeFresh(true)
      try { window.localStorage.setItem(DEGREE_KEY, JSON.stringify({ at: Date.now(), wire: w })) } catch { /* storage blocked */ }
    }).catch(() => { /* the last copy stays */ })
    return () => { alive = false }
  }, [])

  const dropKey = dropping.join('|')
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const view = useMemo(() => (wire ? buildDegree(wire, term, extras, dropping) : null), [wire, term, extras, dropKey])
  const list = useMemo(() => (view ? candidates(view, term, inSchedule) : []), [view, term, inSchedule])
  const strm = termCode(term)

  const checkOne = useCallback(async (c: Candidate) => {
    if (!strm) return
    const code = normCodeKey(c.code)
    setChecking(c.code)
    try {
      let key = cacheRef.current.keys[code]
      if (!key) {
        const hits = await api.writeRead<{ code: string; crse_id: string; offer_nbr: string }[]>('catalog_search', { keyword: c.code })
        const hit = hits.find((h) => normCodeKey(h.code) === code)
        if (!hit) throw new Error('not found in the course catalog')
        key = { crse_id: hit.crse_id, offer_nbr: hit.offer_nbr }
        const next = { ...cacheRef.current, keys: { ...cacheRef.current.keys, [code]: key } }
        cacheRef.current = next
        setCache(next)
        saveCatalog(next)
      }
      const read = () => api.writeRead<{ options: unknown[] }>('catalog_sections', { crse_id: key!.crse_id, offer_nbr: key!.offer_nbr, strm })
      let sections
      try { sections = await read() } catch (e) {
        // A cold session cannot open a course page directly: the class list read warms it, then once more.
        await api.writeRead('plan_class_list')
        sections = await read()
        void e
      }
      const next = remember(cacheRef.current, term, c.code, sections.options.length, Date.now())
      cacheRef.current = next
      setCache(next)
      saveCatalog(next)
    } catch (e) {
      setFailed((f) => ({ ...f, [code]: msg(e) }))
    } finally {
      setChecking(null)
    }
  }, [strm, term])

  // One course at a time, in list order, until every candidate has a fresh answer. The page stays usable meanwhile.
  useEffect(() => {
    if (!view || checking || stop.current) return
    const next = nextToCheck(list, cache, term, Date.now(), new Set(Object.keys(failed)))
    if (next) void checkOne(next)
  }, [view, list, cache, term, checking, failed, checkOne])

  useEffect(() => {
    const t = window.setInterval(() => setNow(Date.now()), 30000)
    return () => window.clearInterval(t)
  }, [])

  const state = (c: Candidate) => (failed[normCodeKey(c.code)] ? ({ state: 'failed', why: failed[normCodeKey(c.code)] } as const) : checking === c.code ? ({ state: 'checking' } as const) : availabilityOf(cache, term, c.code, now))
  const shown = searchCandidates(list, text)
  const open = shown.filter((c) => state(c).state === 'open')
  const pending = shown.filter((c) => ['unknown', 'checking'].includes(state(c).state))
  const closed = shown.filter((c) => state(c).state === 'closed')
  const bad = shown.filter((c) => state(c).state === 'failed')

  // What adding the picked course to the open term would do to the credits, worked out by the same code that saves.
  const effect = useMemo(() => {
    if (!pick || !view || !wire) return null
    const draft = { [pick.course.row]: term }
    const p = planSave(view, wire, term, draft)
    const load = loadOf(termLoads(coursesOf(view), {}), term)
    return { plan: p, before: load.credits, limit: load.limit.max, move: p.moves[0] ?? null }
  }, [pick, view, wire, term])

  useEffect(() => {
    setSections(null)
    if (!pick || !strm) return
    const key = cacheRef.current.keys[normCodeKey(pick.code)]
    if (!key) return
    let alive = true
    setSections('loading')
    api.writeRead<NonNullable<Exclude<typeof sections, 'loading' | 'failed'>>>('catalog_sections', { crse_id: key.crse_id, offer_nbr: key.offer_nbr, strm })
      .then((v) => { if (alive) setSections(v) })
      .catch(() => { if (alive) setSections('failed') })
    return () => { alive = false }
  }, [pick, strm])

  // Keep it in the schedule only: its classes from the catalog become a course of the schedule that Apply will plan and then enrol.
  const keepInSchedule = () => {
    if (!treeFresh || !pick || !effect?.move || !sections || sections === 'loading' || sections === 'failed') return
    const k = cacheRef.current.keys[normCodeKey(pick.code)]
    if (!k || !strm) return
    const key = { strm, crse_id: k.crse_id, crse_offer_nbr: k.offer_nbr, acad_career: 'CRED', institution: 'CNAQA' }
    const course = normalizeCourses({ courses: [{ key, sections }] }).courses[0]
    if (!course || !course.options.length) { setError('Its classes could not be read, so it cannot be added to the schedule.'); return }
    const m = effect.move
    const c: PCourse = {
      code: pick.code, title: pick.title, term, key: course.key, options: course.options.filter((o) => o.status.toLowerCase() === 'open'),
      units: parseUnits(course.units) ?? pick.units,
      addPlan: { year: m.year, index: m.index, toCode: m.toCode, to: m.to },
    }
    if (!c.options.length) { setError('None of its classes are open right now.'); return }
    onKeep(c)
  }

  const saveNow = async () => {
    if (!treeFresh || !effect?.move || !wire) return
    setSaving('working')
    setError(null)
    const { error: failedMsg } = await executeMoves([effect.move], () => {})
    if (failedMsg) { setError(failedMsg); setSaving('idle'); return }
    setSaving('done')
    onAdded()
  }

  return (
    <div className="v2">
      <header className="v2-bar">
        <h1>Add a course to {term}</h1>
        <span className="v2-status">{checking ? `checking ${checking}…` : pending.length ? `${pending.length} still to check` : 'all checked'}</span>
        <span className="v2-spacer" />
        <button type="button" className="btn btn-quiet" onClick={onBack}>Back</button>
      </header>
      <main className="v2-degree">
        <p className="v2-muted">Only courses of your own degree are listed. PeopleSoft's catalog is read in the background (the first time takes a few minutes; answers are kept for 6 hours).</p>
        <label className="v2-yearpick">Search your degree courses{' '}
          <input type="search" value={text} onChange={(e) => setText(e.target.value)} placeholder="code or title" aria-label="Search your degree courses" />
        </label>
        {!view ? <p className="v2-muted">Reading your degree from PeopleSoft…</p> : null}

        {pick && effect ? (
          <section className="v2-card v2-save" aria-label="Add this course">
            <h2>{pick.code} {pick.title}</h2>
            <p>{term} has <strong>{creditText(effect.before)}</strong> credits now (limit {effect.limit}). With this course: <strong>{creditText(effect.move?.after ?? effect.before + (pick.units ?? 0))}</strong>.</p>
            {(() => {
              const r = getCatalogRule(pick.code)
              if (!r) return null
              const parts: string[] = []
              if (r.prerequisites) parts.push(`Prerequisites: ${formatRuleTree(r.prerequisites)}`)
              if (r.corequisites?.length) parts.push(`Co-requisite: ${r.corequisites.join(', ')}`)
              if (r.cohort_rules?.length) parts.push(`Cohort rules: ${r.cohort_rules.map((c) => `${c.condition}: ${formatRuleTree(c.prerequisites)}`).join('; ')}`)
              if (r.notes?.length) parts.push(`Notes: ${r.notes.join('; ')}`)
              return parts.length ? <p className="v2-muted">{parts.join(' · ')}</p> : null
            })()}
            {effect.plan.stuck ? (
              <p className="v2-alert" role="alert">
                That would put {term} over its {effect.limit} credits. Free some credits first: move a course that is only planned (not enrolled) to another term in "Plan my degree", or drop an enrolled one (which needs care). PeopleSoft does not let a required course be left without a term, so a planned course can only be moved, not removed. Nothing was changed.
              </p>
            ) : null}
            {!treeFresh ? <p className="v2-muted">Waiting for PeopleSoft's current plan before you can add (the copy on this device may be out of date).</p> : null}
            {sections === 'loading' ? <p className="v2-muted">Reading its classes from the catalog…</p> : null}
            {sections === 'failed' ? <p className="v2-muted">Could not read its classes right now.</p> : null}
            {sections && sections !== 'loading' && sections !== 'failed' ? (
              <ul className="v2-savelist" aria-label="Classes this term">
                {sections.options.map((o) => (
                  <li key={o.option_no}>{o.components.map((c) => `${c.kind}: ${c.schedule_text || 'no time'}${c.seats_text ? ` (${c.seats_text})` : ''}`).join(' · ')} <span className="v2-muted">{o.status}</span></li>
                ))}
              </ul>
            ) : null}
            {error ? <p className="v2-alert" role="alert">{error}</p> : null}
            {saving === 'done' ? <p>Saved in your PeopleSoft plan. The schedule is being read again.</p> : null}
            <div className="v2-actions">
              <button type="button" className="btn" disabled={!treeFresh || !!effect.plan.stuck || !effect.move || saving !== 'idle'} onClick={() => void saveNow()}>
                {saving === 'working' ? 'Saving…' : 'Save to my PeopleSoft plan now'}
              </button>
              <button type="button" className="btn btn-quiet" disabled={!treeFresh || !!effect.plan.stuck || !effect.move || saving !== 'idle' || sections === null || sections === 'loading' || sections === 'failed'} onClick={keepInSchedule}>Keep it in my schedule only</button>
              <button type="button" className="btn btn-quiet" disabled={saving === 'working'} onClick={() => { setPick(null); setError(null); setSaving('idle') }}>Cancel</button>
            </div>
            <p className="v2-muted">Saving plans it in {term} in PeopleSoft now (course planning only; it does not enrol you). Keeping it in the schedule changes nothing in PeopleSoft until you press Apply: then it is planned and enrolled in the one review.</p>
          </section>
        ) : null}

        <h2>Open this term for you <span className="v2-muted">{open.length}</span></h2>
        {open.length === 0 ? <p className="v2-muted">{pending.length ? 'Still checking…' : 'None of your remaining degree courses have classes this term.'}</p> : null}
        <div className="v2-blocks v2-blocks--wide">
          {open.map((c) => {
            const a = state(c)
            return (
              <div key={c.code} className="v2-coursecard">
                <div><strong>{c.code}</strong> {c.title}</div>
                <div className="v2-muted">{c.units === null ? 'units unknown' : `${creditText(c.units)} cr`} · planned for {c.plannedTerm ?? 'no term'}{a.state === 'open' ? ` · ${a.options} class combinations` : ''}</div>
                <button type="button" className="btn btn-sm" onClick={() => { setPick(c); setError(null); setSaving('idle') }}>Add…</button>
              </div>
            )
          })}
        </div>

        {pending.length ? <p className="v2-muted">Still to check: {pending.map((c) => c.code).join(', ')}</p> : null}
        {closed.length ? <details><summary>No classes this term ({closed.length})</summary><p className="v2-muted">{closed.map((c) => c.code).join(', ')}</p></details> : null}
        {bad.length ? <details><summary>Could not check ({bad.length})</summary><ul>{bad.map((c) => <li key={c.code}>{c.code}: {failed[normCodeKey(c.code)]}</li>)}</ul></details> : null}
        <p className="v2-muted" hidden>{CATALOG_KEY}</p>
      </main>
    </div>
  )
}

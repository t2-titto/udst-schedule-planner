import { useCallback, useEffect, useMemo, useState } from 'react'
import { api, ApiError } from '../api'
import { buildDegree, type ExtraCourse, type DegreeCourse, type WireCourseList, type WireDegree, type WireRequirement } from './degree'
import { creditText } from './credits'
import { prerequisiteOf } from './prereq'
import { ago } from './Planner'
import { PlanBoard } from './PlanBoard'
import { PlanSave } from './PlanSave'
import { coursesOf, EMPTY_DRAFT, termLoads, type Draft } from './draft'
import { termOrder } from './data'
import { commit, redo, reset, startHistory, undo, type History } from './history'

interface Props {
  /** The open term ("Winter 2027"): what is completed, in progress or planned is judged against it. */
  term: string
  /** Courses PeopleSoft counts in a term that the tree may not list (from the schedule's own data). */
  extras?: ExtraCourse[]
  /** False when the schedule has not been loaded: the term totals then miss courses chosen inside Course Lists, so Save stays off. */
  extrasKnown?: boolean
  onBack: () => void
}

const KEY = 'schedule-planner:v2:degree'

function loadCache(): { at: number; wire: WireDegree } | null {
  try {
    const c = JSON.parse(window.localStorage.getItem(KEY) ?? 'null') as { at: number; wire: WireDegree } | null
    return c && typeof c.at === 'number' && c.wire && Array.isArray(c.wire.rows) ? c : null
  } catch {
    return null
  }
}

const STATUS_TEXT: Record<DegreeCourse['status'], string> = { completed: 'Completed', inProgress: 'In progress', planned: 'Planned', unplanned: 'Not planned' }

/** The degree the way PeopleSoft has it, read-only: years, semesters, courses, credits per term, progress, prerequisites and course-list choices on request. */
export function PlanDegree({ term, extras = [], extrasKnown = true, onBack }: Props) {
  const cached = useMemo(loadCache, [])
  const [wire, setWire] = useState<WireDegree | null>(cached?.wire ?? null)
  const [at, setAt] = useState<number | null>(cached?.at ?? null)
  const [reading, setReading] = useState(false)
  // A write is only offered on a tree PeopleSoft gave THIS visit: the copy from the browser's memory has no age limit and may be out of date.
  const [fresh, setFresh] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [now, setNow] = useState(() => Date.now())
  const [year, setYear] = useState(0)
  const [saving, setSaving] = useState(false)
  const [mode, setMode] = useState<'board' | 'university'>('board')
  const [hist, setHist] = useState<History<Draft>>(() => startHistory(EMPTY_DRAFT))
  const [reqs, setReqs] = useState<Record<string, WireRequirement | 'loading' | 'failed'>>({})
  const [lists, setLists] = useState<Record<string, WireCourseList | 'loading' | 'failed'>>({})

  const read = useCallback(async () => {
    setReading(true)
    setFresh(false)
    setError(null)
    try {
      const w = await api.writeRead<WireDegree>('degree_tree')
      setWire(w)
      setAt(Date.now())
      setFresh(true)
      try { window.localStorage.setItem(KEY, JSON.stringify({ at: Date.now(), wire: w })) } catch { /* storage blocked */ }
    } catch (e) {
      setError(e instanceof ApiError ? e.message : 'Could not read your degree. What is shown is the last copy, if any.')
    } finally {
      setReading(false)
    }
  }, [])
  useEffect(() => { void read() }, [read])
  useEffect(() => {
    const t = window.setInterval(() => setNow(Date.now()), 30000)
    return () => window.clearInterval(t)
  }, [])

  const view = useMemo(() => (wire ? buildDegree(wire, term, extras) : null), [wire, term, extras])
  const indexInYear = (y: number, row: number) => (wire ? wire.rows.filter((r) => r.year === y).findIndex((r) => r.row === row) : -1)

  const loadReq = async (c: DegreeCourse) => {
    const k = `${c.year}:${c.row}`
    setReqs((r) => ({ ...r, [k]: 'loading' }))
    try {
      const v = await api.writeRead<WireRequirement>('degree_requirement', { year: c.year, index: indexInYear(c.year, c.row) })
      // The position may belong to another course if this copy of the tree is old: text that is not this course's is never shown as its own.
      if (v.course.replace(/\s+/g, '') !== c.code.replace(/\s+/g, '')) throw new Error('another course')
      setReqs((r) => ({ ...r, [k]: v }))
    } catch {
      setReqs((r) => ({ ...r, [k]: 'failed' }))
    }
  }
  const loadList = async (y: number, row: number) => {
    const k = `${y}:${row}`
    setLists((l) => ({ ...l, [k]: 'loading' }))
    try {
      const v = await api.writeRead<WireCourseList>('degree_course_list', { year: y, index: indexInYear(y, row) })
      setLists((l) => ({ ...l, [k]: v }))
    } catch {
      setLists((l) => ({ ...l, [k]: 'failed' }))
    }
  }

  // A Course List's chosen course (it has a real term) counts towards that term; this is only known once the list has been read.
  const listExtra = useMemo(() => {
    const m = new Map<string, { credits: number; codes: string[] }>()
    for (const l of Object.values(lists)) {
      if (!l || l === 'loading' || l === 'failed') continue
      for (const c of l.choices) {
        const u = c.units === null ? null : Number(c.units)
        if (!/^(Fall|Winter|Spring) \d{4}$/.test(c.term) || u === null || Number.isNaN(u)) continue
        if (view?.extras.some((x) => x.code.replace(/\s+/g, '') === c.code.replace(/\s+/g, ''))) continue // already counted from the schedule's data
        const e = m.get(c.term) ?? { credits: 0, codes: [] }
        e.credits += u
        e.codes.push(`${c.code} (from a list)`)
        m.set(c.term, e)
      }
    }
    return m
  }, [lists, view])

  // The same numbers (and the same planning-semester rule) as the board, so the two never disagree about a term being over.
  const termTable = useMemo(() => (view ? [...termLoads(coursesOf(view), {}).values()].sort((a, b) => termOrder(a.term) - termOrder(b.term)) : []), [view])
  const prereqs = useMemo(() => {
    const m = new Map<string, NonNullable<ReturnType<typeof prerequisiteOf>>>()
    if (!view) return m
    for (const y of view.years) for (const sm of y.semesters) for (const b of sm.blocks) {
      if (b.kind !== 'course') continue
      const r = reqs[`${b.course.year}:${b.course.row}`]
      const p = r && r !== 'loading' && r !== 'failed' ? prerequisiteOf(b.course.code, r.text) : null
      if (p) m.set(b.course.code, p)
    }
    return m
  }, [view, reqs])

  const doneCodes = useMemo(() => new Set(view ? view.years.flatMap((y) => y.semesters).flatMap((s) => s.blocks).flatMap((b) => (b.kind === 'course' && (b.course.status === 'completed' || b.course.status === 'inProgress') ? [b.course.code] : [])) : []), [view])

  return (
    <div className="v2">
      <header className="v2-bar">
        <h1>Plan my degree</h1>
        <span className="v2-status">{term}{reading ? ' · reading PeopleSoft…' : at ? ` · updated ${ago(at, now)}` : ''}</span>
        <span className="v2-spacer" />
        <button type="button" className="btn btn-quiet" disabled={reading} onClick={() => void read()}>↻ Refresh</button>
        <button type="button" className="btn btn-quiet" onClick={onBack}>Back</button>
      </header>
      <main className="v2-degree">
        {error ? <p className="v2-alert" role="alert">{error}</p> : null}
        {view ? (
          <div className="v2-degree__years" role="tablist" aria-label="View">
            <button type="button" role="tab" aria-selected={mode === 'board'} className="btn btn-quiet btn-sm" onClick={() => setMode('board')}>My terms (draft)</button>
            <button type="button" role="tab" aria-selected={mode === 'university'} className="btn btn-quiet btn-sm" onClick={() => setMode('university')}>University planning</button>
          </div>
        ) : null}
        {!view ? <p className="v2-muted">{reading ? 'Reading your degree from PeopleSoft (about 25 seconds the first time)…' : 'Nothing read yet.'}</p> : mode === 'board' && wire ? (saving ? (
          <PlanSave view={view} wire={wire} openTerm={term} draft={hist.present}
            onCancel={() => setSaving(false)}
            onDone={(fresh, clean) => {
              if (fresh) {
                setWire(fresh)
                setAt(Date.now())
                try { window.localStorage.setItem(KEY, JSON.stringify({ at: Date.now(), wire: fresh })) } catch { /* storage blocked */ }
                // Keep only the draft changes PeopleSoft does not have yet (a partly saved draft keeps its unsent moves).
                const label = (row: number) => { const t = fresh.rows.find((r) => r.row === row)?.term_choice; return t?.options.find((o) => o.code === t.selected)?.label ?? null }
                const left: Draft = Object.fromEntries(Object.entries(hist.present).filter(([row, to]) => label(Number(row)) !== to))
                setHist(reset(clean ? EMPTY_DRAFT : left))
              } else if (clean) setHist(reset(EMPTY_DRAFT))
              setSaving(false)
            }} />
        ) : (
          <PlanBoard canSave={extrasKnown && fresh && !reading} saveBlockedWhy={!extrasKnown ? 'schedule' : !fresh || reading ? 'reading' : null} view={view} wire={wire} openTerm={term} prereqs={prereqs} history={hist}
            onChange={(next, tag) => setHist((h) => commit(h, next, Date.now(), tag))}
            onUndo={() => setHist((h) => undo(h))} onRedo={() => setHist((h) => redo(h))} onDiscard={() => setHist(reset(EMPTY_DRAFT))} onSave={() => setSaving(true)} />
        )) : (
          <>
            <section className="v2-degree__top" aria-label="Progress and credits by term">
              <div className="v2-card">
                <h2>Progress</h2>
                <p>
                  <strong>{creditText(view.progress.completed)}</strong> credits completed · <strong>{creditText(view.progress.inProgress)}</strong> in progress · <strong>{creditText(view.progress.planned)}</strong> planned · <strong>{creditText(view.progress.unplanned)}</strong> not planned
                  {' '}(of {creditText(view.progress.total)} in the listed courses{view.progress.unknown ? `, ${view.progress.unknown} with unknown credits` : ''}).
                </p>
                <p className="v2-muted">A "+" after a term's credits means a Course List may add more: open its choices ("Show the choices") and the course chosen in that term is added. PeopleSoft's own completion figures are shown on each year and semester.</p>
              </div>
              <div className="v2-card">
                <h2>Credits by term</h2>
                <table className="v2-credits__table">
                  <thead><tr><th>Term</th><th>Credits</th><th>Limit</th><th>Courses</th></tr></thead>
                  <tbody>
                    {termTable.map((t) => {
                      const x = listExtra.get(t.term)
                      const total = t.credits + (x?.credits ?? 0)
                      const over = total > t.limit.max
                      return (
                        <tr key={t.term} className={over ? 'is-over' : undefined}>
                          <td>{t.term}</td>
                          <td>{creditText(total)}{x && !t.unknown.length ? '' : '+'}</td>
                          <td>{t.limit.max}{t.limit.confirmed ? '' : ' (not confirmed)'}{over ? ' – over' : ''}</td>
                          <td>{[...t.courses.map((c) => c.code), ...(x?.codes ?? [])].join(', ')}{t.deferred.length ? ` (not counted here: ${t.deferred.map((c) => c.code).join(', ')}, which belong to another term)` : ''}{t.unknown.length ? ` (no credits listed: ${t.unknown.map((c) => c.code).join(', ')})` : ''}</td>
                        </tr>
                      )
                    })}
                  </tbody>
                </table>
              </div>
            </section>

            <div className="v2-degree__years" role="tablist" aria-label="Year">
              {view.years.map((y) => (
                <button key={y.index} type="button" role="tab" aria-selected={year === y.index} className="btn btn-quiet btn-sm" onClick={() => setYear(y.index)}>{y.name.replace(/^\S+\s/, '')}</button>
              ))}
            </div>
            {view.years.filter((y) => y.index === year).map((y) => (
              <section key={y.index} aria-label={y.name}>
                <h2>{y.name} <span className="v2-muted">{y.summary}</span></h2>
                <div className="v2-degree__sems">
                  {y.semesters.map((s) => (
                    <div key={s.index ?? 'x'} className="v2-card v2-sem">
                      <h3>{s.name.replace(/^\S+\s/, '')} <span className="v2-muted">{creditText(s.credits)} credits listed</span></h3>
                      {s.summary ? <p className="v2-muted">{s.summary}</p> : null}
                      <div className="v2-blocks">
                        {s.blocks.map((b) => {
                          if (b.kind === 'group') {
                            const k = `${b.group.year}:${b.group.row}`
                            const l = lists[k]
                            return (
                              <div key={`g${b.group.row}`} className="v2-bigblock">
                                <strong>{b.group.name}</strong>
                                <span className="v2-muted">Choose from a list{b.group.hasSelect ? '' : ''}</span>
                                {l === undefined ? <button type="button" className="btn btn-quiet btn-sm" onClick={() => void loadList(b.group.year, b.group.row)}>Show the choices</button> : null}
                                {l === 'loading' ? <span className="v2-muted">Reading the list (about 15 s)…</span> : null}
                                {l === 'failed' ? <span className="v2-alert">Could not read it. <button type="button" className="btn btn-quiet btn-sm" onClick={() => void loadList(b.group.year, b.group.row)}>Try again</button></span> : null}
                                {l && l !== 'loading' && l !== 'failed' ? (
                                  <div className="v2-smallblocks">
                                    {l.choices.map((c, i) => (
                                      <div key={c.row} className="v2-smallblock">
                                        {i > 0 ? <em className="v2-join">{c.joined_by === 'and' ? 'and' : 'or'}</em> : null}
                                        <strong>{c.code}</strong> {c.title} <span className="v2-muted">{c.units ?? '?'} cr · {c.term ? c.term : 'not taken'}</span>
                                      </div>
                                    ))}
                                  </div>
                                ) : null}
                              </div>
                            )
                          }
                          const c = b.course
                          const k = `${c.year}:${c.row}`
                          const r = reqs[k]
                          const pre = r && r !== 'loading' && r !== 'failed' ? prerequisiteOf(c.code, r.text) : null
                          return (
                            <div key={`c${c.row}`} className={`v2-coursecard is-${c.status}`}>
                              <div><strong>{c.code}</strong> {c.title}</div>
                              <div className="v2-muted">{c.units === null ? 'units unknown' : `${creditText(c.units)} cr`} · {c.category || '—'} · {STATUS_TEXT[c.status]}{c.term ? `: ${c.term}` : ''}</div>
                              {c.hasRequirement && r === undefined ? <button type="button" className="v2-linkbtn" onClick={() => void loadReq(c)}>Prerequisite</button> : null}
                              {r === 'loading' ? <span className="v2-muted">Reading (about 12 s)…</span> : null}
                              {r === 'failed' ? <span className="v2-alert">Could not read it. <button type="button" className="v2-linkbtn" onClick={() => void loadReq(c)}>Try again</button></span> : null}
                              {r && r !== 'loading' && r !== 'failed' ? (
                                <p className="v2-prereq">
                                  {pre ? <>Needs {pre.op === 'and' ? 'all of' : pre.op === 'or' ? 'one of' : ''} {[...pre.courses, ...pre.notes].join(pre.op === 'and' ? ' and ' : ' or ')}. {pre.courses.length && pre.notes.length === 0 ? (pre.courses.every((x) => doneCodes.has(x)) ? 'You have it.' : pre.op === 'or' && pre.courses.some((x) => doneCodes.has(x)) ? 'You have one.' : 'Not met yet.') : ''} </> : null}
                                  <span className="v2-muted">PeopleSoft says: {r.text}</span>
                                </p>
                              ) : null}
                            </div>
                          )
                        })}
                      </div>
                    </div>
                  ))}
                </div>
              </section>
            ))}
          </>
        )}
      </main>
    </div>
  )
}

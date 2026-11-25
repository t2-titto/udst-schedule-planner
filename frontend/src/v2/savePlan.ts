import type { PlanCreditsReq } from '../api'
import type { DegreeCourse, WireDegree } from './degree'
import { limitFor } from './credits'
import { allowedTerms, coursesOf, loadOf, moveCourse, termLoads, type Draft } from './draft'
import type { DegreeView } from './degree'

/** One planned course going to another term, in the form the server takes. */
export interface Move {
  course: DegreeCourse
  from: string | null
  to: string
  toCode: string
  fromCode: string
  year: number
  /** Position among that Year's rows (PeopleSoft numbers rows over whatever is expanded, so a row number would not do). */
  index: number
  /** The destination term's total after this move, given the moves ordered before it. */
  after: number
  unknown: string[]
}

export interface SavePlanned {
  moves: Move[]
  /** Set when no order of the moves keeps every term within its limit at every step. */
  stuck: string | null
}

const codeOf = (wire: WireDegree, row: number, label: string): string | null => {
  const r = wire.rows.find((x) => x.row === row)
  return r?.term_choice?.options.find((o) => o.label === label)?.code ?? null
}

/**
 * The moves of a draft, in an order where every term stays within its limit after each one (a move out of a full term goes first).
 * PeopleSoft's Save accepts an over-limit plan, so the order matters: this app never leaves an intermediate state over the limit.
 */
export function planSave(view: DegreeView, wire: WireDegree, openTerm: string, draft: Draft): SavePlanned {
  const courses = coursesOf(view)
  const wanted = courses.filter((c) => c.row in draft)
  const optionsOf = (c: DegreeCourse) => wire.rows.find((r) => r.row === c.row)?.term_choice?.options.map((o) => ({ term: o.label, code: o.code })) ?? []
  let applied: Draft = {}
  const left = [...wanted]
  const moves: Move[] = []
  while (left.length) {
    let picked = -1
    let pickedAfter = 0
    let pickedUnknown: string[] = []
    for (let i = 0; i < left.length; i++) {
      const c = left[i]
      const to = draft[c.row]
      const next = moveCourse(applied, c, to, allowedTerms(c, optionsOf(c), openTerm))
      // Strict: a plan WRITE never relies on the planning-semester rule (it was never verified against PeopleSoft's Enrol click; the server is strict too).
      const load = loadOf(termLoads(courses, next, false), to)
      const unknown = load.unknown.map((x) => x.code)
      if (load.credits <= limitFor(to).max + 1e-9) {
        picked = i
        pickedAfter = load.credits
        pickedUnknown = unknown
        break
      }
    }
    if (picked < 0) return { moves, stuck: `${left.map((c) => `${c.code} to ${draft[c.row]}`).join(', ')}: every order would put a term over its credit limit at some step.` }
    const c = left.splice(picked, 1)[0]
    const to = draft[c.row]
    applied = moveCourse(applied, c, to, allowedTerms(c, optionsOf(c), openTerm))
    const inYear = wire.rows.filter((r) => r.year === c.year)
    moves.push({
      course: c,
      from: c.term,
      to,
      toCode: codeOf(wire, c.row, to) ?? '',
      fromCode: wire.rows.find((r) => r.row === c.row)?.term_choice?.selected ?? '',
      year: c.year,
      index: inYear.findIndex((r) => r.row === c.row),
      after: pickedAfter,
      unknown: pickedUnknown,
    })
  }
  return { moves, stuck: null }
}

/** The server's request for one move. */
export function moveRequest(m: Move): { args: Record<string, number | string>; credits: PlanCreditsReq; expect: string[] } {
  return {
    args: { year: m.year, index: m.index, term_code: m.toCode },
    // The course the student chose, by code: the server refuses the move when PeopleSoft's own tree has another course at that position.
    expect: [m.course.code.replace(/[^A-Za-z0-9]/g, '')],
    credits: { term: m.to, limit: limitFor(m.to).max, after: m.after, unknown: m.unknown },
  }
}

/** Page-side problems with a move, found before anything is sent. */
export function moveProblems(m: Move): string[] {
  const out: string[] = []
  if (!m.toCode) out.push(`${m.course.code}: PeopleSoft does not offer ${m.to} for it.`)
  if (m.index < 0) out.push(`${m.course.code}: its row could not be found.`)
  if (m.unknown.length) out.push(`${m.to}: the credits of ${m.unknown.join(', ')} are unknown.`)
  return out
}

/**
 * After all moves: the whole plan read again from PeopleSoft against what was expected (every course where it was, except the moved ones).
 * This is the proof across all Years; each single move only proves its own Year.
 */
export function compareTrees(before: WireDegree, moves: Pick<Move, 'course' | 'toCode'>[], after: WireDegree): string[] {
  const problems: string[] = []
  if (before.rows.length !== after.rows.length) problems.push(`PeopleSoft now lists ${after.rows.length} rows, it listed ${before.rows.length} before.`)
  const moved = new Map(moves.map((m) => [m.course.code, m.toCode]))
  const key = (r: WireDegree['rows'][number]) => r.code || r.label
  const afterBy = new Map(after.rows.map((r) => [key(r), r]))
  for (const b of before.rows) {
    const a = afterBy.get(key(b))
    if (!a) { problems.push(`${key(b)} is missing from PeopleSoft's plan.`); continue }
    const was = b.term_choice?.selected ?? null
    const now = a.term_choice?.selected ?? null
    const want = moved.has(key(b)) ? moved.get(key(b))! : was
    if (now !== want) problems.push(`${key(b)}: PeopleSoft has ${now ?? 'a fixed term'}, expected ${want ?? 'a fixed term'}.`)
  }
  return problems
}

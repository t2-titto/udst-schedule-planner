import { termOrder } from './data'
import { limitFor, type TermLimit } from './credits'
import type { DegreeCourse, DegreeView } from './degree'
import { collectRuleCodes, evalRuleTree, formatRuleTree, getCatalogRule } from './catalogRules'
import { isMet, normCode, type Prerequisite } from './prereq'

/**
 * The draft: only the courses the student moved, as row -> term name ("Spring 2027").
 * A row that is put back where PeopleSoft has it is removed, so an empty draft means "no changes".
 * Nothing here talks to PeopleSoft.
 */
export type Draft = Readonly<Record<number, string>>

export const EMPTY_DRAFT: Draft = {}

export interface TermOption {
  term: string
  code: string
}

/** Every course of the degree, in the order PeopleSoft lists them. */
export const coursesOf = (view: DegreeView): DegreeCourse[] => [
  ...view.years.flatMap((y) => y.semesters).flatMap((s) => s.blocks).flatMap((b) => (b.kind === 'course' ? [b.course] : [])),
  ...view.extras,
]

/** What PeopleSoft has right now for this course (null: not planned). */
export const originalTerm = (c: DegreeCourse): string | null => c.term

export const termOf = (c: DegreeCourse, draft: Draft): string | null => (c.row in draft ? draft[c.row] : originalTerm(c))

/** A course can be moved when PeopleSoft gives it a term dropdown and it is not taken or in progress. */
export const movable = (c: DegreeCourse): boolean => c.plannable && (c.status === 'planned' || c.status === 'unplanned')

/** The terms a course may go to: the ones PeopleSoft offers, from the open term onward. */
export function allowedTerms(c: DegreeCourse, options: TermOption[], openTerm: string): string[] {
  if (!movable(c)) return []
  const open = termOrder(openTerm)
  return options.map((o) => o.term).filter((t) => termOrder(t) >= open).sort((a, b) => termOrder(a) - termOrder(b))
}

/** Put `c` in `term` (null = not planned). Returns the same object when nothing changes, so history ignores it. */
export function moveCourse(draft: Draft, c: DegreeCourse, term: string | null, allowed: string[]): Draft {
  if (!movable(c)) return draft
  if (term !== null && !allowed.includes(term)) return draft
  if (termOf(c, draft) === term) return draft
  const next: Record<number, string> = { ...draft }
  if (term === originalTerm(c)) delete next[c.row]
  else if (term === null) return draft // un-planning is a Phase 4 decision, not a draft move
  else next[c.row] = term
  return next
}

export const changeCount = (draft: Draft): number => Object.keys(draft).length

export interface TermLoad {
  term: string
  credits: number
  limit: TermLimit
  over: boolean
  courses: DegreeCourse[]
  /** Courses of the term whose units PeopleSoft did not give: the credits are "at least" this. */
  unknown: DegreeCourse[]
  /** Planned courses PeopleSoft lists in this term that belong to another term in the degree's order: not counted here when the term would be over its limit (see REQUIREMENTS-PLANNING.md). */
  deferred: DegreeCourse[]
}

/** Credits per term with the draft applied: completed, in progress and planned courses all count (PeopleSoft counts them all). Courses inside Course Lists are not known here. */
export function termLoads(courses: DegreeCourse[], draft: Draft, setAside = true): Map<string, TermLoad> {
  const m = new Map<string, TermLoad>()
  const add = (t: string, c: DegreeCourse) => {
    const e = m.get(t) ?? { term: t, credits: 0, limit: limitFor(t), over: false, courses: [], deferred: [], unknown: [] }
    e.credits += c.units ?? 0
    e.courses.push(c)
    m.set(t, e)
    return e
  }
  for (const c of courses) {
    const t = termOf(c, draft)
    if (c.dropping || !t) continue
    if (c.units === null) {
      // Unknown credits are never counted as 0: they are listed, so the term shows "at least" and a save into it is refused.
      const e = m.get(t) ?? { term: t, credits: 0, limit: limitFor(t), over: false, courses: [], deferred: [], unknown: [] }
      e.unknown.push(c)
      m.set(t, e)
      continue
    }
    add(t, c)
  }
  // The planning-semester rule: a term over its limit does not count planned (not enrolled, not taken) courses that belong to another
  // term in the degree's order and that the student did not move here in this draft. They are counted in the term they belong to instead.
  const moved: [string, DegreeCourse][] = []
  for (const e of setAside ? [...m.values()] : []) {
    if (e.credits <= e.limit.max) continue
    for (const c of [...e.courses]) {
      const away = c.plannable && (c.status === 'planned' || c.status === 'unplanned') && !(c.row in draft) && c.planningTerm && c.planningTerm !== e.term && c.units !== null
      if (!away) continue
      e.credits -= c.units ?? 0
      e.courses = e.courses.filter((x) => x !== c)
      e.deferred.push(c)
      moved.push([c.planningTerm as string, c])
    }
  }
  for (const [t, c] of moved) add(t, c)
  for (const e of m.values()) e.over = e.credits > e.limit.max
  return m
}

export const loadOf = (loads: Map<string, TermLoad>, term: string): TermLoad => loads.get(term) ?? { term, credits: 0, limit: limitFor(term), over: false, courses: [], deferred: [], unknown: [] }

/** Academic year "2026" = Fall 2026, Winter 2027, Spring 2027 (PeopleSoft's term order). */
export const academicYearOf = (term: string): number | null => {
  const m = /^(fall|winter|spring)\s+(\d{4})$/i.exec(term.trim())
  if (!m) return null
  return m[1].toLowerCase() === 'fall' ? Number(m[2]) : Number(m[2]) - 1
}
export const termsOfAcademicYear = (y: number): string[] => [`Fall ${y}`, `Winter ${y + 1}`, `Spring ${y + 1}`]
export const academicYearLabel = (y: number): string => `${y}–${String(y + 1).slice(2)}`

export interface Warning {
  kind: 'credits' | 'prerequisite' | 'corequisite'
  course?: string
  paired?: string
  term: string
  text: string
}

/**
 * Prerequisite and co-requisite warnings for the draft. `prereqs` and `coreqs` hold only requirement texts that were read (by course code).
 * A prerequisite counts as met when that course is completed or in progress, or planned in a term before the course's own term.
 * A co-requisite counts as met when that course is completed or in progress, or planned in the same (or earlier) term.
 * Codes that are not in the degree list are not judged (we cannot tell).
 */
export function warningsFor(courses: DegreeCourse[], draft: Draft, prereqs: Map<string, Prerequisite>, openTerm: string, coreqs: Map<string, string[]> = new Map()): Warning[] {
  const out: Warning[] = []
  const loads = termLoads(courses, draft)
  for (const l of loads.values()) {
    if (l.over) out.push({ kind: 'credits', term: l.term, text: `${l.term} has ${l.credits} credits; PeopleSoft allows ${l.limit.max}${l.limit.confirmed ? '' : ' (not confirmed)'}.` })
  }
  const known = new Set(courses.map((c) => normCode(c.code)))
  for (const c of courses) {
    const t = termOf(c, draft)
    if (!t) continue
    const myOrder = termOrder(t)
    const done = new Set<string>()
    for (const o of courses) {
      const ot = termOf(o, draft)
      if (!ot || o === c) continue
      const before = termOrder(ot) < myOrder
      const taken = o.status === 'completed' || o.status === 'inProgress'
      if (taken || before) done.add(normCode(o.code))
    }
    const req = prereqs.get(normCode(c.code))
    if (req) {
      const judged: Prerequisite = { ...req, courses: req.courses.filter((x) => known.has(x)) }
      if (judged.courses.length > 0 && isMet(judged, done) === false) {
        const lacking = judged.courses.filter((x) => !done.has(x))
        out.push({ kind: 'prerequisite', course: c.code, term: t, text: `${c.code} in ${t} needs ${lacking.join(req.op === 'or' ? ' or ' : ' and ')} planned or finished before it.` })
      }
    } else {
      const cat = getCatalogRule(c.code)
      if (cat?.prerequisites) {
        const refs = collectRuleCodes(cat.prerequisites)
        if (refs.some((x) => known.has(x)) && evalRuleTree(cat.prerequisites, done) === false) {
          out.push({ kind: 'prerequisite', course: c.code, term: t, text: `${c.code} in ${t} needs ${formatRuleTree(cat.prerequisites)} planned or finished before it.` })
        }
      } else if (cat?.cohort_rules?.length) {
        const anyKnown = cat.cohort_rules.some((cr) => collectRuleCodes(cr.prerequisites).some((x) => known.has(x)))
        const anyMet = cat.cohort_rules.some((cr) => evalRuleTree(cr.prerequisites, done) === true)
        if (anyKnown && !anyMet) {
          const summary = cat.cohort_rules.map((cr) => `${cr.condition}: ${formatRuleTree(cr.prerequisites)}`).join('; ')
          out.push({ kind: 'prerequisite', course: c.code, term: t, text: `${c.code} in ${t} prerequisite (${summary}) is not met in earlier terms.` })
        }
      }
    }
  }
  for (const c of courses) {
    const t = termOf(c, draft)
    const liveCos = coreqs.get(normCode(c.code)) ?? []
    const catCos = getCatalogRule(c.code)?.corequisites ?? []
    const cos = [...new Set([...liveCos, ...catCos])]
    if (!t || !cos.length) continue
    const myOrder = termOrder(t)
    for (const co of cos) {
      if (!known.has(co)) continue
      const peer = courses.find((o) => normCode(o.code) === co)
      if (!peer) continue
      const pt = termOf(peer, draft)
      const taken = peer.status === 'completed' || peer.status === 'inProgress'
      const sameOrEarlier = !!pt && termOrder(pt) <= myOrder
      if (!taken && !sameOrEarlier) {
        out.push({ kind: 'corequisite', course: c.code, paired: peer.code, term: t, text: `${c.code} in ${t} requires co-requisite ${peer.code} in the same term${pt ? ` (currently in ${pt})` : ' (not planned yet)'}.` })
      }
    }
  }
  void openTerm
  return out
}

/**
 * Where to put a course that does not fit: of the allowed terms (other than `from`) where it fits under the limit, the one with the
 * fewest credits; the nearest to `from` breaks a tie. Null when nowhere fits.
 */
export function suggestTerm(c: DegreeCourse, draft: Draft, courses: DegreeCourse[], allowed: string[], from: string): string | null {
  if (c.units === null) return null
  const loads = termLoads(courses.filter((x) => x !== c), draft)
  let best: { term: string; credits: number; dist: number } | null = null
  for (const t of allowed) {
    if (t === from) continue
    const l = loadOf(loads, t)
    if (l.credits + c.units > l.limit.max) continue
    const dist = Math.abs(termOrder(t) - termOrder(from))
    if (!best || l.credits < best.credits || (l.credits === best.credits && dist < best.dist)) best = { term: t, credits: l.credits, dist }
  }
  return best?.term ?? null
}

export const REVIEW_NOTE = 'Review this afterwards: it was chosen by credits only, not by when the course is offered.'

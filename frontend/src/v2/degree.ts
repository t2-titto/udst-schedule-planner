import { termOrder } from './data'
import { limitFor, type TermLimit } from './credits'
import { parseUnits } from './model'

/** What the server returns for `degree_tree` (the connector's own field names). */
export interface WireDegree {
  years: { index: number; name: string; summary: string; state: string }[]
  semesters: { index: number; year: number; name: string; summary: string }[]
  rows: {
    row: number
    year: number
    semester: number | null
    label: string
    joined_by_and: boolean
    code: string
    title: string
    category: string
    units: string | null
    term: string | null
    term_choice: { selected: string; options: { code: string; label: string }[] } | null
    has_select: boolean
    has_requirement: boolean
  }[]
}

export interface WireCourseList {
  name: string
  choices: { row: number; joined_by: string; code: string; title: string; category: string; units: string | null; term: string }[]
}

export interface WireRequirement {
  course: string
  text: string
}

/** completed: term before the open one; inProgress: the open term; planned: a later term; unplanned: a dropdown with no term chosen. */
export type CourseStatus = 'completed' | 'inProgress' | 'planned' | 'unplanned'

export interface DegreeCourse {
  row: number
  year: number
  code: string
  title: string
  category: string
  units: number | null
  /** The term name ("Winter 2027"), or null when not planned. */
  term: string | null
  status: CourseStatus
  /** A dropdown exists: the student can still change its term (Phase 4). */
  plannable: boolean
  hasRequirement: boolean
  /** The term this course belongs to in the degree's own order (its Semester block), or null when that cannot be worked out. */
  planningTerm?: string | null
  /** The student is dropping this enrolled course in the schedule being built: it no longer counts towards its term's credits. */
  dropping?: boolean
}

/** A requirement group row: a Course List ("choose one of these") with its Select button. */
export interface DegreeGroup {
  row: number
  year: number
  name: string
  hasSelect: boolean
}

export type Block = { kind: 'course'; course: DegreeCourse } | { kind: 'group'; group: DegreeGroup }

export interface DegreeSemester {
  index: number | null
  year: number
  name: string
  /** PeopleSoft's own words: "Total Courses Required: 4 Completed: 100%". */
  summary: string
  blocks: Block[]
  /** Credits of the listed courses (a Course List adds none until a course is chosen). Unknown units are not counted. */
  credits: number
  unknownUnits: number
}

export interface DegreeYear {
  index: number
  name: string
  summary: string
  semesters: DegreeSemester[]
}

export interface TermCredits {
  term: string
  credits: number
  limit: TermLimit
  over: boolean
  courses: string[]
}

/** A course PeopleSoft counts in a term that is not a row of the tree (typically one chosen inside a Course List). Units are never guessed: null means unknown. */
export interface ExtraCourse {
  code: string
  title: string
  term: string
  units: number | null
}

export interface DegreeView {
  years: DegreeYear[]
  /** Courses of the plan or the enrolled list that the tree does not list as a row; they count towards their term but cannot be moved here. */
  extras: DegreeCourse[]
  /** Credits per term across the whole degree, from the term each course is taken or planned in. */
  byTerm: TermCredits[]
  progress: { completed: number; inProgress: number; planned: number; unplanned: number; total: number; unknown: number }
}

/** "Fall 2025" is order 6075, "Winter 2026" 6076, "Spring 2026" 6077, "Fall 2026" 6078 (see `termOrder`). The inverse. */
export function termFromOrder(n: number): string | null {
  if (!Number.isInteger(n) || n < 3000) return null
  const r = n % 3
  return r === 0 ? `Fall ${n / 3}` : r === 1 ? `Winter ${(n - 1) / 3 + 1}` : `Spring ${(n - 2) / 3 + 1}`
}

/**
 * The degree's own order of terms: its first Semester is the first term the student took courses in, and each next Semester is the next
 * term (Fall, Winter, Spring). So a course's Semester block says which term it belongs to, whatever term PeopleSoft has it planned in.
 */
export function curriculumTerms(w: WireDegree): Map<number, string> {
  const sems = [...new Set(w.rows.map((r) => r.semester).filter((x): x is number => x !== null))].sort((a, b) => a - b)
  const out = new Map<number, string>()
  if (!sems.length) return out
  const first = w.rows.find((r) => r.semester === sems[0] && r.term && termOrder(r.term) > 0)
  if (!first?.term) return out
  const base = termOrder(first.term)
  for (const sem of sems) { const t = termFromOrder(base + (sem - sems[0])); if (t) out.set(sem, t) }
  return out
}

/** Course code -> the term it belongs to in the degree's own order, for the courses not taken yet (the ones with a term dropdown). */
export function planningMap(w: WireDegree): Record<string, string> {
  const terms = curriculumTerms(w)
  const out: Record<string, string> = {}
  for (const r of w.rows) {
    if (!r.code || !r.term_choice || r.semester === null) continue
    const t = terms.get(r.semester)
    if (t) out[r.code] = t
  }
  return out
}

const clean = (s: string) => s.replace(/\s+/g, ' ').trim()

/** The wire tree as the page works with it. `openTerm` ("Winter 2027") decides what is completed, in progress or planned. */
export function buildDegree(w: WireDegree, openTerm: string, extra: ExtraCourse[] = [], dropping: string[] = []): DegreeView {
  const open = termOrder(openTerm)
  const planning = curriculumTerms(w)
  const norm = (c: string) => c.replace(/\s+/g, '').toUpperCase()
  const drop = new Set(dropping.map(norm))
  const courseOf = (r: WireDegree['rows'][number]): DegreeCourse => {
    const choice = r.term_choice
    const label = choice ? (choice.options.find((o) => o.code === choice.selected)?.label ?? null) : r.term
    const term = label && label.trim() ? label.trim() : null
    let status: CourseStatus
    if (!term) status = 'unplanned'
    else if (choice) status = 'planned'
    else status = termOrder(term) < open ? 'completed' : termOrder(term) === open ? 'inProgress' : 'planned'
    return { row: r.row, year: r.year, code: r.code, title: r.title, category: r.category, units: parseUnits(r.units), term, status, plannable: !!choice, hasRequirement: r.has_requirement, planningTerm: r.semester === null ? null : planning.get(r.semester) ?? null, ...(drop.has(norm(r.code)) ? { dropping: true } : {}) }
  }
  const years: DegreeYear[] = w.years.map((y) => ({ index: y.index, name: clean(y.name), summary: clean(y.summary), semesters: [] }))
  const all: DegreeCourse[] = []
  for (const r of w.rows) {
    const y = years.find((x) => x.index === r.year)
    if (!y) continue
    let sem = y.semesters.find((s) => s.index === r.semester)
    if (!sem) {
      const head = r.semester === null ? undefined : w.semesters.find((s) => s.index === r.semester)
      sem = { index: r.semester, year: r.year, name: head ? clean(head.name) : 'Courses', summary: head ? clean(head.summary) : '', blocks: [], credits: 0, unknownUnits: 0 }
      y.semesters.push(sem)
    }
    if (r.code) {
      const c = courseOf(r)
      all.push(c)
      sem.blocks.push({ kind: 'course', course: c })
      if (c.units === null) sem.unknownUnits++
      else sem.credits += c.units
    } else {
      sem.blocks.push({ kind: 'group', group: { row: r.row, year: r.year, name: clean(r.label), hasSelect: r.has_select } })
    }
  }
  const inTree = new Set(all.map((c) => c.code.replace(/\s+/g, '').toUpperCase()))
  const extras: DegreeCourse[] = extra
    .filter((x) => x.term && !inTree.has(x.code.replace(/\s+/g, '').toUpperCase()))
    .map((x, i) => ({
      row: -(i + 1), year: -1, code: x.code, title: x.title, category: 'Other (e.g. from a Course List)', units: x.units, term: x.term,
      status: termOrder(x.term) < open ? 'completed' : termOrder(x.term) === open ? 'inProgress' : 'planned', plannable: false, hasRequirement: false, ...(drop.has(norm(x.code)) ? { dropping: true } : {}),
    }))
  const termMap = new Map<string, TermCredits>()
  for (const c of [...all, ...extras]) {
    if (c.dropping || !c.term || c.units === null) continue
    const t = termMap.get(c.term) ?? { term: c.term, credits: 0, limit: limitFor(c.term), over: false, courses: [] }
    t.credits += c.units
    t.courses.push(c.code)
    termMap.set(c.term, t)
  }
  const byTerm = [...termMap.values()].sort((a, b) => termOrder(a.term) - termOrder(b.term))
  for (const t of byTerm) t.over = t.credits > t.limit.max
  const sumBy = (st: CourseStatus) => all.filter((c) => c.status === st).reduce((n, c) => n + (c.units ?? 0), 0)
  return {
    years,
    extras,
    byTerm,
    progress: { completed: sumBy('completed'), inProgress: sumBy('inProgress'), planned: sumBy('planned'), unplanned: sumBy('unplanned'), total: all.reduce((n, c) => n + (c.units ?? 0), 0), unknown: all.filter((c) => c.units === null).length },
  }
}

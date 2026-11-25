import { conflicts } from '../engine/solver'
import type { Block, Chosen, Schedule } from '../engine/solver'
import type { ClassOption, Course, Meeting } from '../engine/types'
import { labOnly } from '../plan'
import { currentOption, type EnrolledRow } from '../lock'
import { codeOf } from '../lock'

/** A course on the planner: what PeopleSoft offers for it, and what the student holds in it (if anything). */
export interface PCourse {
  code: string
  title: string
  term: string
  key?: Course['key']
  /** Row in the Plan & Enrol list (courses not held only). */
  row?: number
  /** Every combination PeopleSoft offers. Empty until they are known (an enrolled course's are read in the background). */
  options: ClassOption[]
  /** What the student is enrolled in, built from PeopleSoft's enrolled list. */
  held?: ClassOption
  /** Credit units of the course; null when PeopleSoft did not give them (never counted as 0). */
  units?: number | null
  /**
   * Set on a degree course the student added to the schedule that PeopleSoft does not have planned yet: Apply plans it into the term
   * (course planning only) and then enrols it, inside the one review.
   */
  addPlan?: AddPlan
}

/** Where the course sits in PeopleSoft's degree tree and which term code to plan it into. */
export interface AddPlan {
  year: number
  index: number
  toCode: string
  to: string
}

export interface PlannerState {
  /** Courses taken out of the schedule: a held one becomes a drop, a new one just leaves. */
  removed: string[]
  /** Enrolled courses the student unlocked. */
  unlocked: string[]
  /** The combination chosen for a course (an option id). */
  choice: Record<string, string>
  /** Class numbers the student pinned: a pinned class is always part of the course's combination, whatever the score says. */
  pins: string[]
}

/** "3.00" -> 3. Anything that is not a non-negative number is unknown (null), never 0. */
export function parseUnits(text: string | undefined | null): number | null {
  if (typeof text !== 'string') return null
  const m = /^\s*(\d+(?:\.\d+)?)\s*$/.exec(text)
  return m ? Number(m[1]) : null
}

export const emptyState = (): PlannerState => ({ removed: [], unlocked: [], choice: {}, pins: [] })

export const hasClass = (o: ClassOption, nbr: string): boolean => o.components.some((x) => x.classNbr === nbr)

/**
 * The options that keep every pin, applied one after another: a pin that no remaining option can honour (it contradicts an earlier pin)
 * is left out and named in `ignored`, so the pins never leave a course without a combination.
 */
export function pinnedOptions(options: ClassOption[], pins: string[]): { options: ClassOption[]; ignored: string[] } {
  let left = options
  const ignored: string[] = []
  for (const p of pins) {
    if (!options.some((o) => hasClass(o, p))) continue // not a class of this course
    const next = left.filter((o) => hasClass(o, p))
    if (next.length) left = next
    else ignored.push(p)
  }
  return { options: left, ignored }
}

export const isLocked = (c: PCourse, s: PlannerState): boolean => !!c.held && !s.unlocked.includes(c.code)

/** The option a course shows right now, or null when it was removed or nothing is known. */
export function effective(c: PCourse, s: PlannerState): ClassOption | null {
  if (s.removed.includes(c.code)) return null
  if (isLocked(c, s)) return c.held ?? null
  const picked = s.choice[c.code] ? c.options.find((o) => o.id === s.choice[c.code]) : undefined
  if (picked) return picked
  // Unlocked and nothing picked yet: stay on what is held, else the first combination.
  return (c.held && (currentOption({ ...courseOf(c) }, c.held) ?? c.held)) || c.options[0] || null
}

export const courseOf = (c: PCourse): Course => ({ code: c.code, title: c.title, term: c.term, key: c.key, row: c.row, options: c.options })

export interface PBlock {
  courseCode: string
  classNbr: string
  kind: string
  room: string
  instructors: string[]
  day: number
  start: number
  end: number
  locked: boolean
  /** Whether the course is a held one that was unlocked (so it keeps a marker of what it is leaving). */
  held: boolean
  /** The class belongs to a lab-only change: it will be dropped and enrolled again. */
  redo?: boolean
  /** The student pinned this class. */
  pinned?: boolean
}

export function blocksOf(courses: PCourse[], s: PlannerState): PBlock[] {
  const out: PBlock[] = []
  for (const c of courses) {
    const o = effective(c, s)
    if (!o) continue
    const locked = isLocked(c, s)
    const redo = redoChange(c, o)
    for (const comp of o.components) {
      for (const m of comp.meetings) {
        out.push({ courseCode: c.code, classNbr: comp.classNbr, kind: comp.kind, room: comp.room, instructors: comp.instructors, day: m.day, start: m.start, end: m.end, locked, held: !!c.held, redo, pinned: s.pins.includes(comp.classNbr) })
      }
    }
  }
  return out
}

export interface Overlap {
  a: PBlock
  b: PBlock
  day: number
  start: number
  end: number
}

/** Every pair of classes from different courses that meet at the same time. */
export function overlapsOf(blocks: PBlock[]): Overlap[] {
  const out: Overlap[] = []
  for (let i = 0; i < blocks.length; i++) {
    for (let j = i + 1; j < blocks.length; j++) {
      const a = blocks[i]
      const b = blocks[j]
      if (a.courseCode === b.courseCode) continue
      const m1: Meeting = { day: a.day, start: a.start, end: a.end }
      const m2: Meeting = { day: b.day, start: b.start, end: b.end }
      if (conflicts(m1, m2, 0)) out.push({ a, b, day: a.day, start: Math.max(a.start, b.start), end: Math.min(a.end, b.end) })
    }
  }
  return out
}

const meetsOf = (o: ClassOption): Meeting[] => o.components.flatMap((c) => c.meetings)

/** PeopleSoft refuses a swap that changes only the lab, so for an enrolled course those combinations are not offered. */
export const swappable = (c: PCourse, o: ClassOption): boolean => !c.held || sameAsHeld(c, o) || !labOnly(c.held, o)

/** Changes only the lab of an enrolled course: PeopleSoft refuses it as a swap, so it is done as a drop and a new enrolment. */
export const redoChange = (c: PCourse, o: ClassOption): boolean => !!c.held && !sameAsHeld(c, o) && labOnly(c.held, o)

/** The combination is exactly what the student already holds (choosing it changes nothing, so it is always allowed). */
export function sameAsHeld(c: PCourse, o: ClassOption): boolean {
  if (!c.held) return false
  const have = new Set(c.held.components.map((x) => x.classNbr))
  // Same rule the plan uses to keep a course: every class of the option is already held (PeopleSoft sometimes adds extra classes to what is held).
  return o.components.length > 0 && o.components.every((x) => have.has(x.classNbr))
}

/** How far (in minutes, summed) an option's meetings sit from another's: the smaller, the closer to what the student had. */
function distance(a: ClassOption, b: ClassOption): number {
  const am = meetsOf(a)
  const bm = meetsOf(b)
  let d = 0
  for (const m of am) d += Math.min(...bm.map((x) => Math.abs(x.day - m.day) * 600 + Math.abs(x.start - m.start)), 600)
  return d
}

/**
 * The combinations of a course that overlap nothing else in the calendar, nearest to its current one first. Used for the
 * "use this instead" button next to an overlap.
 */
export function fixesFor(course: PCourse, courses: PCourse[], s: PlannerState, limit = 3): ClassOption[] {
  const current = effective(course, s)
  const others = blocksOf(courses.filter((c) => c.code !== course.code), s)
  const free = course.options.filter(swappable.bind(null, course)).filter((o) => {
    const mine: PBlock[] = o.components.flatMap((comp) => comp.meetings.map((m) => ({ courseCode: course.code, classNbr: comp.classNbr, kind: comp.kind, room: comp.room, instructors: comp.instructors, day: m.day, start: m.start, end: m.end, locked: false, held: false })))
    return overlapsOf([...mine, ...others]).length === 0 && o.id !== current?.id && !(current && sameAsHeld(course, current) && sameAsHeld(course, o))
  })
  free.sort((x, y) => (current ? distance(x, current) - distance(y, current) : 0))
  return free.slice(0, limit)
}

/** A schedule in the shape the planner's existing plan builder and calendar export take. */
export function asSchedule(courses: PCourse[], s: PlannerState): Schedule {
  const picks: Chosen[] = []
  const blocks: Block[] = []
  courses.forEach((c, i) => {
    const o = effective(c, s)
    if (!o) return
    picks.push({ course: courseOf(c), courseIndex: i, option: o, instructorScore: 0, limiting: [] })
    for (const comp of o.components)
      for (const m of comp.meetings) blocks.push({ courseIndex: i, courseCode: c.code, courseTitle: c.title, section: comp.section, classNbr: comp.classNbr, kind: comp.kind, room: comp.room, instructors: comp.instructors, day: m.day, start: m.start, end: m.end, online: /^online/i.test(comp.room.trim()) })
  })
  return { picks, blocks, freeDays: [], scores: { instructor: 0, timing: 0, freeDay: 0, total: 0 } }
}

/** Put a ranked suggestion into the planner: every course that can move takes the suggestion's combination. */
export function applySuggestion(courses: PCourse[], s: PlannerState, suggestion: Schedule): PlannerState {
  const choice = { ...s.choice }
  for (const p of suggestion.picks) {
    const c = courses.find((x) => x.code === p.course.code)
    if (c && !isLocked(c, s)) choice[c.code] = p.option.id
  }
  return { ...s, choice }
}

/** Courses for the planner: new ones from the plan list, held ones from the enrolled list (with their options when read). */
export function buildCourses(
  planCourses: Course[],
  enrolledOptions: Record<string, ClassOption>,
  enrolledRows: EnrolledRow[],
  held: Record<string, { key?: Course['key']; options: ClassOption[] }>,
  term: string,
  added: PCourse[] = [],
): PCourse[] {
  const out: PCourse[] = []
  for (const code of Object.keys(enrolledOptions)) {
    const row = enrolledRows.find((r) => codeOf(r.course) === code)
    const read = held[code]
    // The units are on the lecture row of the enrolled list (a lab row has none).
    const units = enrolledRows.filter((r) => codeOf(r.course) === code).map((r) => parseUnits(r.units)).find((u) => u !== null) ?? null
    out.push({ code, title: row ? row.course.replace(code, '').trim() : '', term, key: read?.key, options: read?.options ?? [], held: enrolledOptions[code], units })
  }
  for (const c of planCourses) {
    if (enrolledOptions[c.code]) continue
    out.push({ code: c.code, title: c.title, term: c.term, key: c.key, row: c.row, options: c.options, units: parseUnits(c.units) })
  }
  // A course added from the catalog only counts while PeopleSoft does not list it itself (after it is planned it comes from the plan list).
  for (const a of added) if (!out.some((x) => x.code === a.code)) out.push(a)
  return out
}

/**
 * What the solver may choose from: a locked course has only what is held; removed courses are not part of it. `allowRedo` also offers the
 * combinations that change only the lab of an enrolled course (done as a drop and a new enrolment, with the seat risk).
 */
export function solverCourses(courses: PCourse[], s: PlannerState, allowRedo = false): Course[] {
  const out: Course[] = []
  for (const c of courses) {
    if (s.removed.includes(c.code)) continue
    const fixed = isLocked(c, s) || (c.held && c.options.length === 0)
    // A pinned class is honoured even when it is a lab-only change (which is otherwise left out of the suggestions).
    const pins = s.pins.filter((p) => c.options.some((o) => hasClass(o, p)))
    const base = fixed ? (c.held ? [c.held] : []) : c.options.filter((o) => allowRedo || swappable(c, o) || pins.some((p) => hasClass(o, p)))
    const options = fixed ? base : pinnedOptions(base, pins).options
    if (options.length) out.push({ code: c.code, title: c.title, term: c.term, key: c.key, row: c.row, options: options.map((o) => ({ ...o, status: fixed || sameAsHeld(c, o) ? 'Open' : o.status, components: o.components.map((x) => ({ ...x, seatsOpen: fixed ? null : x.seatsOpen })) })) })
  }
  return out
}

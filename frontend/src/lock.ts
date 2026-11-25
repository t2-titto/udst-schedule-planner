import { parseDay, parseTime } from './engine/normalize'
import { conflicts } from './engine/solver'
import type { ClassOption, Component, Course, Meeting } from './engine/types'

/** One class of the student's enrolled list for a term, as the server reads it from PeopleSoft's own page. */
export interface EnrolledRow {
  row: number
  class_nbr: string
  component: string
  section: string
  course: string
  schedule: string[]
  rooms?: string[]
  instructors?: string[]
  /** PeopleSoft's units for the course; it is on the lecture row only ("3.00"), empty on a lab row. */
  units?: string
  status: string
}

/**
 * The one class number to drop a whole course with: its lecture (PeopleSoft drops the linked lab with it). Never a lab on its own:
 * PeopleSoft's review step does not accept one, and a course is dropped as a whole.
 */
export function primaryClass(rows: EnrolledRow[]): EnrolledRow | undefined {
  return rows.find((r) => !/lab/i.test(r.component)) ?? rows[0]
}

/** "Monday 3:30PM to 5:00PM" or "Monday, Wednesday 3:30PM to 5:00PM" (one meeting per day). */
export function parseSchedule(lines: string[]): Meeting[] {
  const out: Meeting[] = []
  for (const line of lines) {
    const m = /^([A-Za-z,\s]+?)\s+(\d{1,2}:\d{2}\s*[AP]M)\s+to\s+(\d{1,2}:\d{2}\s*[AP]M)/i.exec(line.trim())
    if (!m) continue
    const start = parseTime(m[2])
    const end = parseTime(m[3])
    if (start === null || end === null) continue
    for (const d of m[1].split(/[,\s]+/)) {
      const day = parseDay(d)
      if (day !== null) out.push({ day, start, end })
    }
  }
  return out
}

/** The course code at the start of PeopleSoft's "DACS 2202 Intro. to Computing Systems". */
export const codeOf = (course: string): string => /^([A-Z]{2,6}\s?\d{3,4}[A-Z]?)/.exec(course.trim())?.[1].replace(/^([A-Z]+)(\d)/, '$1 $2') ?? ''

/**
 * What the student is really enrolled in, per course code, as one option. It is built from the enrolled list itself, not
 * matched to a fetched option: PeopleSoft can hold a class (a second lab, say) that no fetched option contains.
 */
/** PeopleSoft's word for a class the student waits for ("Waitlisted" on the Plan & Enrol list, UNVERIFIED on the Drop page). */
export const isWaitlistedStatus = (s: string): boolean => /^wait(ing|\s?list(ed)?)$/i.test(s.trim())

export function enrolledOptions(rows: EnrolledRow[], includeWaitlisted = false): Record<string, ClassOption> {
  const by = new Map<string, EnrolledRow[]>()
  for (const r of rows) {
    const code = codeOf(r.course)
    if (!code || !(/^enrolled$/i.test(r.status.trim()) || (includeWaitlisted && isWaitlistedStatus(r.status)))) continue
    by.set(code, [...(by.get(code) ?? []), r])
  }
  const out: Record<string, ClassOption> = {}
  for (const [code, list] of by) {
    const components: Component[] = list.map((r) => ({
      dates: '',
      kind: r.component,
      classNbr: r.class_nbr,
      section: r.section,
      room: r.rooms?.[0] ?? '',
      instructors: r.instructors?.length ? r.instructors : ['Staff'],
      meetings: parseSchedule(r.schedule),
      seatsOpen: null,
      seatsTotal: null,
    }))
    // A course is waitlisted when any class of it is: PeopleSoft's lab and lecture of one course share one standing.
    const waitlisted = list.some((r) => isWaitlistedStatus(r.status))
    out[code] = { id: `enrolled:${code}`, optionNo: waitlisted ? 'Waitlisted' : 'Enrolled', status: 'Open', ...(waitlisted ? { waitlisted: true } : {}), components }
  }
  return out
}

/** The fetched option that matches what the student holds (all of its classes are held), if there is one. */
export function currentOption(course: Course, held: ClassOption | undefined): ClassOption | undefined {
  if (!held) return undefined
  const nbrs = new Set(held.components.map((c) => c.classNbr))
  return course.options.find((o) => o.components.length > 0 && o.components.every((c) => nbrs.has(c.classNbr)))
}

export type LockKind = 'enrolled' | 'pin'

export interface Locked {
  courses: Course[]
  locks: Record<string, LockKind>
  /** Courses an enrolled lock was lifted from because a pin needs their time. */
  autoUnlocked: string[]
  notices: string[]
}

/** A copy of an option the solver will always accept: the seats a student's own enrolment uses must not count against it. */
const accepted = (o: ClassOption): ClassOption => ({ ...o, status: 'Open', components: o.components.map((c) => ({ ...c, seatsOpen: null })) })

/**
 * Narrow the chosen courses to what is fixed.
 *  - An enrolled course is locked to the combination the student holds, until the student unlocks it.
 *  - A pinned course is locked to the pinned combination (a whole option: lecture and lab together, as PeopleSoft offers it).
 *  - A pin wins over an enrolled lock: if a pinned combination overlaps another enrolled course, that course is unlocked so it
 *    can be changed, and the notice says so.
 */
export function applyLocks(chosen: Course[], enrolled: Record<string, ClassOption>, unlocked: string[], pins: Record<string, string>): Locked {
  const locks: Record<string, LockKind> = {}
  const fixed = new Map<string, ClassOption>()
  for (const c of chosen) {
    const pin = pins[c.code] ? c.options.find((o) => o.id === pins[c.code]) : undefined
    if (pin) {
      fixed.set(c.code, pin)
      locks[c.code] = 'pin'
    } else if (enrolled[c.code] && !unlocked.includes(c.code)) {
      fixed.set(c.code, enrolled[c.code])
      locks[c.code] = 'enrolled'
    }
  }
  const autoUnlocked: string[] = []
  const notices: string[] = []
  for (const [pc, po] of [...fixed]) {
    if (locks[pc] !== 'pin') continue
    for (const [ec, eo] of [...fixed]) {
      if (ec === pc || locks[ec] !== 'enrolled') continue
      const clash = po.components.some((a) => eo.components.some((b) => a.meetings.some((x) => b.meetings.some((y) => conflicts(x, y, 0)))))
      if (clash) {
        fixed.delete(ec)
        delete locks[ec]
        autoUnlocked.push(ec)
        notices.push(`${pc} is pinned and overlaps your enrolled ${ec}. ${ec} is unlocked so you can swap it to another combination.`)
      }
    }
  }
  const courses = chosen.map((c) => {
    const o = fixed.get(c.code)
    return o ? { ...c, options: [accepted(o)] } : c
  })
  return { courses, locks, autoUnlocked, notices }
}

/**
 * A waitlisted course the Drop page does not list: built from the Plan & Enrol list's own "Selected Class" text (no class numbers, so it
 * can be shown and counted but not dropped here). `summary` looks like "Monday 10:00AM to 11:00AM Wednesday 10:00AM to 11:00AM".
 */
export function waitlistedFromSummary(code: string, summary: string): ClassOption | null {
  const lines = summary.split(/(?=(?:Sunday|Monday|Tuesday|Wednesday|Thursday|Friday|Saturday|Sun|Mon|Tue|Wed|Thu|Fri|Sat)\b)/i).map((l) => l.trim()).filter(Boolean)
  const meetings = parseSchedule(lines)
  if (!meetings.length) return null
  const comp: Component = { dates: '', kind: 'Waitlisted', classNbr: '', section: '', room: '', instructors: ['Staff'], meetings, seatsOpen: null, seatsTotal: null }
  return { id: `enrolled:${code}`, optionNo: 'Waitlisted', status: 'Open', waitlisted: true, components: [comp] }
}

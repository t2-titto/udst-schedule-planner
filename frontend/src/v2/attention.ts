import { openTerm, termOrder } from './data'
import type { DegreeView } from './degree'
import { coursesOf, termLoads, warningsFor } from './draft'
import type { Prerequisite } from './prereq'
import { blocksOf, effective, overlapsOf, type PCourse, type PlannerState } from './model'

/** The rows of PeopleSoft's class list the home screen keeps. */
export interface ClassRow {
  course_label: string
  term: string
  status: string
}

/** What the home screen remembers between visits, so it can open at once. */
export interface Snapshot {
  at: number
  term: string
  rows: ClassRow[]
  /** Courses PeopleSoft counts in a term that the degree tree does not list (from the last schedule load). */
  extras: { code: string; title: string; term: string; units: number | null }[]
  /** Problems found in the schedule the last time it was built. */
  overlaps: string[]
  full: string[]
  /** Waitlist changes seen between two reads ("X is now enrolled"), kept until the next change. */
  waitMoved?: string[]
}

export type AttentionKind = 'over' | 'noclasses' | 'prereq' | 'overlap' | 'full' | 'waitlist'

export interface AttentionItem {
  kind: AttentionKind
  text: string
}

export const SNAPSHOT_KEY = 'schedule-planner:v2:home'
export const OLD_AFTER_MS = 3600 * 1000

export function loadSnapshot(): Snapshot | null {
  try {
    const s = JSON.parse(window.localStorage.getItem(SNAPSHOT_KEY) ?? 'null') as Snapshot | null
    return s && typeof s.at === 'number' && Array.isArray(s.rows) ? { ...s, extras: s.extras ?? [], overlaps: s.overlaps ?? [], full: s.full ?? [] } : null
  } catch { return null }
}

export function saveSnapshot(s: Snapshot): void {
  try { window.localStorage.setItem(SNAPSHOT_KEY, JSON.stringify(s)) } catch { /* storage blocked */ }
}

const code = (label: string) => label.split(':')[0].trim()

const waitRe = /^wait(ing|\s?list(ed)?)$/i

/** What happened to waitlisted classes between two reads of PeopleSoft's class list. */
export function waitTransitions(before: ClassRow[], after: ClassRow[]): string[] {
  const out: string[] = []
  for (const b of before.filter((x) => waitRe.test(x.status.trim()))) {
    const a = after.find((x) => x.term === b.term && code(x.course_label) === code(b.course_label))
    if (!a) out.push(`${code(b.course_label)} is no longer on your class list (it was waitlisted, ${b.term}).`)
    else if (/^enrolled$/i.test(a.status.trim())) out.push(`${code(b.course_label)} is now enrolled: you came off the waitlist (${b.term}).`)
    else if (!waitRe.test(a.status.trim())) out.push(`${code(b.course_label)} is no longer waitlisted (${b.term}); PeopleSoft now says "${a.status}".`)
  }
  return out
}

export interface Counts {
  enrolled: number
  planned: number
  noClasses: number
  waitlisted: number
}

/** Enrolled vs planned for the open term, from PeopleSoft's class list. Planned = classes available, waiting for you to enrol. */
export function countsFor(rows: ClassRow[], term: string): Counts {
  const r = rows.filter((x) => x.term === term)
  const is = (re: RegExp) => r.filter((x) => re.test(x.status.trim())).length
  return { enrolled: is(/^enrolled$/i), planned: is(/^classes available$/i), noClasses: is(/^no classes available$/i), waitlisted: is(waitRe) }
}

/** The open term, decided by the same function the schedule uses (`data.openTerm`): one rule, one tie-break. */
export const openTermOf = (rows: ClassRow[]): string => openTerm(rows)

/** Overlaps and full classes of the schedule as it is now, as short sentences. Saved when the schedule is built, shown on the home screen. */
export function scheduleIssues(courses: PCourse[], state: PlannerState): { overlaps: string[]; full: string[] } {
  const overlaps = overlapsOf(blocksOf(courses, state)).map((o) => `${o.a.courseCode} and ${o.b.courseCode} overlap`)
  const full: string[] = []
  for (const c of courses) {
    if (state.removed.includes(c.code) || c.held) continue
    const o = effective(c, state)
    if (o && (o.status.toLowerCase() === 'closed' || o.components.some((x) => x.seatsOpen === 0))) full.push(`${c.code}: the chosen class is full or closed`)
  }
  return { overlaps: [...new Set(overlaps)], full }
}

/**
 * Everything that needs the student's attention, from what is known now. Nothing here sends anything, and a list can be wrong when its inputs
 * are old: the home screen shows their age, and every write reads PeopleSoft again first.
 */
export function attentionList(input: { term: string; rows: ClassRow[]; view: DegreeView | null; prereqs: Map<string, Prerequisite>; overlaps: string[]; full: string[]; waitlistOn?: boolean; waitMoved?: string[] }): AttentionItem[] {
  const out: AttentionItem[] = []
  const { term, rows, view } = input
  if (view) {
    const courses = coursesOf(view)
    for (const l of [...termLoads(courses, {}).values()].sort((a, b) => termOrder(a.term) - termOrder(b.term))) {
      if (termOrder(l.term) >= termOrder(term) && l.over) out.push({ kind: 'over', text: `${l.term} has ${l.credits} credits; the limit is ${l.limit.max}${l.limit.confirmed ? '' : ' (not confirmed)'}.` })
    }
    for (const w of warningsFor(courses, {}, input.prereqs, term)) if (w.kind === 'prerequisite') out.push({ kind: 'prereq', text: w.text })
  }
  for (const r of rows.filter((x) => x.term === term && /^no classes available$/i.test(x.status.trim()))) out.push({ kind: 'noclasses', text: `${code(r.course_label)} is planned for ${term} but has no classes.` })
  for (const r of rows.filter((x) => waitRe.test(x.status.trim()))) {
    out.push({
      kind: 'waitlist',
      text: input.waitlistOn
        ? `${code(r.course_label)} is waitlisted (${r.term}). You are not enrolled; PeopleSoft may enrol you later without asking. (Waitlist support is unverified.)`
        : `${code(r.course_label)} is waitlisted (${r.term}). Waitlists are off here: turn them on in Preferences, or check PeopleSoft.`,
    })
  }
  if (input.waitlistOn) for (const t of input.waitMoved ?? []) out.push({ kind: 'waitlist', text: t })
  for (const t of input.overlaps) out.push({ kind: 'overlap', text: t })
  for (const t of input.full) out.push({ kind: 'full', text: t })
  return out
}

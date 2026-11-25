import type { PCourse } from './model'
import { isWaitlistedStatus } from '../lock'

/** The rows of PeopleSoft's class list that matter here. */
export interface ListRow {
  course_label: string
  term: string
  status: string
}

export interface PlanDiff {
  /** Courses PeopleSoft now shows as enrolled that the page does not know as enrolled, and the reverse. */
  enrolledNow: string[]
  enrolledGone: string[]
  /** Planned (classes available) that the page does not have, and the reverse. */
  plannedNew: string[]
  plannedGone: string[]
}

const codeOfLabel = (label: string) => label.split(':')[0].trim()

/**
 * Compare PeopleSoft's class list, read just now, with what the page loaded earlier. A difference means the page's credit total may be wrong
 * (someone planned or dropped something in PeopleSoft meanwhile), so Apply must not trust it.
 */
export function planDiff(fresh: ListRow[], term: string, courses: PCourse[], includeWaitlisted = false): PlanDiff {
  const rows = fresh.filter((r) => r.term === term)
  // A waitlisted course is held, with the same standing as enrolled, when the waitlist switch is on. With it off the page does not hold it either.
  const heldNow = (r: ListRow) => /^enrolled$/i.test(r.status.trim()) || (includeWaitlisted && isWaitlistedStatus(r.status))
  const freshEnrolled = new Set(rows.filter(heldNow).map((r) => codeOfLabel(r.course_label)))
  const freshPlanned = new Set(rows.filter((r) => /^classes available$/i.test(r.status.trim())).map((r) => codeOfLabel(r.course_label)))
  const localEnrolled = new Set(courses.filter((c) => c.held).map((c) => c.code))
  // A course added from the catalog is not on PeopleSoft's list yet on purpose: Apply plans it first.
  const localPlanned = new Set(courses.filter((c) => !c.held && !c.addPlan).map((c) => c.code))
  const minus = (a: Set<string>, b: Set<string>) => [...a].filter((x) => !b.has(x)).sort()
  return {
    enrolledNow: minus(freshEnrolled, localEnrolled),
    enrolledGone: minus(localEnrolled, freshEnrolled),
    plannedNew: minus(freshPlanned, localPlanned),
    plannedGone: minus(localPlanned, freshPlanned),
  }
}

export const planChanged = (d: PlanDiff): boolean => d.enrolledNow.length + d.enrolledGone.length + d.plannedNew.length + d.plannedGone.length > 0

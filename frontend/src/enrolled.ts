import { api } from './api'
import { codeOf, enrolledOptions, type EnrolledRow } from './lock'
import type { ClassOption } from './engine/types'

/** What PeopleSoft's own enrolled list says for one term, read in this session. */
export interface EnrolledState {
  term: string
  /** The row of the term on PeopleSoft's drop page; -1 when the term is not there (nothing enrolled in it). */
  termRow: number
  rows: EnrolledRow[]
  options: Record<string, ClassOption>
  /**
   * Classes the Drop Classes page lists but the Plan and Enrol page's "Selected Class" summary of the same course does not mention.
   * They are not drawn or counted as held until the student checks them (the two PeopleSoft pages disagree about them).
   */
  unconfirmed: EnrolledRow[]
}

const squash = (t: string) => t.replace(/\s+/g, ' ').trim().toLowerCase()

/**
 * Split the drop page's classes into those the plan page's schedule summary also shows and those it does not.
 * `summaries` is the plan page's "Selected Class" text per course code, read from the same refresh. A course the plan page does not show at all
 * is not held (the plan page is the truth). `undefined` means the plan page was not read, so nothing can be checked and every class is trusted.
 */
export function reconcile(rows: EnrolledRow[], summaries: Record<string, string> | undefined): { confirmed: EnrolledRow[]; unconfirmed: EnrolledRow[] } {
  const confirmed: EnrolledRow[] = []
  const unconfirmed: EnrolledRow[] = []
  for (const r of rows) {
    if (summaries === undefined) { confirmed.push(r); continue }
    const text = summaries[codeOf(r.course)]
    if (text === undefined) { unconfirmed.push(r); continue }
    if (text === '' || r.schedule.length === 0) { confirmed.push(r); continue }
    const hay = squash(text)
    ;(r.schedule.every((line) => hay.includes(squash(line))) ? confirmed : unconfirmed).push(r)
  }
  return { confirmed, unconfirmed }
}

/** Read the enrolled list for the term (a read: nothing is changed). */
export async function readEnrolled(term: string, summaries: Record<string, string> | undefined = undefined, includeWaitlisted = false): Promise<EnrolledState> {
  const terms = await api.writeRead<{ row: number; term: string }[]>('drop_terms')
  const t = terms.find((x) => x.term.trim().toLowerCase() === term.trim().toLowerCase())
  if (!t) return { term, termRow: -1, rows: [], options: {}, unconfirmed: [] }
  const l = await api.writeRead<{ classes: EnrolledRow[] }>('enrolled_classes', { row: t.row })
  return buildEnrolled(term, t.row, l.classes, summaries, includeWaitlisted)
}

/**
 * The enrolled state from the drop page's classes. The Plan and Enrol page is the source of truth: a class only the drop page lists is
 * kept out of `rows` too, so no plan, check or drop uses it.
 */
export function buildEnrolled(term: string, termRow: number, classes: EnrolledRow[], summaries: Record<string, string> | undefined = undefined, includeWaitlisted = false): EnrolledState {
  if (termRow < 0) return { term, termRow: -1, rows: [], options: {}, unconfirmed: [] }
  const { confirmed, unconfirmed } = reconcile(classes, summaries)
  return { term, termRow, rows: confirmed, options: enrolledOptions(confirmed, includeWaitlisted), unconfirmed }
}

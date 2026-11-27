import type { ClassOption, Course } from '../engine/types'
import type { EnrolledState } from '../enrolled'
import { codeOf, primaryClass } from '../lock'
import type { Plan } from '../plan'
import type { Credits } from './credits'
import type { RoundPlan, RoundSwap } from '../swapRounds'
import type { AddPlan } from './model'
import { overlapsBetween, blockerText } from '../swapRounds'

/** "Not known here": the server finds the row or option by the course code and class numbers (see `prepare_enrol_stop`). */
export const NO_ROW = 4294967295

/** The server reads these numbers as unsigned 32-bit integers: a missing, negative or fractional one is refused with a bare 422. */
export const wireRow = (n: number | null | undefined): number => (typeof n === 'number' && Number.isInteger(n) && n >= 0 && n < NO_ROW ? n : NO_ROW)

export interface WirePick {
  row: number
  optionRow: number
  course: string
  key?: Course['key']
  classNbrs: string[]
  seen: Record<string, number | null>
  /** Join the course's waitlist instead of enrolling (unverified). */
  waitlist?: boolean
}

export interface WireSwap {
  course: string
  oldNbrs: string[]
  newNbrs: string[]
  key?: Course['key']
  seen: Record<string, number | null>
  waitsFor: string[]
  hint: string
}

export interface WirePlan {
  course: string
  year: number
  index: number
  termCode: string
  credits: { term: string; limit: number; after: number; unknown: string[]; overrideLimit?: boolean }
}

export interface ApplyRequest {
  term: string
  /** The term's row on PeopleSoft's drop page, or null when the term is not there (nothing enrolled in it): then there is nothing to drop, and the server refuses an Apply that drops. */
  termRow: number | null
  drops: { course: string; classNbr: string }[]
  rounds: WireSwap[][]
  enrols: { pick: WirePick; depends: string[]; afterDrop?: boolean; afterPlan?: boolean }[]
  /** Courses planned into a term (either for current-term enrolment with `afterPlan`, or standalone Future Semester move-outs). */
  plans: WirePlan[]
  /** The credit check the page made. The server refuses an apply that enrols without it, or with numbers over the limit without explicit override. */
  credits: { term: string; limit: number; after: number; unknown: string[]; overrideLimit?: boolean }
}

const nbrs = (o: ClassOption) => o.components.map((c) => c.classNbr)
const seen = (o: ClassOption): Record<string, number | null> => Object.fromEntries(o.components.map((c) => [c.classNbr, c.seatsOpen]))
const overlapsWith = (c: RoundSwap, other: RoundSwap) => overlapsBetween(c.next, other.now).length > 0

/**
 * The one request the Apply screen sends. Every number the server needs as an unsigned integer goes through `wireRow`, and the type
 * has no optional number, so a missing one is a compile error here instead of a 422 on the page.
 */
export function buildApplyRequest(
  plan: Plan,
  rp: RoundPlan,
  enrolled: EnrolledState,
  drops: Plan['drops'],
  credits: Credits,
  added: Record<string, AddPlan> = {},
  movePlans: WirePlan[] = [],
): ApplyRequest {
  const swapItem = (sw: RoundSwap): WireSwap => {
    const waits = rp.waits[sw.code] ?? []
    const course = plan.swaps.find((x) => x.course.code === sw.code)!.course
    return {
      course: sw.code,
      oldNbrs: nbrs(sw.now),
      newNbrs: nbrs(sw.next),
      key: course.key,
      seen: seen(sw.next),
      waitsFor: [...new Set(waits.map((w) => w.by).filter((c) => !rp.converted.some((x) => x.code === c)))],
      hint: waits.length ? `It was held back because it ${blockerText(waits[0])}.` : '',
    }
  }
  // A course dropped and enrolled again (a loop, or a change of only the lab): dropped first, enrolled after the swaps.
  const redoDrops = rp.converted.map((c) => ({ course: c.code, classNbr: primaryClass(enrolled.rows.filter((r) => codeOf(r.course) === c.code))?.class_nbr ?? '' }))
  const redoEnrols = rp.converted.map((c) => {
    const course = plan.swaps.find((x) => x.course.code === c.code)!.course
    return {
      pick: { row: NO_ROW, optionRow: NO_ROW, course: c.code, key: course.key, classNbrs: nbrs(c.next), seen: seen(c.next) },
      depends: rp.rounds.flat().filter((x) => overlapsWith(c, x)).map((x) => x.code),
      afterDrop: true,
    }
  })
  return {
    term: enrolled.term,
    termRow: wireRow(enrolled.termRow) === NO_ROW ? null : wireRow(enrolled.termRow),
    drops: [...drops.map((d) => ({ course: d.code, classNbr: d.classNbr })), ...redoDrops],
    rounds: rp.rounds.map((round) => round.map(swapItem)),
    enrols: [
      ...plan.enrols.map((e) => {
        const add = added[e.course.code]
        // A course PeopleSoft does not have planned yet has no row on its Plan & Enrol list: the server finds the row and the option after planning it.
        return {
          pick: { row: add ? NO_ROW : wireRow(e.course.row), optionRow: add ? NO_ROW : wireRow(e.next.optionRow), course: e.course.code, key: e.course.key, classNbrs: nbrs(e.next), seen: seen(e.next), ...(e.next.waitlist ? { waitlist: true } : {}) },
          depends: e.depends,
          ...(add ? { afterPlan: true } : {}),
        }
      }),
      ...redoEnrols,
    ],
    plans: [
      ...plan.enrols.filter((e) => added[e.course.code]).map((e) => {
        const a = added[e.course.code]
        return {
          course: e.course.code,
          year: a.year,
          index: a.index,
          termCode: a.toCode,
          credits: {
            term: enrolled.term,
            limit: credits.limit.max,
            after: credits.sent,
            unknown: credits.unknown,
            ...(credits.overrideLimit ? { overrideLimit: true } : {}),
          },
        }
      }),
      ...movePlans.filter((m) => !plan.enrols.some((e) => added[e.course.code] && e.course.code === m.course)),
    ],
    credits: {
      term: enrolled.term,
      limit: credits.limit.max,
      after: credits.sent,
      unknown: credits.unknown,
      ...(credits.overrideLimit ? { overrideLimit: true } : {}),
    },
  }
}

/** Names of the numeric fields the server reads as unsigned integers. */
const UNSIGNED = new Set(['row', 'optionRow', 'termRow'])

/** Problems that would make the server refuse the request (undefined values vanish in JSON; negative or fractional numbers fail as u32). */
export function wireProblems(value: unknown, path = ''): string[] {
  const out: string[] = []
  if (value === undefined) out.push(`${path}: undefined`)
  else if (Array.isArray(value)) value.forEach((v, i) => out.push(...wireProblems(v, `${path}[${i}]`)))
  else if (value && typeof value === 'object') {
    for (const [k, v] of Object.entries(value)) {
      if (k === 'key') continue // optional on the wire
      if (k === 'termRow' && v === null) continue // "not known": the server refuses an Apply that drops without it
      if (UNSIGNED.has(k) && !(typeof v === 'number' && Number.isInteger(v) && v >= 0 && v <= NO_ROW)) out.push(`${path}.${k}: ${String(v)} is not an unsigned integer`)
      else out.push(...wireProblems(v, `${path}.${k}`))
    }
  }
  return out
}

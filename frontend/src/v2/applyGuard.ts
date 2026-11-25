import type { ClassOption } from '../engine/types'
import { firstClash, type Plan } from '../plan'

/**
 * The review's drop checkbox: a student can untick "drop it" and keep a course that a new enrolment or a swap was planned to replace.
 * The plan's clash check assumes every drop happens, so it cannot see this. Any clash between a course that stays (its drop unticked)
 * and a class the run would add is returned here, and Apply stays off until it is resolved.
 */
export function keptDropClashes(plan: Plan, held: Record<string, ClassOption>, dropOn: Record<string, boolean>): string[] {
  const out: string[] = []
  for (const d of plan.drops) {
    if (dropOn[d.code] !== false) continue
    const h = held[d.code]
    if (!h) continue
    const adds = [
      ...plan.enrols.map((e) => ({ code: e.course.code, option: e.next })),
      ...plan.swaps.filter((s) => s.course.code !== d.code).map((s) => ({ code: s.course.code, option: s.next })),
    ]
    for (const a of adds) {
      const clash = firstClash([{ code: d.code, option: h }, a])
      if (clash) out.push(`You kept ${d.code} (its drop is unticked), but ${a.code} would be added at the same time: ${clash}. Tick "drop it" for ${d.code}, or go back and remove ${a.code}.`)
    }
  }
  return out
}

/** PeopleSoft's plan is re-read when the review opens; a check older than this is not trusted for a write and is run again. */
export const VERIFY_MAX_AGE_MS = 120 * 1000
export const verifyStale = (verifiedAt: number | null, now: number): boolean => verifiedAt === null || now - verifiedAt > VERIFY_MAX_AGE_MS

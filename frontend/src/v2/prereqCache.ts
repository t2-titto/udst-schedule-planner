import type { Prerequisite } from './prereq'
import { corequisiteOf, prerequisiteOf } from './prereq'

/** The free text of each course's requirement page, kept for a day so the attention list does not read PeopleSoft again and again. */
export const PREREQ_KEY = 'schedule-planner:v2:prereqs'
export const PREREQ_TTL_MS = 24 * 3600 * 1000

export type PrereqCache = Record<string, { text: string; at: number }>

export function loadPrereqs(): PrereqCache {
  try {
    const c = JSON.parse(window.localStorage.getItem(PREREQ_KEY) ?? 'null') as PrereqCache | null
    return c && typeof c === 'object' ? c : {}
  } catch { return {} }
}

export function savePrereqs(c: PrereqCache): void {
  try { window.localStorage.setItem(PREREQ_KEY, JSON.stringify(c)) } catch { /* storage blocked */ }
}

export interface ToRead {
  code: string
  year: number
  /** Position among that Year's rows (row numbers change with what is expanded). */
  index: number
}

/** The next course whose text is missing or older than a day. */
export const nextToRead = (list: ToRead[], cache: PrereqCache, now: number, skip: Set<string>): ToRead | null =>
  list.find((c) => !skip.has(c.code) && !(cache[c.code] && now - cache[c.code].at <= PREREQ_TTL_MS)) ?? null

/** The parsed prerequisites of every course whose text is in the cache and still good. */
export function prereqMap(cache: PrereqCache, now: number): Map<string, Prerequisite> {
  const m = new Map<string, Prerequisite>()
  for (const [code, e] of Object.entries(cache)) {
    if (now - e.at > PREREQ_TTL_MS) continue
    const p = prerequisiteOf(code, e.text)
    if (p) m.set(code, p)
  }
  return m
}

/** The parsed co-requisites of every course whose text is in the cache and still good. */
export function coreqMap(cache: PrereqCache, now: number): Map<string, string[]> {
  const m = new Map<string, string[]>()
  for (const [code, e] of Object.entries(cache)) {
    if (now - e.at > PREREQ_TTL_MS) continue
    const c = corequisiteOf(code, e.text)
    if (c.length) m.set(code, c)
  }
  return m
}

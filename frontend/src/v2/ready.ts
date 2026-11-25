import type { DegreeCourse, DegreeView, WireDegree } from './degree'
import { termOrder } from './data'
import { codeOf } from '../lock'

/** A degree course the student could add to this term's schedule. Only courses of the student's own degree are ever candidates. */
export interface Candidate {
  code: string
  title: string
  units: number | null
  /** The term PeopleSoft has it planned in ("Spring 2027"), or null when not planned. */
  plannedTerm: string | null
  course: DegreeCourse
}

const norm = (s: string) => s.replace(/\s+/g, '').toUpperCase()

/**
 * Degree courses that are not taken, not in progress, not already planned for the open term, and not already in the schedule.
 * Nearest planned term first (then not planned), so the likeliest ones are checked first.
 */
export function candidates(view: DegreeView, openTerm: string, inSchedule: Iterable<string>): Candidate[] {
  const have = new Set([...inSchedule].map((c) => norm(codeOf(c) || c)))
  const out: Candidate[] = []
  for (const y of view.years) for (const s of y.semesters) for (const b of s.blocks) {
    if (b.kind !== 'course') continue
    const c = b.course
    if (!c.plannable || c.status === 'completed' || c.status === 'inProgress') continue
    if (c.term === openTerm || have.has(norm(c.code))) continue
    out.push({ code: c.code, title: c.title, units: c.units, plannedTerm: c.term, course: c })
  }
  const rank = (c: Candidate) => (c.plannedTerm ? termOrder(c.plannedTerm) : Number.MAX_SAFE_INTEGER)
  return out.sort((a, b) => rank(a) - rank(b) || a.code.localeCompare(b.code))
}

/** The search box: degree courses only, by any part of the code or title. */
export function searchCandidates(list: Candidate[], text: string): Candidate[] {
  const q = text.trim().toLowerCase().replace(/\s+/g, ' ')
  if (!q) return list
  const qc = q.replace(/\s+/g, '')
  return list.filter((c) => c.code.toLowerCase().replace(/\s+/g, '').includes(qc) || c.title.toLowerCase().includes(q))
}

/** PeopleSoft's term code: "1" + two-digit year + 1 (Winter), 5 (Spring) or 8 (Fall). Null for anything else. */
export function termCode(term: string): string | null {
  const m = /^(Winter|Spring|Fall) (\d{4})$/.exec(term.trim())
  if (!m) return null
  return `1${m[2].slice(2)}${m[1] === 'Winter' ? 1 : m[1] === 'Spring' ? 5 : 8}`
}

export type Availability = { state: 'unknown' } | { state: 'checking' } | { state: 'open'; options: number; at: number } | { state: 'closed'; at: number } | { state: 'failed'; why: string }

export interface CatalogKey {
  crse_id: string
  offer_nbr: string
}

export interface CatalogCache {
  keys: Record<string, CatalogKey>
  /** term -> code -> when it was checked and how many class combinations it had (0 = none this term) */
  open: Record<string, Record<string, { at: number; options: number }>>
}

export const CATALOG_KEY = 'schedule-planner:v2:catalog'
/** How long "has classes this term" stays good before it is checked again. */
export const OPEN_TTL_MS = 6 * 3600 * 1000

export function loadCatalog(): CatalogCache {
  try {
    const c = JSON.parse(window.localStorage.getItem(CATALOG_KEY) ?? 'null') as CatalogCache | null
    if (c && c.keys && c.open) return c
  } catch { /* storage blocked or damaged */ }
  return { keys: {}, open: {} }
}

export function saveCatalog(c: CatalogCache): void {
  try { window.localStorage.setItem(CATALOG_KEY, JSON.stringify(c)) } catch { /* storage blocked */ }
}

export function availabilityOf(cache: CatalogCache, term: string, code: string, now: number): Availability {
  const e = cache.open[term]?.[norm(code)]
  if (!e || now - e.at > OPEN_TTL_MS) return { state: 'unknown' }
  return e.options > 0 ? { state: 'open', options: e.options, at: e.at } : { state: 'closed', at: e.at }
}

export function remember(cache: CatalogCache, term: string, code: string, options: number, now: number): CatalogCache {
  return { ...cache, open: { ...cache.open, [term]: { ...(cache.open[term] ?? {}), [norm(code)]: { at: now, options } } } }
}

/** The next candidate whose answer is missing or old, in list order. */
export function nextToCheck(list: Candidate[], cache: CatalogCache, term: string, now: number, skip: Set<string>): Candidate | null {
  return list.find((c) => !skip.has(norm(c.code)) && availabilityOf(cache, term, c.code, now).state === 'unknown') ?? null
}

export const normCodeKey = norm

export type { WireDegree }

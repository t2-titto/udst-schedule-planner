import type { ClassOption, Course } from '../engine/types'
import type { EnrolledRow } from '../lock'

/**
 * What a refresh may reuse. Class times and the combinations of a course hardly change within a term, so they are remembered in this
 * browser and re-read only when they could have changed: the course's enrolled classes are different, or the copy is old.
 * Seats are not trusted from here: the apply steps read them fresh right before anything is sent.
 */
export interface HeldEntry {
  /** The enrolled class numbers of the course when this was read. */
  fp: string
  key?: Course['key']
  options: ClassOption[]
  at: number
}

export interface Cache {
  term: string
  /** Course keys of the plan's courses, so the next read can open them directly. */
  keys: Record<string, unknown>
  held: Record<string, HeldEntry>
}

const KEY = 'schedule-planner:v2:cache'
const ACCOUNT_SESSION_KEY = 'schedule-planner:account'
export const HELD_TTL_MS = 12 * 60 * 60 * 1000

function cacheKey(): string {
  try {
    const acct = window.sessionStorage.getItem(ACCOUNT_SESSION_KEY)
    return acct ? `${KEY}:${acct}` : KEY
  } catch {
    return KEY
  }
}

export function loadCache(): Cache | null {
  try {
    try { window.localStorage.removeItem(KEY) } catch { /* ignore */ }
    const raw = window.sessionStorage.getItem(cacheKey())
    if (!raw) return null
    const c = JSON.parse(raw) as Partial<Cache>
    if (typeof c.term !== 'string' || typeof c.keys !== 'object' || c.keys === null || typeof c.held !== 'object' || c.held === null) return null
    return { term: c.term, keys: c.keys as Cache['keys'], held: c.held as Cache['held'] }
  } catch {
    return null
  }
}

function save(c: Cache): void {
  try {
    try { window.localStorage.removeItem(KEY) } catch { /* ignore */ }
    window.sessionStorage.setItem(cacheKey(), JSON.stringify(c))
  } catch {
    /* storage blocked or full: the planner just reads everything next time */
  }
}

export function clearCache(): void {
  try {
    window.sessionStorage.removeItem(cacheKey())
    window.localStorage.removeItem(KEY)
  } catch {
    /* ignore */
  }
}

/** The enrolled class numbers of one course, sorted: when they change, the course's other combinations may have too. */
export function fingerprint(rows: EnrolledRow[], code: string, codeOf: (course: string) => string): string {
  return rows.filter((r) => codeOf(r.course) === code).map((r) => r.class_nbr).sort().join(',')
}

/** A remembered read of a course's combinations, if it is for the same term, the same enrolled classes, and not too old. */
export function usableHeld(cache: Cache | null, term: string, code: string, fp: string, now: number): HeldEntry | undefined {
  if (!cache || cache.term !== term) return undefined
  const e = cache.held[code]
  return e && e.fp === fp && fp !== '' && now - e.at < HELD_TTL_MS ? e : undefined
}

function forTerm(term: string): Cache {
  const c = loadCache()
  return c && c.term === term ? c : { term, keys: {}, held: {} }
}

export function rememberHeld(term: string, code: string, entry: HeldEntry): void {
  const c = forTerm(term)
  c.held[code] = entry
  save(c)
}

export function rememberKeys(term: string, keys: Record<string, unknown>): void {
  const c = forTerm(term)
  c.keys = { ...c.keys, ...keys }
  save(c)
}

export function ago(at: number, now = Date.now()): string {
  const s = Math.max(0, Math.floor((now - at) / 1000))
  if (s < 45) return 'just now'
  const m = Math.floor(s / 60)
  if (m < 60) return `${m} min ago`
  const h = Math.floor(m / 60)
  if (h < 24) return `${h} h ago`
  return `${Math.floor(h / 24)} d ago`
}

import { api } from '../api'
import { normalizeCourses } from '../engine/normalize'
import { isJoinable } from '../engine/solver'
import type { ClassOption, Course } from '../engine/types'
import { buildEnrolled, readEnrolled, type EnrolledState } from '../enrolled'
import type { EnrolledRow } from '../lock'
import { fingerprint, loadCache, rememberHeld, rememberKeys, usableHeld } from './cache'
import { codeOf, isWaitlistedStatus, waitlistedFromSummary } from '../lock'
import { DEFAULT_SETTINGS, type Settings } from '../engine/types'

export interface OtherRow {
  code: string
  title: string
  note: string
}

export interface Loaded {
  term: string
  enrolled: EnrolledState
  /** Courses of the plan with classes this term (not held), with their combinations. */
  plan: Course[]
  /** Everything else the class list showed: courses without classes this term and other terms. */
  others: OtherRow[]
  /** Per-course raw status from PeopleSoft's Plan & Enrol list (e.g. "Classes available", "Enrolled"). */
  classStatuses?: Record<string, { term: string; status: string }>
}

export type LoadStep = 'enrolled' | 'plan' | 'times'

/**
 * A class option PeopleSoft marks "Wait List" is not one the student can enrol in. It stays out of every list unless waitlist support is on
 * and the option still has a place; a course the student already holds never offers one (a swap into a waitlist is not supported).
 */
export function keepJoinable(options: ClassOption[], waitlistOn: boolean): ClassOption[] {
  return options.filter((o) => !o.waitlist || (waitlistOn && isJoinable(o)))
}

/**
 * Keep every normalized option (Open, Waitlist, and Closed) sorted Open-first so the UI's three toggles
 * (`Open` / `Waitlist` / `Closed`) filter instantaneously without refetching PeopleSoft.
 */
export function sortBrowseable(options: ClassOption[]): ClassOption[] {
  const rank = (o: ClassOption) => {
    const s = o.status.trim().toLowerCase()
    if (!o.waitlist && s === 'open' && o.components.every((c) => c.seatsOpen == null || c.seatsOpen > 0)) return 0
    if (o.waitlist || s.includes('wait')) return 1
    return 2
  }
  return [...options].sort((a, b) => rank(a) - rank(b))
}

const isAvailable = (status: string) => /^classes available$/i.test(status.trim())

/** "Fall 2026" -> a number that grows with the academic year (Fall, then Winter, then Spring), or -1 when unreadable. */
export function termOrder(term: string): number {
  const m = /^(fall|winter|spring)\s+(\d{4})$/i.exec(term.trim())
  if (!m) return -1
  const year = Number(m[2])
  const season = m[1].toLowerCase()
  return season === 'fall' ? year * 3 : (year - 1) * 3 + (season === 'winter' ? 1 : 2)
}

/**
 * The open term is the one whose courses PeopleSoft says have classes available. When none do (everything is already enrolled),
 * it is the latest term the student holds classes in.
 */
export function openTerm(rows: { term: string; status: string }[]): string {
  const count = new Map<string, number>()
  for (const r of rows) if (isAvailable(r.status) && r.term) count.set(r.term, (count.get(r.term) ?? 0) + 1)
  let best = ''
  let n = 0
  // A tie goes to the earlier term, so the open term (and with it the credit limit) never depends on the order the rows came in.
  for (const [t, c] of count) if (c > n || (c === n && termOrder(t) < termOrder(best))) [best, n] = [t, c]
  if (best) return best
  const held = [...new Set(rows.filter((r) => /^enrolled$/i.test(r.status.trim()) && r.term).map((r) => r.term))]
  held.sort((a, b) => termOrder(b) - termOrder(a))
  return held[0] ?? ''
}

/**
 * The plan page's "Selected Class" text per held course of the term (enrolled or waitlisted). A held course missing here is not held:
 * the Drop page can list a class the plan page does not show.
 */
export function summariesOf(rows: { term: string; status: string; course_label: string; schedule_summary: string }[], term: string): Record<string, string> {
  const out: Record<string, string> = {}
  for (const r of rows) {
    const held = /^enrolled$/i.test(r.status.trim()) || isWaitlistedStatus(r.status)
    if (r.term === term && held) out[codeOf(r.course_label)] = r.schedule_summary ?? ''
  }
  return out
}

/** Waitlisted courses the Plan & Enrol list shows but the Drop page does not: added to the held courses from the list's own text. */
export function addListedWaitlist(enrolled: EnrolledState, rows: { term: string; status: string; course_label: string; schedule_summary: string }[], term: string): void {
  for (const r of rows) {
    const code = codeOf(r.course_label)
    if (r.term !== term || !code || !isWaitlistedStatus(r.status) || enrolled.options[code]) continue
    const o = waitlistedFromSummary(code, r.schedule_summary ?? '')
    if (o) enrolled.options = { ...enrolled.options, [code]: o }
  }
}

/** The server reads at most this many courses in one request. */
export const SECTIONS_BATCH = 12
/** ...and at most this many courses' combinations in one swap read. */
export const HELD_BATCH = 10

export interface LoadOpts {
  /** The term from last time: with it the class list and the enrolled list are read on two sessions at the same time. */
  termHint?: string
  /** Ignore everything remembered (course keys) and read it all again. */
  full?: boolean
}

/** Everything the planner needs from PeopleSoft for the open term, reporting each step as it finishes. */
export async function loadTerm(onStep: (s: LoadStep, detail?: string) => void, opts: LoadOpts = {}): Promise<Loaded> {
  let list: Awaited<ReturnType<typeof api.classes>>
  let early: { termRow: number; classes: EnrolledRow[] } | null = null
  if (opts.termHint) {
    try {
      const r = await api.refresh(opts.termHint)
      list = r.classes
      early = r.enrolled ? { termRow: r.enrolled.termRow, classes: r.enrolled.classes as EnrolledRow[] } : null
    } catch {
      list = await api.classes()
    }
  } else {
    list = await api.classes()
  }
  const term = openTerm(list.rows)
  const rows = list.rows.filter((r) => r.term === term && isAvailable(r.status))
  onStep('plan', `${rows.length} courses`)
  const summaries = summariesOf(list.rows, term)
  // The enrolled list came with the class list when the term was already known and is still the open one.
  const enrolled = early && term === opts.termHint ? buildEnrolled(term, early.termRow, early.classes, summaries, !!loadSettings().rules.joinWaitlist) : await readEnrolled(term || list.term_label, summaries, !!loadSettings().rules.joinWaitlist)
  if (enrolled && loadSettings().rules.joinWaitlist) addListedWaitlist(enrolled, list.rows, term)
  onStep('enrolled', `${Object.keys(enrolled?.options ?? {}).length} courses`)
  let plan: Course[] = []
  if (rows.length) {
    // Remembered keys let the server open each course directly (a few seconds) instead of through the class list (about 15 s each).
    const cache = opts.full ? null : loadCache()
    const keys = cache && cache.term === term ? cache.keys : {}
    // The server takes at most 12 courses per request and 60 remembered keys: ask in batches and send only the keys of that batch.
    const courses: unknown[] = []
    for (let i = 0; i < rows.length; i += SECTIONS_BATCH) {
      const batch = rows.slice(i, i + SECTIONS_BATCH)
      const known = Object.fromEntries(batch.flatMap((r) => { const code = codeOf(r.course_label); return code in keys ? [[code, keys[code]]] : [] }))
      courses.push(...(await api.sections(batch.map((r) => r.row), known)).courses)
    }
    const waitOn = !!loadSettings().rules.joinWaitlist
    plan = normalizeCourses({ courses }).courses.map((c) => ({ ...c, options: keepJoinable(c.options, waitOn) }))
    rememberKeys(term, Object.fromEntries(plan.filter((c) => c.key).map((c) => [c.code, c.key])))
  }
  onStep('times', `${plan.length} courses`)
  const others: OtherRow[] = list.rows
    .filter((r) => !(r.term === term && isAvailable(r.status)) && !(enrolled && enrolled.options[codeOf(r.course_label)]))
    .map((r) => ({ code: codeOf(r.course_label) || r.course_label, title: r.course_label.replace(/^[^:]*:\s*/, ''), note: r.term ? `${r.term} · ${r.status}` : r.status }))
  return { term, enrolled: enrolled ?? { term, termRow: -1, rows: [], options: {}, unconfirmed: [] }, plan, others }
}

/**
 * Which enrolled courses can reuse what was remembered and which must be read: a course is reused when its enrolled classes are the
 * same as when it was read and the copy is not old. `full` reuses nothing.
 */
export function splitHeld(l: Loaded, full: boolean, now = Date.now()): { have: Record<string, { key?: Course['key']; options: ClassOption[] }>; need: string[] } {
  const cache = full ? null : loadCache()
  const have: Record<string, { key?: Course['key']; options: ClassOption[] }> = {}
  const need: string[] = []
  for (const code of Object.keys(l.enrolled.options)) {
    const e = usableHeld(cache, l.term, code, fingerprint(l.enrolled.rows, code, codeOf), now)
    if (e) have[code] = { key: e.key, options: e.options }
    else need.push(code)
  }
  return { have, need }
}

/** Remember a read of a course's combinations (for the next load). */
export function rememberHeldRead(l: Loaded, code: string, key: Course['key'] | undefined, options: ClassOption[]): void {
  if (options.length === 0) return
  rememberHeld(l.term, code, { fp: fingerprint(l.enrolled.rows, code, codeOf), key, options, at: Date.now() })
}

/**
 * The combinations PeopleSoft offers for each enrolled course, read through the swap page (one course at a time: the swap guide is
 * shared, so these reads never overlap). Each result is reported as it arrives.
 */
export async function loadHeldOptions(
  codes: string[],
  onCourse: (code: string, key: Course['key'], options: ClassOption[]) => void,
  signal: { cancelled: boolean },
): Promise<void> {
  // The server reads the courses on several PeopleSoft sessions at once (about 25 s for five courses instead of about 65 s).
  try {
    for (let i = 0; i < codes.length; i += HELD_BATCH) {
      const r = await api.heldOptions(codes.slice(i, i + HELD_BATCH))
      if (signal.cancelled) return
      for (const c of r.courses) {
        const n = normalizeCourses({ courses: [{ key: c.course_key as Course['key'], row: c.row ?? 0, sections: c.sections }] })
        const course = n.courses[0]
        if (course) onCourse(c.code, course.key, keepJoinable(course.options, false))
      }
    }
    return
  } catch {
    /* fall through to the one-at-a-time way */
  }
  const cands = await api.writeRead<{ row: number; course: string }[]>('swap_candidates')
  for (const code of codes) {
    if (signal.cancelled) return
    const cand = cands.find((c) => codeOf(c.course) === code)
    if (!cand) continue
    try {
      const r = await api.writeRead<{ course_key: Course['key']; sections: unknown }>('swap_options', { candidate_row: cand.row })
      const n = normalizeCourses({ courses: [{ key: r.course_key, row: cand.row, sections: r.sections }] })
      const c = n.courses[0]
      if (c) onCourse(code, c.key, keepJoinable(c.options, false))
    } catch {
      /* this course stays locked to what it is; the planner says its other combinations could not be read */
    }
  }
}

/** Merge so that settings saved by an older version still load after new fields are added. */
export function mergeSettings(s: Partial<Settings> | undefined): Settings {
  const d = DEFAULT_SETTINGS
  return {
    ...d,
    ...s,
    instructorScores: { ...(s?.instructorScores ?? {}) },
    timing: { ...d.timing, ...(s?.timing ?? {}) },
    weights: { ...d.weights, ...(s?.weights ?? {}) },
    rules: { ...d.rules, ...(s?.rules ?? {}) },
    freeDayScores: Array.isArray(s?.freeDayScores) && s.freeDayScores.length === 7 ? s.freeDayScores : d.freeDayScores,
  }
}

const KEY = 'schedule-planner:v2'

export interface Saved {
  settings: Settings
}

export function loadSettings(): Settings {
  let s: Partial<Settings> | undefined
  try {
    const raw = window.localStorage.getItem(KEY)
    if (raw) s = (JSON.parse(raw) as Partial<Saved>).settings
  } catch {
    /* storage blocked: defaults */
  }
  const merged = mergeSettings(s)
  // Older saves carried a keyframe curve; only the window (from, until, fade) is used now.
  const { start, end, fade } = merged.timing
  return { ...merged, timing: { start, end, fade } }
}

export interface Bootstrapped {
  loaded: Loaded
  degree: unknown
  catalogKeysFromServer: Record<string, Course['key']>
}

/**
 * Unified single-call bootstrap for Planie: fetches `classes`, `enrolled`, and `degree_tree` in parallel on the server,
 * then fetches section combinations keeping Open, Waitlist, and Closed options for instant toggle filtering.
 */
export async function bootstrapPlanie(onStep: (s: LoadStep, detail?: string) => void, opts: LoadOpts = {}): Promise<Bootstrapped> {
  const boot = await api.bootstrap(opts.termHint)
  const list = boot.classes
  const term = openTerm(list.rows)
  const rows = list.rows.filter((r) => r.term === term && isAvailable(r.status))
  onStep('plan', `${rows.length} courses`)
  const summaries = summariesOf(list.rows, term)
  const early = boot.enrolled ? { termRow: boot.enrolled.termRow, classes: boot.enrolled.classes as EnrolledRow[] } : null
  const enrolled =
    early && (!opts.termHint || term === opts.termHint)
      ? buildEnrolled(term, early.termRow, early.classes, summaries, true)
      : await readEnrolled(term || list.term_label, summaries, true)
  if (enrolled) addListedWaitlist(enrolled, list.rows, term)
  onStep('enrolled', `${Object.keys(enrolled?.options ?? {}).length} courses`)

  let plan: Course[] = []
  if (rows.length) {
    const cache = opts.full ? null : loadCache()
    const keys = cache && cache.term === term ? cache.keys : {}
    const courses: unknown[] = []
    for (let i = 0; i < rows.length; i += SECTIONS_BATCH) {
      const batch = rows.slice(i, i + SECTIONS_BATCH)
      const known = Object.fromEntries(batch.flatMap((r) => { const code = codeOf(r.course_label); return code in keys ? [[code, keys[code]]] : [] }))
      courses.push(...(await api.sections(batch.map((r) => r.row), known)).courses)
    }
    plan = normalizeCourses({ courses }).courses.map((c) => ({ ...c, options: sortBrowseable(c.options) }))
    rememberKeys(term, Object.fromEntries(plan.filter((c) => c.key).map((c) => [c.code, c.key])))
  }
  onStep('times', `${plan.length} courses`)

  const others: OtherRow[] = list.rows
    .filter((r) => !(r.term === term && isAvailable(r.status)) && !(enrolled && enrolled.options[codeOf(r.course_label)]))
    .map((r) => ({ code: codeOf(r.course_label) || r.course_label, title: r.course_label.replace(/^[^:]*:\s*/, ''), note: r.term ? `${r.term} · ${r.status}` : r.status }))
  const classStatuses: Record<string, { term: string; status: string }> = {}
  for (const r of list.rows) {
    const code = codeOf(r.course_label)
    if (code) classStatuses[code] = { term: r.term, status: r.status }
  }
  const catalogKeysFromServer = Object.fromEntries(plan.filter((c) => c.key).map((c) => [c.code, c.key]))
  return {
    loaded: { term, enrolled: enrolled ?? { term, termRow: -1, rows: [], options: {}, unconfirmed: [] }, plan, others, classStatuses },
    degree: boot.degree,
    catalogKeysFromServer,
  }
}

export function normalizeCompatibleEntry(
  entry: {
    code: string
    key?: { strm: string; crse_id: string; crse_offer_nbr: string; acad_career: string; institution: string }
    crse_id?: string
    offer_nbr?: string
    sections?: unknown
    error?: string
  },
  fallbackTitle: string,
  fallbackUnits: number | null,
  term: string,
): { course: Course; openCount: number; waitlistCount: number; closedCount: number; notOffered: boolean } | null {
  const sec = entry.sections as { options?: unknown[]; units?: string; title?: string; course?: string } | null
  const m = /^(Winter|Spring|Fall)\s+(\d{4})$/.exec(term.trim())
  const strm = m ? `1${m[2].slice(2)}${m[1] === 'Winter' ? 1 : m[1] === 'Spring' ? 5 : 8}` : '1271'
  const key = entry.key ?? {
    strm,
    crse_id: entry.crse_id ?? '000000',
    crse_offer_nbr: entry.offer_nbr ?? '1',
    acad_career: 'CRED',
    institution: 'CNAQA',
  }
  if (!sec || !Array.isArray(sec.options)) {
    return {
      course: {
        code: entry.code,
        title: fallbackTitle,
        term,
        units: fallbackUnits != null ? String(fallbackUnits) : '3.00',
        key,
        row: 0,
        options: [],
      },
      openCount: 0,
      waitlistCount: 0,
      closedCount: 0,
      notOffered: true,
    }
  }
  const n = normalizeCourses({ courses: [{ key, row: 0, sections: sec }] })
  const norm = n.courses[0]
  const opts = sortBrowseable(norm?.options ?? [])
  const openCount = opts.filter((o) => o.status.trim().toLowerCase() === 'open' && !o.waitlist && o.components.every((c) => c.seatsOpen == null || c.seatsOpen > 0)).length
  const waitlistCount = opts.filter((o) => !!o.waitlist || /wait/i.test(o.status)).length
  const closedCount = Math.max(0, opts.length - openCount - waitlistCount)
  return {
    course: {
      code: norm?.code ?? entry.code,
      title: norm?.title || sec.title || fallbackTitle,
      term: norm?.term || term,
      units: norm?.units ?? sec.units ?? (fallbackUnits != null ? String(fallbackUnits) : '3.00'),
      key,
      row: 0,
      options: opts,
    },
    openCount,
    waitlistCount,
    closedCount,
    notOffered: opts.length === 0,
  }
}

export function saveSettings(settings: Settings): void {
  try {
    window.localStorage.setItem(KEY, JSON.stringify({ settings } satisfies Saved))
  } catch {
    /* storage can be blocked; the planner works without it */
  }
}

import type { PCourse, PlannerState } from './model'

export interface TermLimit {
  max: number
  /** false when the limit is the owner's belief, not something PeopleSoft has been seen to enforce. */
  confirmed: boolean
  note: string
}

/**
 * The most credits PeopleSoft allows in one term. Fall/Winter 18 is confirmed by PeopleSoft's own refusal ("Your planning exceeded the
 * limit of 18 credits in one Fall/Winter term"). Spring 12 is believed, not yet seen.
 */
export function limitFor(term: string): TermLimit {
  if (/spring/i.test(term)) return { max: 12, confirmed: false, note: 'Spring limit not confirmed yet (12 is assumed)' }
  return { max: 18, confirmed: true, note: 'PeopleSoft allows 18 credits in a Fall or Winter term' }
}

export interface CreditLine {
  code: string
  title: string
  units: number | null
  kind: 'enrolled' | 'planned'
  /** In the student's schedule (not ticked off). */
  inSchedule: boolean
  /** Counted by PeopleSoft when Enrol is pressed after this Apply's drops. */
  counts: boolean
  /** The student is on a waitlist for it (not enrolled): PeopleSoft may or may not count its credits. UNVERIFIED. */
  waitlisted?: boolean
}

export interface WayOut {
  code: string
  title: string
  units: number | null
  /** 'unplan' for a planned course, 'drop' for an enrolled one. */
  action: 'unplan' | 'drop'
  /** Removing just this one is enough to get under the limit. */
  enough: boolean
}

export interface Credits {
  limit: TermLimit
  /** Credits of the courses in the schedule. */
  schedule: number
  /** What PeopleSoft counts today: every enrolled and every planned course of the term. */
  now: number
  /** What PeopleSoft counts when Enrol is pressed after this Apply: enrolled courses not being dropped, plus every planned course (in the schedule or not). */
  after: number
  /** Courses whose units are unknown and that count: the total is "at least". */
  unknown: string[]
  /** How many credits must go to be at the limit. */
  excess: number
  over: boolean
  /** Apply must not run: over the limit, or the total cannot be trusted because units are unknown. */
  blocked: boolean
  lines: CreditLine[]
  wayOut: WayOut[]
  /** Planned courses left out of this term's total because they belong to another term in the degree's order (only when the term would be over). */
  deferred: { code: string; to: string; units: number | null }[]
  /** Credits of waitlisted courses inside `after`. PeopleSoft may or may not count them (unverified). */
  waitlistUnits: number
  /** The total is over the limit only because of waitlisted courses: the student may override. */
  waitlistOnly: boolean
  /** The student chose to go on although credits put the total over (waitlist or intentional override). */
  overridden: boolean
  /** True when explicitly overriding the hard limit (rather than only excluding waitlisted units). */
  overrideLimit: boolean
  /** The credit total sent to the server: `after`, less the waitlisted credits when the student overrode. */
  sent: number
}

const sum = (xs: (number | null)[]) => xs.reduce<number>((a, x) => a + (x ?? 0), 0)

/**
 * The credit numbers for the open term. PeopleSoft checks the limit when Enrol is pressed (which our enrol also does to open the class list),
 * and it counts every course planned for the term whether or not the student ticked it in the schedule. So a planned course that is unticked
 * still counts until it is un-planned. Unknown units are never counted as 0: they make the total untrustworthy and block.
 */
export function creditsFor(
  courses: PCourse[],
  state: PlannerState,
  term: string,
  planning: Record<string, string> = {},
  waitlistOverride = false,
  overrideLimit = false,
): Credits {
  const limit = limitFor(term)
  const lines: CreditLine[] = courses.map((c) => {
    const inSchedule = !state.removed.includes(c.code)
    const kind = c.held ? 'enrolled' : 'planned'
    // An enrolled course taken out of the schedule is dropped by Apply, so it no longer counts at the Enrol click; a planned one still does.
    // A course added from the catalog that is not planned in PeopleSoft yet counts only while it is in the schedule.
    const counts = kind === 'enrolled' || c.addPlan ? inSchedule : true
    return { code: c.code, title: c.title, units: c.units ?? null, kind, inSchedule, counts, ...(c.held?.waitlisted ? { waitlisted: true } : {}) }
  })
  // The planning-semester rule: when the term would be over its limit, a planned course (not enrolled, not added by this schedule) that belongs
  // to another term in the degree's order is not counted here. PeopleSoft may still list it in this term; its own Enrol click is the judge.
  const total = (ls: CreditLine[]) => sum(ls.filter((l) => l.counts).map((l) => l.units))
  const deferred: { code: string; to: string; units: number | null }[] = []
  if (total(lines) > limit.max) {
    for (const l of lines) {
      const c = courses.find((x) => x.code === l.code)
      const home = planning[l.code]
      if (c && l.counts && l.kind === 'planned' && !c.addPlan && home && home !== term) {
        l.counts = false
        deferred.push({ code: l.code, to: home, units: l.units })
      }
    }
  }
  const afterLines = lines.filter((l) => l.counts)
  const schedule = sum(lines.filter((l) => l.inSchedule).map((l) => l.units))
  const now = sum(lines.map((l) => l.units))
  const after = sum(afterLines.map((l) => l.units))
  const unknown = afterLines.filter((l) => l.units === null).map((l) => l.code)
  const excess = Math.max(0, after - limit.max)
  const over = after > limit.max
  const wayOut: WayOut[] = []
  if (over) {
    const known = afterLines.filter((l) => l.units !== null)
    const order = (k: CreditLine['kind']) => known.filter((l) => l.kind === k).sort((a, b) => (b.units ?? 0) - (a.units ?? 0) || a.code.localeCompare(b.code))
    for (const l of [...order('planned'), ...order('enrolled')]) {
      wayOut.push({ code: l.code, title: l.title, units: l.units, action: l.kind === 'planned' ? 'unplan' : 'drop', enough: after - (l.units ?? 0) <= limit.max })
    }
  }
  const waitlistUnits = sum(afterLines.filter((l) => l.waitlisted).map((l) => l.units))
  const waitlistOnly = over && waitlistUnits > 0 && after - waitlistUnits <= limit.max
  const waitOverridden = waitlistOnly && waitlistOverride
  const explicitOverride = over && overrideLimit
  const overridden = waitOverridden || explicitOverride
  const sent = waitOverridden && !explicitOverride ? after - waitlistUnits : after
  return {
    limit,
    schedule,
    now,
    after,
    unknown,
    excess,
    over,
    blocked: (over && !overridden) || unknown.length > 0,
    lines,
    wayOut,
    deferred,
    waitlistUnits,
    waitlistOnly,
    overridden,
    overrideLimit: explicitOverride,
    sent,
  }
}

/** "16 / 18" style text. */
export const creditText = (n: number) => (Number.isInteger(n) ? String(n) : n.toFixed(1))

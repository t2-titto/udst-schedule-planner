import { fmtCompact } from '../engine/normalize'
import { isOnlineRoom, solve, timeScore } from '../engine/solver'
import { DAY_SHORT, type Settings } from '../engine/types'
import type { Fix } from './fixes'
import { applySuggestion, isLocked, solverCourses, type PCourse, type PlannerState } from './model'
import { effectiveSettings, formatTime } from './scale'

export interface Outlier {
  /** One sentence naming the small thing. */
  what: string
  /** How many points (0 to 10 scale, like the header score) the best schedule would gain without it. */
  gain: number
  fix: Fix
}

/** Gains smaller than this are not worth a note. */
export const MIN_GAIN = 0.15

const top = (courses: PCourse[], state: PlannerState, settings: Settings) => solve(solverCourses(courses, state), effectiveSettings(settings)).schedules[0]

/**
 * What small thing keeps the best schedule from scoring higher? Each candidate is tried by solving again with just that thing
 * changed, so the gain is real: the class that sits outside the preferred hours, a course still locked, a pin, a rule.
 * Returns the one with the biggest gain, or null when nothing is worth mentioning.
 */
export function findOutlier(courses: PCourse[], state: PlannerState, settings: Settings): Outlier | null {
  const best = top(courses, state, settings)
  if (!best) return null
  const base = best.scores.total
  if (base >= 9.5) return null
  const found: Outlier[] = []
  const add = (what: string, score: number | undefined, fix: Fix) => {
    if (score !== undefined && score - base >= MIN_GAIN) found.push({ what, gain: score - base, fix })
  }

  // 1. The class that sits furthest outside the preferred hours.
  const t = settings.timing
  let worst: { b: (typeof best.blocks)[number]; avg: number } | null = null
  for (const b of best.blocks) {
    if (b.online && settings.rules.ignoreOnline) continue
    let sum = 0
    for (let m = b.start; m < b.end; m++) sum += timeScore(m, t)
    const avg = sum / Math.max(1, b.end - b.start)
    if (avg < 9.5 && (!worst || avg < worst.avg)) worst = { b, avg }
  }
  if (worst) {
    const start = Math.min(t.start, worst.b.start)
    const end = Math.max(t.end, worst.b.end)
    const next = { ...settings, timing: { ...t, start, end } }
    const s = top(courses, state, next)?.scores.total
    add(
      `${worst.b.courseCode} ${/lab/i.test(worst.b.kind) ? 'lab' : 'lecture'} (${DAY_SHORT[worst.b.day]} ${fmtCompact(worst.b.start, worst.b.end)}) is outside your preferred ${formatTime(t.start)} to ${formatTime(t.end)}.`,
      s,
      { label: `Prefer ${formatTime(start)} to ${formatTime(end)}`, settings: (cur) => ({ ...cur, timing: { ...cur.timing, start, end } }) },
    )
  }

  // 1b. An online class that is still counted against the preferred hours.
  if (!settings.rules.ignoreOnline) {
    const online = best.blocks.find((b) => isOnlineRoom(b.room))
    if (online) {
      const next = { ...settings, rules: { ...settings.rules, ignoreOnline: true } }
      add(`${online.courseCode} ${/lab/i.test(online.kind) ? 'lab' : 'lecture'} is online (${online.room}) but still counts against your preferred hours.`, top(courses, state, next)?.scores.total, {
        label: "Don't score online classes",
        settings: (cur) => ({ ...cur, rules: { ...cur.rules, ignoreOnline: true } }),
      })
    }
  }

  // 2. A course still locked to what you have.
  for (const c of courses) {
    if (!c.held || !isLocked(c, state) || state.removed.includes(c.code) || c.options.length === 0) continue
    const next = { ...state, unlocked: [...state.unlocked, c.code] }
    const sch = top(courses, next, settings)
    add(`${c.code} is locked to what you are enrolled in.`, sch?.scores.total, {
      label: `Unlock ${c.code}`,
      state: (st) => (sch ? applySuggestion(courses, { ...st, unlocked: [...new Set([...st.unlocked, c.code])] }, sch) : st),
    })
  }

  // 3. A pin.
  for (const p of state.pins) {
    const owner = courses.find((c) => c.options.some((o) => o.components.some((x) => x.classNbr === p)))
    if (!owner) continue
    const next = { ...state, pins: state.pins.filter((x) => x !== p) }
    const sch = top(courses, next, settings)
    add(`The pin on class ${p} (${owner.code}) holds the schedule back.`, sch?.scores.total, {
      label: `Unpin ${p}`,
      state: (st) => (sch ? applySuggestion(courses, { ...st, pins: st.pins.filter((x) => x !== p) }, sch) : { ...st, pins: st.pins.filter((x) => x !== p) }),
    })
  }

  // 4. A rule.
  const r = settings.rules
  const eff = effectiveSettings(settings).rules
  const rules: { on: boolean; what: string; label: string; rules: Partial<Settings['rules']> }[] = [
    { on: r.latestEnd !== null, what: `Your latest-end rule (${r.latestEnd !== null ? formatTime(r.latestEnd) : ''}) rules out better schedules.`, label: 'Drop the latest-end rule', rules: { latestEnd: null } },
    { on: r.minGap > 0, what: `Your ${r.minGap}-minute minimum gap rules out better schedules.`, label: 'Drop the minimum gap', rules: { minGap: 0 } },
    { on: eff.minFreeDays > 0, what: `Requiring ${eff.minFreeDays} free day${eff.minFreeDays > 1 ? 's' : ''} rules out better schedules.`, label: 'Do not require free days', rules: { minFreeDays: 0 } },
    { on: r.excludeFull, what: 'Skipping full classes rules out better schedules.', label: 'Allow full classes', rules: { excludeFull: false } },
  ]
  for (const x of rules) {
    if (!x.on) continue
    const sch = top(courses, state, { ...settings, rules: { ...settings.rules, ...x.rules } })?.scores.total
    add(x.what, sch, { label: x.label, settings: (cur) => ({ ...cur, rules: { ...cur.rules, ...x.rules } }) })
  }

  found.sort((a, b) => b.gain - a.gain)
  return found[0] ?? null
}

import { explainEmpty, solve } from '../engine/solver'
import { fmtCompact } from '../engine/normalize'
import { DAY_SHORT } from '../engine/types'
import type { Settings } from '../engine/types'
import { applySuggestion, blocksOf, isLocked, overlapsOf, solverCourses, swappable, type PCourse, type PlannerState } from './model'
import { optionBlocks } from './guides'
import { effectiveSettings, formatTime } from './scale'

export interface Fix {
  label: string
  /** Changes the planner's choices. */
  state?: (s: PlannerState) => PlannerState
  /** Changes the preferences. */
  settings?: (s: Settings) => Settings
}

export interface NoSuggestions {
  reasons: string[]
  fixes: Fix[]
}

/**
 * Greedy best effort when nothing fits: every course that can move takes the combination that overlaps the fewest classes
 * already placed (locked ones first), so the calendar shows the clashes instead of staying empty.
 */
export function bestEffort(courses: PCourse[], s: PlannerState): PlannerState {
  const choice = { ...s.choice }
  const placed = courses.filter((c) => isLocked(c, s) && !s.removed.includes(c.code))
  let blocks = blocksOf(placed, s)
  for (const c of courses) {
    if (s.removed.includes(c.code) || isLocked(c, s)) continue
    const opts = c.options.filter((o) => swappable(c, o))
    if (opts.length === 0) continue
    let best = opts[0]
    let bestN = Infinity
    for (const o of opts) {
      const n = overlapsOf([...optionBlocks(c, o), ...blocks]).length
      if (n < bestN) { bestN = n; best = o }
    }
    choice[c.code] = best.id
    blocks = [...blocks, ...optionBlocks(c, best)]
  }
  return { ...s, choice }
}

const fits = (courses: PCourse[], s: PlannerState, settings: Settings) => solve(solverCourses(courses, s), effectiveSettings(settings)).schedules[0]

/** Why the ranked list is empty and what would make it non-empty. Each fix was checked by solving again. */
export function suggestFixes(courses: PCourse[], state: PlannerState, settings: Settings): NoSuggestions {
  const eff = effectiveSettings(settings)
  const sc = solverCourses(courses, state)
  const result = solve(sc, eff)
  // Locked classes that overlap each other block everything, whatever the rules say: name them first.
  const lockedNow = courses.filter((c) => isLocked(c, state) && !state.removed.includes(c.code))
  const lockedClash = overlapsOf(blocksOf(lockedNow, state))
  const reasons = lockedClash.length
    ? lockedClash.map((o) => `${o.a.courseCode} and ${o.b.courseCode} are locked and overlap (${DAY_SHORT[o.day]} ${fmtCompact(o.start, o.end)}). Unlock one of them to move it.`)
    : explainEmpty(sc, result, eff)
  const fixes: Fix[] = []
  const r = settings.rules

  // 1. Loosen the rule that blocks.
  const loosen: { on: boolean; label: string; rules: Partial<Settings['rules']> }[] = [
    { on: r.latestEnd !== null, label: `Allow classes after ${r.latestEnd !== null ? formatTime(r.latestEnd) : ''}`, rules: { latestEnd: null } },
    { on: r.minGap > 0, label: 'Allow back-to-back classes (no minimum gap)', rules: { minGap: 0 } },
    { on: eff.rules.minFreeDays > 0, label: 'Do not require free days', rules: { minFreeDays: 0 } },
    { on: r.excludeFull, label: 'Allow full classes', rules: { excludeFull: false } },
  ]
  for (const l of loosen) {
    if (!l.on) continue
    const next = { ...settings, rules: { ...settings.rules, ...l.rules } }
    if (fits(courses, state, next)) fixes.push({ label: l.label, settings: (s) => ({ ...s, rules: { ...s.rules, ...l.rules } }) })
  }

  // 2. Unlock an enrolled course whose fixed classes block.
  for (const c of courses) {
    if (!c.held || !isLocked(c, state) || state.removed.includes(c.code) || c.options.length === 0) continue
    const next = { ...state, unlocked: [...state.unlocked, c.code] }
    const s = fits(courses, next, settings)
    if (s) fixes.push({ label: `Unlock ${c.code}`, state: (st) => applySuggestion(courses, { ...st, unlocked: [...new Set([...st.unlocked, c.code])] }, s) })
  }

  // 3. Remove a course that cannot fit (one that is not yet enrolled first).
  const removable = courses.filter((c) => !state.removed.includes(c.code)).sort((a, b) => Number(!!a.held) - Number(!!b.held))
  for (const c of removable) {
    if (fixes.length >= 6) break
    const next = { ...state, removed: [...state.removed, c.code] }
    const s = fits(courses, next, settings)
    if (s) fixes.push({ label: c.held ? `Remove ${c.code} (drops it when you apply) and fit the rest` : `Remove ${c.code} and fit the rest`, state: (st) => applySuggestion(courses, { ...st, removed: [...new Set([...st.removed, c.code])] }, s) })
  }

  // 4. Always possible: show the least bad calendar, with the clashes marked.
  fixes.push({ label: 'Show the least overlapping calendar anyway', state: (st) => bestEffort(courses, st) })
  return { reasons, fixes }
}

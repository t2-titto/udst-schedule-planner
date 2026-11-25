import { solve } from '../engine/solver'
import type { Settings } from '../engine/types'
import { applySuggestion, solverCourses, type PCourse, type PlannerState } from './model'
import { effectiveSettings } from './scale'

/**
 * The calendar following the preferences: every course that can move takes the combination of the best-ranked schedule for the
 * current locks, pins and preferences. Returns the same object when nothing would change, so no history step is made for it.
 */
export function followBest(courses: PCourse[], state: PlannerState, settings: Settings): PlannerState {
  const best = solve(solverCourses(courses, state), effectiveSettings(settings)).schedules[0]
  if (!best) return state
  const next = applySuggestion(courses, state, best)
  const same = Object.keys(next.choice).length === Object.keys(state.choice).length && Object.entries(next.choice).every(([k, v]) => state.choice[k] === v)
  return same ? state : next
}

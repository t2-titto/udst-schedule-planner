import { solve, type Schedule } from '../engine/solver'
import type { Settings } from '../engine/types'
import { redoChange, solverCourses, type PCourse, type PlannerState } from './model'
import { effectiveSettings } from './scale'

export interface RedoSuggestion {
  schedule: Schedule
  /** The enrolled courses this schedule changes only by dropping and enrolling again. */
  redoCourses: string[]
  /** Points above the best schedule that needs no drop and re-enrol (the whole score when there is none). */
  gain: number
}

/** A drop and re-enrol suggestion is only worth showing when it beats the best ordinary one by at least this much. */
export const MIN_REDO_GAIN = 0.05

/**
 * Schedules that become possible only by accepting a drop and a new enrolment for some enrolled course, shown only when that matters:
 * each one must really need such a change and score higher than the best schedule without one (or there is no ordinary schedule at all).
 * A new enrolment, a swap or a kept course never makes a schedule appear here.
 */
export function redoSuggestions(courses: PCourse[], state: PlannerState, settings: Settings, limit = 5): RedoSuggestion[] {
  const eff = effectiveSettings(settings)
  const normalTop = solve(solverCourses(courses, state), eff).schedules[0]?.scores.total ?? null
  const withRedo = solve(solverCourses(courses, state, true), eff)
  const out: RedoSuggestion[] = []
  for (const sch of withRedo.schedules) {
    const redoCourses = sch.picks.filter((p) => {
      const c = courses.find((x) => x.code === p.course.code)
      const o = c?.options.find((x) => x.id === p.option.id)
      return !!c && !!o && redoChange(c, o)
    }).map((p) => p.course.code)
    if (redoCourses.length === 0) continue
    const gain = normalTop === null ? sch.scores.total : sch.scores.total - normalTop
    if (gain < MIN_REDO_GAIN) continue
    out.push({ schedule: sch, redoCourses, gain })
    if (out.length >= limit) break
  }
  return out
}

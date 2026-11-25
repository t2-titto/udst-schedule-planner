import { activeDays, scoreSelection } from '../engine/solver'
import type { Settings, ClassOption } from '../engine/types'
import { blocksOf, courseOf, effective, isLocked, overlapsOf, solverCourses, redoChange, swappable, type PBlock, type PCourse, type PlannerState } from './model'
import { effectiveSettings } from './scale'

export interface Span { day: number; start: number; end: number }

/** One real place a dragged class can land. */
export interface Guide {
  optionId: string
  /** Where the dragged kind of class would meet (every meeting of it). */
  here: Span[]
  /** Where the course's other classes would meet if this were chosen (shown much fainter). */
  others: (Span & { kind: string })[]
  /** The whole course would overlap another course here. */
  clash: boolean
  /** Total score of the whole schedule with this combination (engine scale, 0 to 10). */
  score: number
  current: boolean
  /** Only the lab changes: it would be a drop and a new enrolment, not a swap. */
  redo: boolean
}

export const optionBlocks = (c: PCourse, o: ClassOption): PBlock[] =>
  o.components.flatMap((comp) =>
    comp.meetings.map((m) => ({ courseCode: c.code, classNbr: comp.classNbr, kind: comp.kind, room: comp.room, instructors: comp.instructors, day: m.day, start: m.start, end: m.end, locked: false, held: !!c.held })),
  )

const rank = (g: Guide) => (g.current ? 4 : 0) + (g.redo ? 0 : 2) + (g.clash ? 0 : 1)

/**
 * The real positions a class can be dragged to: one per distinct place of that kind of class, taken from the best combination
 * that has it (the current one, else one without overlaps, else the best score). Overlapping places are included, marked.
 */
export function guidesFor(courses: PCourse[], state: PlannerState, b: PBlock, settings: Settings): Guide[] {
  const c = courses.find((x) => x.code === b.courseCode)
  if (!c || isLocked(c, state) || state.removed.includes(c.code)) return []
  const eff = effectiveSettings(settings)
  const cur = effective(c, state)
  const rest = courses.filter((x) => x.code !== c.code)
  const others = blocksOf(rest, state)
  const picks = rest.flatMap((x) => {
    const o = effective(x, state)
    return o ? [{ course: courseOf(x), option: o }] : []
  })
  const days = activeDays(solverCourses(courses, state))
  const byPlace = new Map<string, Guide>()
  for (const o of c.options) {
    const redo = redoChange(c, o)
    if (!swappable(c, o) && !redo) continue
    const same = o.components.filter((x) => x.kind === b.kind)
    const here = same.flatMap((x) => x.meetings.map((m) => ({ day: m.day, start: m.start, end: m.end })))
    if (here.length === 0) continue
    const g: Guide = {
      optionId: o.id,
      here,
      others: o.components.filter((x) => x.kind !== b.kind).flatMap((x) => x.meetings.map((m) => ({ day: m.day, start: m.start, end: m.end, kind: x.kind }))),
      clash: overlapsOf([...optionBlocks(c, o), ...others]).length > 0,
      score: scoreSelection([...picks, { course: courseOf(c), option: o }], days, eff).total,
      current: o.id === cur?.id,
      redo,
    }
    const key = here.map((m) => `${m.day}:${m.start}:${m.end}`).sort().join('|')
    const old = byPlace.get(key)
    if (!old || rank(g) > rank(old) || (rank(g) === rank(old) && g.score > old.score)) byPlace.set(key, g)
  }
  return [...byPlace.values()]
}

/** The guide whose place of the dragged class is nearest a point (day, start minute); null when there are none. */
export function nearestGuide(guides: Guide[], day: number, start: number): Guide | null {
  let best: { g: Guide; d: number } | null = null
  for (const g of guides) {
    for (const m of g.here) {
      const d = Math.abs(m.day - day) * 600 + Math.abs(m.start - start)
      if (!best || d < best.d) best = { g, d }
    }
  }
  return best ? best.g : null
}

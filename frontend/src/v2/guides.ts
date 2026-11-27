import { activeDays, scoreSelection } from '../engine/solver'
import type { Settings, ClassOption, Component } from '../engine/types'
import {
  blocksOf,
  courseOf,
  effective,
  isLocked,
  overlapsOf,
  solverCourses,
  redoChange,
  swappable,
  type PBlock,
  type PCourse,
  type PlannerState,
} from './model'
import { effectiveSettings } from './scale'

export interface Span {
  day: number
  start: number
  end: number
}

export interface LinkedGhost extends Span {
  kind: string
  teacher: string
  section: string
}

export interface SlotChoice {
  option: ClassOption
  teacher: string
  section: string
  seatsOpen: number | null
  seatsTotal: number | null
  status: 'Open' | 'Waitlist' | 'Closed'
  clash: boolean
  clashesWith: string[]
  redo: boolean
  others: LinkedGhost[]
  score: number
}

/** One real place on the timetable a dragged class can land (possibly offering multiple teachers at identical times). */
export interface Guide {
  optionId: string
  option: ClassOption
  /** Where the dragged component meets. */
  here: Span[]
  /** Linked Lab/Tut destinations (shown faintly while dragging). */
  others: LinkedGhost[]
  /** Teachers & sections available at this exact same day/time slot (length >= 2 triggers Same-Time Picker). */
  optionsAtSlot: SlotChoice[]
  teacher: string
  section: string
  seatsOpen: number | null
  seatsTotal: number | null
  status: 'Open' | 'Waitlist' | 'Closed'
  clash: boolean
  clashesWith: string[]
  score: number
  current: boolean
  /** True when only the lab changes on an already-enrolled course (drop + re-enrol seat risk). */
  redo: boolean
}

export interface GuideFilters {
  open: boolean
  waitlist: boolean
  closed: boolean
  allowRedo: boolean
}

export const DEFAULT_GUIDE_FILTERS: GuideFilters = {
  open: true,
  waitlist: false,
  closed: false,
  allowRedo: false,
}

export const optionBlocks = (c: PCourse, o: ClassOption): PBlock[] =>
  o.components.flatMap((comp) =>
    comp.meetings.map((m) => ({
      courseCode: c.code,
      classNbr: comp.classNbr,
      kind: comp.kind,
      room: comp.room,
      instructors: comp.instructors,
      day: m.day,
      start: m.start,
      end: m.end,
      locked: false,
      held: !!c.held,
    })),
  )

export function optionStatusOf(o: ClassOption): 'Open' | 'Waitlist' | 'Closed' {
  const s = o.status.trim().toLowerCase()
  if (o.waitlist || s.includes('wait')) return 'Waitlist'
  if (s === 'open' && o.components.every((c) => c.seatsOpen == null || c.seatsOpen > 0)) return 'Open'
  return 'Closed'
}

export function optionAllowedByFilters(o: ClassOption, filters: GuideFilters, isCurrent: boolean): boolean {
  if (isCurrent) return true
  const st = optionStatusOf(o)
  if (st === 'Open') return filters.open
  if (st === 'Waitlist') return filters.waitlist
  return filters.closed
}

export function teacherOfComp(comp: Component | undefined): string {
  const list = comp?.instructors?.filter((x) => x && x !== 'Staff')
  if (list && list.length > 0) return list[0]
  return comp?.instructors?.[0] ?? 'Staff'
}

const rankChoice = (c: SlotChoice, currentId?: string) =>
  (c.option.id === currentId ? 8 : 0) +
  (c.status === 'Open' ? 4 : c.status === 'Waitlist' ? 2 : 0) +
  (!c.redo ? 2 : 0) +
  (!c.clash ? 1 : 0)

/**
 * Group compatible options for `b` by their primary meeting time slot so multiple instructors meeting at the
 * identical slot consolidate cleanly into one dotted target with an `N teachers` pill.
 */
export function guidesFor(
  courses: PCourse[],
  state: PlannerState,
  b: PBlock,
  settings: Settings,
  filters: GuideFilters = DEFAULT_GUIDE_FILTERS,
): Guide[] {
  const c = courses.find((x) => x.code === b.courseCode)
  if (!c || isLocked(c, state) || state.removed.includes(c.code)) return []
  const eff = effectiveSettings(settings)
  const cur = effective(c, state)
  const rest = courses.filter((x) => x.code !== c.code)
  const othersBlocks = blocksOf(rest, state)
  const picks = rest.flatMap((x) => {
    const o = effective(x, state)
    return o ? [{ course: courseOf(x), option: o }] : []
  })
  const days = activeDays(solverCourses(courses, state))
  const byPlace = new Map<string, { here: Span[]; choices: SlotChoice[] }>()

  for (const o of c.options) {
    const isCur = o.id === cur?.id
    if (!optionAllowedByFilters(o, filters, isCur)) continue
    const redo = redoChange(c, o)
    if (!swappable(c, o) && !redo) continue
    if (redo && !filters.allowRedo && !isCur) continue

    const same = o.components.filter((x) => x.kind === b.kind)
    const primaryComp = same[0]
    const here = same.flatMap((x) => x.meetings.map((m) => ({ day: m.day, start: m.start, end: m.end })))
    if (here.length === 0) continue

    const others: LinkedGhost[] = o.components
      .filter((x) => x.kind !== b.kind)
      .flatMap((x) =>
        x.meetings.map((m) => ({
          day: m.day,
          start: m.start,
          end: m.end,
          kind: x.kind,
          teacher: teacherOfComp(x),
          section: x.section,
        })),
      )

    const ols = overlapsOf([...optionBlocks(c, o), ...othersBlocks])
    const clashesWith = [...new Set(ols.flatMap((ol) => [ol.a.courseCode, ol.b.courseCode]).filter((x) => x !== c.code))]
    const choice: SlotChoice = {
      option: o,
      teacher: teacherOfComp(primaryComp),
      section: primaryComp?.section ?? o.optionNo,
      seatsOpen: primaryComp?.seatsOpen ?? null,
      seatsTotal: primaryComp?.seatsTotal ?? null,
      status: optionStatusOf(o),
      clash: clashesWith.length > 0,
      clashesWith,
      redo,
      others,
      score: scoreSelection([...picks, { course: courseOf(c), option: o }], days, eff).total,
    }

    const key = here.map((m) => `${m.day}:${m.start}:${m.end}`).sort().join('|')
    const group = byPlace.get(key) ?? { here, choices: [] }
    group.choices.push(choice)
    byPlace.set(key, group)
  }

  const out: Guide[] = []
  for (const { here, choices } of byPlace.values()) {
    choices.sort((a, b) => rankChoice(b, cur?.id) - rankChoice(a, cur?.id) || b.score - a.score)
    const best = choices[0]
    out.push({
      optionId: best.option.id,
      option: best.option,
      here,
      others: best.others,
      optionsAtSlot: choices,
      teacher: best.teacher,
      section: best.section,
      seatsOpen: best.seatsOpen,
      seatsTotal: best.seatsTotal,
      status: best.status,
      clash: best.clash,
      clashesWith: best.clashesWith,
      score: best.score,
      current: best.option.id === cur?.id,
      redo: best.redo,
    })
  }
  return out
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

import { conflicts } from './engine/solver'
import type { Schedule } from './engine/solver'
import type { ClassOption, Component, Course } from './engine/types'
import type { EnrolledRow } from './lock'
import { codeOf, primaryClass } from './lock'

/** `redo`: the change touches only the lab, which PeopleSoft refuses as a swap, so it is done as a drop and a new enrolment. */
export interface PlanSwap { course: Course; now: ClassOption; next: ClassOption; blocker?: string; redo?: boolean }
export interface PlanEnrol { course: Course; next: ClassOption; blocker?: string; depends: string[] }
export interface PlanDrop { code: string; title: string; classNbr: string; classes: string[] }

export interface Plan {
  keeps: { code: string; now: ClassOption }[]
  swaps: PlanSwap[]
  enrols: PlanEnrol[]
  drops: PlanDrop[]
  /** Things that make the plan unsafe to send: a lab-only swap, a clash with a class you hold, a course not fetched live. */
  blockers: string[]
  /** One line for the review: every new class was checked against every class held, hidden ones included. */
  checked: number
}

const isLab = (c: Component) => /lab/i.test(c.kind)
const nbrs = (o: ClassOption) => o.components.map((c) => c.classNbr)

/** The class you hold already covers every class this option needs: nothing to do for the course. */
export const sameClasses = (held: ClassOption, next: ClassOption): boolean => {
  const h = new Set(nbrs(held))
  return next.components.length > 0 && next.components.every((c) => h.has(c.classNbr))
}

/**
 * PeopleSoft refused a swap that changed only the lab (same lecture) when it was tried live on 2026-11-16, and an older capture says
 * the same: such a swap is not sent.
 */
export const labOnly = (held: ClassOption, next: ClassOption): boolean => {
  const lec = (o: ClassOption) => nbrs({ ...o, components: o.components.filter((c) => !isLab(c)) }).sort().join(',')
  return lec(held) !== '' && lec(held) === lec(next)
}

const meets = (o: ClassOption) => o.components.flatMap((c) => c.meetings.map((m) => ({ ...m, nbr: c.classNbr })))

/** First pair of classes from different courses that overlap, as text. */
export function firstClash(sets: { code: string; option: ClassOption }[]): string | null {
  for (let i = 0; i < sets.length; i++) {
    for (let j = i + 1; j < sets.length; j++) {
      for (const a of meets(sets[i].option)) {
        for (const b of meets(sets[j].option)) {
          if (conflicts(a, b, 0)) return `${sets[i].code} (class ${a.nbr}) overlaps ${sets[j].code} (class ${b.nbr})`
        }
      }
    }
  }
  return null
}

/**
 * What pressing Enrol must do to turn what the student holds into the chosen schedule:
 *  keep what already matches (same class numbers), swap the rest of the held courses in one go, enrol the courses not held, and
 *  offer to drop held courses that are no longer in the plan. Every class held counts for clashes, hidden courses included.
 */
export function buildPlan(
  schedule: Schedule,
  held: Record<string, ClassOption>,
  rows: EnrolledRow[],
  ctx: { selected: string[]; hidden: string[]; courses: Course[]; live: Set<string> },
): Plan {
  const keeps: Plan['keeps'] = []
  const swaps: PlanSwap[] = []
  const enrols: PlanEnrol[] = []
  const blockers: string[] = []
  for (const p of schedule.picks) {
    const h = held[p.course.code]
    if (!h) {
      const blocker = !ctx.live.has(p.course.code) || p.option.optionRow === undefined || !p.course.key ? `${p.course.code}: fetch it from PeopleSoft first (its live row or key is missing).` : undefined
      enrols.push({ course: p.course, next: p.option, blocker, depends: [] })
    } else if (p.option.id === h.id || sameClasses(h, p.option)) {
      keeps.push({ code: p.course.code, now: h })
    } else {
      let blocker: string | undefined
      const redo = labOnly(h, p.option)
      if (!redo && (p.option.optionRow === undefined || !p.course.key)) blocker = `${p.course.code}: fetch it from PeopleSoft first (its live key is missing).`
      swaps.push({ course: p.course, now: h, next: p.option, blocker, redo })
    }
  }
  for (const s of swaps) if (s.blocker) blockers.push(s.blocker)
  for (const e of enrols) if (e.blocker) blockers.push(e.blocker)

  // Held courses that are not part of the plan: hidden ones are only hidden (never dropped); unticked ones are offered for a drop.
  const inPlan = new Set(schedule.picks.map((p) => p.course.code))
  const drops: PlanDrop[] = []
  for (const code of Object.keys(held)) {
    if (inPlan.has(code) || ctx.hidden.includes(code) || ctx.selected.includes(code)) continue
    const mine = rows.filter((r) => codeOf(r.course) === code)
    if (!mine.length) continue
    const main = primaryClass(mine) ?? mine[0]
    drops.push({ code, title: main.course.replace(code, '').trim(), classNbr: main.class_nbr, classes: mine.map((r) => r.class_nbr) })
  }
  const dropped = new Set(drops.map((d) => d.code))

  // The schedule as it will stand: kept and hidden classes as held, new classes from swaps and enrolments.
  const final: { code: string; option: ClassOption }[] = []
  for (const k of keeps) final.push({ code: k.code, option: k.now })
  for (const s of swaps) final.push({ code: s.course.code, option: s.next })
  for (const e of enrols) final.push({ code: e.course.code, option: e.next })
  for (const code of Object.keys(held)) if (!inPlan.has(code) && !dropped.has(code)) final.push({ code, option: held[code] })
  const clash = firstClash(final)
  if (clash) blockers.push(`Clash: ${clash}.`)

  // An enrolment that overlaps a combination a swap gives up can only go in after that swap worked.
  for (const e of enrols) {
    for (const s of swaps) {
      const clashes = firstClash([{ code: e.course.code, option: e.next }, { code: s.course.code, option: s.now }])
      if (clashes && !e.depends.includes(s.course.code)) e.depends.push(s.course.code)
    }
    // The same for a course that is being dropped: PeopleSoft does not refuse an enrolment that overlaps a class still held (seen live
    // 2026-11-16), so if the drop does not work the enrolment must not be sent.
    for (const d of drops) {
      const h = held[d.code]
      if (h && firstClash([{ code: e.course.code, option: e.next }, { code: d.code, option: h }]) && !e.depends.includes(d.code)) e.depends.push(d.code)
    }
  }
  return { keeps, swaps, enrols, drops, blockers, checked: final.reduce((n, f) => n + f.option.components.length, 0) }
}

/** How many courses a schedule would change: the number the "fewest changes" weight works on. */
export const changesOf = (schedule: Schedule, held: Record<string, ClassOption>): number =>
  schedule.picks.filter((p) => {
    const h = held[p.course.code]
    return !h || !(p.option.id === h.id || sameClasses(h, p.option))
  }).length

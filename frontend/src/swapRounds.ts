import { conflicts } from './engine/solver'
import { fmtCompact } from './engine/normalize'
import { DAY_NAMES, type ClassOption, type Meeting } from './engine/types'

/** One course to swap: what is held now and what it should become. */
export interface RoundSwap {
  code: string
  now: ClassOption
  next: ClassOption
  /** Fewest open seats among the new classes (null when unknown). Used to choose which course to drop and re-enrol in a loop. */
  seatsLeft: number | null
}

/** A new class that overlaps a class another course holds today: that course has to move first. */
export interface Blocker {
  /** The course whose current class is in the way. */
  by: string
  newClass: string
  heldClass: string
  day: number
  start: number
  end: number
}

export interface RoundPlan {
  /** Swaps that can go out together, in the order of the rounds. */
  rounds: RoundSwap[][]
  /** For each swap, the courses whose swap has to be done first, with the overlap that says so. */
  waits: Record<string, Blocker[]>
  /** Swaps that cannot be done as swaps because they wait for each other. They become a drop and a new enrolment. */
  converted: RoundSwap[]
  /** The loops that were found, before they were broken. */
  loops: string[][]
  /** Courses that were forced into a drop and a new enrolment (a lab-only change), as opposed to ones chosen to break a loop. */
  forced: string[]
}

const meetingsOf = (o: ClassOption) => o.components.flatMap((c) => c.meetings.map((m): Meeting & { nbr: string } => ({ ...m, nbr: c.classNbr })))

/** Every place the new classes of `a` overlap the classes of `b`. */
export function overlapsBetween(a: ClassOption, b: ClassOption): { newClass: string; heldClass: string; day: number; start: number; end: number }[] {
  const out: { newClass: string; heldClass: string; day: number; start: number; end: number }[] = []
  for (const x of meetingsOf(a)) {
    for (const y of meetingsOf(b)) {
      if (conflicts(x, y, 0)) out.push({ newClass: x.nbr, heldClass: y.nbr, day: x.day, start: Math.max(x.start, y.start), end: Math.min(x.end, y.end) })
    }
  }
  return out
}

/** Strongly connected sets (size 2 or more) of "waits for" edges: the swaps that wait for each other. */
function loopsOf(codes: string[], edges: Map<string, Set<string>>): string[][] {
  const index = new Map<string, number>()
  const low = new Map<string, number>()
  const onStack = new Set<string>()
  const stack: string[] = []
  const out: string[][] = []
  let n = 0
  const visit = (v: string) => {
    index.set(v, n)
    low.set(v, n)
    n++
    stack.push(v)
    onStack.add(v)
    for (const w of edges.get(v) ?? []) {
      if (!index.has(w)) {
        visit(w)
        low.set(v, Math.min(low.get(v)!, low.get(w)!))
      } else if (onStack.has(w)) low.set(v, Math.min(low.get(v)!, index.get(w)!))
    }
    if (low.get(v) === index.get(v)) {
      const comp: string[] = []
      let w: string
      do {
        w = stack.pop()!
        onStack.delete(w)
        comp.push(w)
      } while (w !== v)
      if (comp.length > 1) out.push(comp.sort())
    }
  }
  for (const c of codes) if (!index.has(c)) visit(c)
  return out
}

/**
 * PeopleSoft judges each swap against what is held at that moment, not against the finished schedule (seen live 2026-11-16: two swaps
 * whose new classes overlapped another course's current classes were refused although the final schedule had no overlap).
 * So a swap can only go out when its new classes overlap nothing that is still held: not a fixed course, not a combination another
 * swap in the same round or a later one is about to give up. Swaps that pass together go out together; the rest wait for the round
 * after the swaps they hit have been done. Swaps that wait for each other cannot be ordered: the one with the most seats left is
 * turned into a drop followed by a new enrolment (after the swaps, when its old classes no longer matter).
 *
 * `fixed` are the classes of held courses that do not change.
 */
export function planSwapRounds(all: RoundSwap[], fixed: ClassOption[], forced: string[] = []): RoundPlan {
  const loops: string[][] = []
  const converted: RoundSwap[] = all.filter((s) => forced.includes(s.code))
  let swaps = all.filter((s) => !forced.includes(s.code))
  // Break loops one course at a time until the rest can be ordered.
  for (let guard = 0; guard < all.length + 1; guard++) {
    const edges = new Map<string, Set<string>>()
    for (const s of swaps) {
      for (const t of swaps) {
        if (s.code !== t.code && overlapsBetween(s.next, t.now).length) edges.set(s.code, new Set([...(edges.get(s.code) ?? []), t.code]))
      }
    }
    const found = loopsOf(swaps.map((s) => s.code), edges)
    if (!found.length) break
    loops.push(...found)
    for (const loop of found) {
      const members = swaps.filter((s) => loop.includes(s.code))
      // The course with the most seats left goes (a seat left at a more crowded class is the one we least want to lose).
      members.sort((a, b) => (b.seatsLeft ?? 0) - (a.seatsLeft ?? 0) || a.code.localeCompare(b.code))
      const pick = members[0]
      converted.push(pick)
      swaps = swaps.filter((s) => s.code !== pick.code)
    }
  }

  const rounds: RoundSwap[][] = []
  const done: RoundSwap[] = []
  let remaining = [...swaps]
  const hitsFixed = (s: RoundSwap) => fixed.some((f) => overlapsBetween(s.next, f).length > 0)
  while (remaining.length) {
    const round = remaining.filter((s) => !hitsFixed(s) && remaining.every((t) => t.code === s.code || overlapsBetween(s.next, t.now).length === 0))
    if (!round.length) break // a swap that clashes with a fixed course: the plan's own clash check reports it
    rounds.push(round)
    done.push(...round)
    remaining = remaining.filter((s) => !round.includes(s))
  }
  // Whatever is left clashes with a fixed course; keep it in a last round so it is still sent and PeopleSoft's answer is shown.
  if (remaining.length) rounds.push(remaining)

  const waits: Record<string, Blocker[]> = {}
  rounds.forEach((round, r) => {
    for (const s of round) {
      const list: Blocker[] = []
      for (const t of [...rounds.slice(0, r).flat(), ...converted]) {
        for (const o of overlapsBetween(s.next, t.now)) list.push({ by: t.code, ...o })
      }
      if (list.length) waits[s.code] = list
    }
  })
  return { rounds, waits, converted, loops, forced: converted.filter((c) => forced.includes(c.code)).map((c) => c.code) }
}

export const blockerText = (b: Blocker): string =>
  `waits for ${b.by} to leave ${DAY_NAMES[b.day]} ${fmtCompact(b.start, b.end)} (its class ${b.heldClass} overlaps the new class ${b.newClass})`

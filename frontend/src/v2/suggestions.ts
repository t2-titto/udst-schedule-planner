import { solve, type Schedule } from '../engine/solver'
import { DAY_SHORT, instructorKey, type Settings } from '../engine/types'
import { asSchedule, blocksOf, effective, overlapsOf, solverCourses, type PCourse, type PlannerState } from './model'

export type SetupFreeDay = 'any' | 0 | 1 | 2 | 3 | 4
export type SetupTimePref = 'any' | 'morning' | 'afternoon' | 'evening'
export type SetupPacing = 'any' | 'compact' | 'gapped'

export interface QuickSetup {
  freeDay: SetupFreeDay
  timePref: SetupTimePref
  /** Preferred free days (0=Sun..4=Thu). Empty inherits single `freeDay` when set. */
  freeDays?: number[]
  /** Avoid classes starting before 9:30 AM. */
  noEarly8am?: boolean
  /** Prefer finishing before 5:00 PM. */
  endBefore5pm?: boolean
  /** Spacing between consecutive classes on the same day. */
  pacing?: SetupPacing
  /** Require strict compliance first, falling back with a notice if impossible. */
  strict?: boolean
}

export const DEFAULT_QUICK_SETUP: QuickSetup = {
  freeDay: 'any',
  timePref: 'any',
  freeDays: [],
  noEarly8am: false,
  endBefore5pm: false,
  pacing: 'any',
  strict: false,
}

export function activeFreeDaysOf(setup: QuickSetup): number[] {
  if (setup.freeDays && setup.freeDays.length > 0) return [...new Set(setup.freeDays)].sort((a, b) => a - b)
  return typeof setup.freeDay === 'number' ? [setup.freeDay] : []
}

export function countActivePrefs(setup: QuickSetup): number {
  let n = activeFreeDaysOf(setup).length
  if (setup.timePref !== 'any') n += 1
  if (setup.noEarly8am) n += 1
  if (setup.endBefore5pm) n += 1
  if (setup.pacing && setup.pacing !== 'any') n += 1
  if (setup.strict) n += 1
  return n
}

export interface SuggestionCard {
  id: string
  label: string
  detail: string
  schedule: Schedule
  usesWaitlist?: boolean
  skippedCodes?: string[]
  isOriginal?: boolean
}

function buildSearchSettings(
  base: Settings,
  setup: QuickSetup,
  favTeachers: string[],
  courses: PCourse[],
  variant: { freeDay?: number; startPreference?: 'early' | 'late' | 'evening'; joinWaitlist?: boolean },
): Settings {
  const s: Settings = JSON.parse(JSON.stringify(base))
  s.freeDayScores = [5, 5, 5, 5, 5, 5, 5]
  const freeDays = activeFreeDaysOf(setup)
  s.rules = {
    ...s.rules,
    joinWaitlist: !!variant.joinWaitlist,
    ignoreOnline: true,
    minGap: setup.pacing === 'gapped' ? Math.max(s.rules.minGap, 30) : setup.pacing === 'compact' ? 0 : s.rules.minGap,
  }
  const wantedFreeList = variant.freeDay != null ? [variant.freeDay] : freeDays
  for (const d of wantedFreeList) {
    if (d >= 0 && d <= 4) s.freeDayScores[d] = 10
  }
  s.weights.freeDay = wantedFreeList.length > 0 ? 10 : 5

  const pref =
    variant.startPreference ??
    (setup.timePref === 'morning' ? 'early' : setup.timePref === 'afternoon' ? 'late' : setup.timePref === 'evening' ? 'evening' : 'early')
  let startMin = pref === 'early' ? 8 * 60 + 30 : pref === 'evening' ? 15 * 60 + 30 : 11 * 60
  let endMin = pref === 'early' ? 13 * 60 + 30 : pref === 'evening' ? 22 * 60 : 17 * 60
  if (setup.noEarly8am) startMin = Math.max(startMin, 9 * 60 + 30)
  if (setup.endBefore5pm) endMin = Math.min(endMin, 17 * 60)
  if (endMin <= startMin) endMin = startMin + 120
  s.timing = { ...s.timing, start: startMin, end: endMin }
  s.weights.instructor = favTeachers.length ? 8 : 5

  // Lightly boost any favorite instructors without exposing numeric sliders
  const favSet = new Set(favTeachers.map((t) => t.trim().toLowerCase()))
  for (const c of courses) {
    for (const o of c.options) {
      for (const comp of o.components) {
        for (const inst of comp.instructors) {
          if (favSet.has(inst.trim().toLowerCase())) {
            s.instructorScores[instructorKey(c.code, inst)] = 10
          }
        }
      }
    }
  }
  return s
}

function describeSchedule(s: Schedule): { label: string; detail: string } {
  const blocks = s.blocks
  const usedDays = new Set(blocks.map((b) => b.day))
  const freeDays = [0, 1, 2, 3, 4].filter((d) => !usedDays.has(d))
  const earliest = blocks.length ? Math.min(...blocks.map((b) => b.start)) : 9 * 60
  const latest = blocks.length ? Math.max(...blocks.map((b) => b.end)) : 12 * 60

  let label = 'Balanced'
  if (freeDays.length >= 2) {
    label = `${freeDays.slice(0, 2).map((d) => DAY_SHORT[d]).join(' & ')} free`
  } else if (freeDays.length === 1) {
    label = `${DAY_SHORT[freeDays[0]]} free`
  } else if (earliest >= 10 * 60) {
    label = 'No 8 AMs'
  } else if (latest <= 13 * 60 + 30) {
    label = 'Mornings'
  } else {
    label = 'Compact'
  }

  const fmtHour = (m: number) => {
    const h = Math.floor(m / 60)
    const min = m % 60
    const ap = h >= 12 ? 'p' : 'a'
    const h12 = h % 12 === 0 ? 12 : h % 12
    return min === 0 ? `${h12}${ap}` : `${h12}:${String(min).padStart(2, '0')}${ap}`
  }
  const detail = blocks.length ? `${fmtHour(earliest)}–${fmtHour(latest)}` : ''
  return { label, detail }
}

function scheduleSignature(s: Schedule): string {
  return s.picks
    .map((x) => `${x.course.code}:${x.option.id}`)
    .sort()
    .join('|')
}

export function currentScheduleSignature(courses: PCourse[], state: PlannerState): string {
  return courses
    .filter((c) => !state.removed.includes(c.code))
    .map((c) => `${c.code}:${effective(c, state)?.id ?? ''}`)
    .sort()
    .join('|')
}

function matchesStrictSetup(sched: Schedule, setup: QuickSetup): boolean {
  const wantedFree = new Set(activeFreeDaysOf(setup))
  for (const b of sched.blocks) {
    if (b.online) continue
    if (wantedFree.has(b.day)) return false
    if (setup.noEarly8am && b.start < 9 * 60 + 30) return false
    if (setup.endBefore5pm && b.end > 17 * 60) return false
  }
  if (setup.pacing === 'gapped') {
    for (let day = 0; day < 5; day++) {
      const dayBlocks = sched.blocks.filter((b) => !b.online && b.day === day).sort((a, b) => a.start - b.start)
      for (let i = 1; i < dayBlocks.length; i++) {
        if (dayBlocks[i].start - dayBlocks[i - 1].end < 30) return false
      }
    }
  }
  return true
}

/**
 * Generates conflict-free timetable suggestions labeled in plain words (zero numeric scores).
 * Returns top 3–5 primary cards plus additional valid combinations revealed by "Show more suggestions".
 */
export function buildSuggestions(
  courses: PCourse[],
  state: PlannerState,
  settings: Settings,
  setup: QuickSetup,
  favTeachers: string[] = [],
): { top: SuggestionCard[]; more: SuggestionCard[]; unsolvableReason: string | null; partialNotice: string | null; prefNotice: string | null } {
  const sc = solverCourses(courses, state, false)
  if (sc.length === 0) return { top: [], more: [], unsolvableReason: null, partialNotice: null, prefNotice: null }

  const hasFreePref = activeFreeDaysOf(setup).length > 0
  const variants: { freeDay?: number; startPreference?: 'early' | 'late' | 'evening' }[] = [
    {},
    ...(!hasFreePref ? [{ freeDay: 4 }, { freeDay: 0 }, { freeDay: 2 }] : []),
    { startPreference: 'early' },
    { startPreference: 'late' },
    { startPreference: 'evening' },
    { freeDay: 1 },
    { freeDay: 3 },
  ]

  const seen = new Set<string>()
  const cards: SuggestionCard[] = []
  let firstError: string | null = null

  const runVariants = (
    input: ReturnType<typeof solverCourses>,
    usesWaitlist: boolean,
    skippedCodes?: string[],
    limit = 12,
    strictCheck = false,
  ) => {
    if (input.length === 0) return
    for (const v of variants) {
      const res = solve(input, buildSearchSettings(settings, setup, favTeachers, courses, { ...v, joinWaitlist: usesWaitlist }))
      if (res.schedules.length === 0 && !firstError && !usesWaitlist && !skippedCodes?.length) {
        const blocked = res.diagnostics.find((d) => d.kept === 0)
        firstError = blocked
          ? `No conflict-free combination fits ${blocked.code} right now.`
          : 'No conflict-free combination fits current options.'
      }
      for (const sched of res.schedules) {
        if (strictCheck && !matchesStrictSetup(sched, setup)) continue
        const sig = `${skippedCodes?.join(',') ?? ''}:${scheduleSignature(sched)}`
        if (seen.has(sig)) continue
        seen.add(sig)
        const baseDesc = describeSchedule(sched)
        const hasWlPick = usesWaitlist && sched.picks.some((p) => p.option.status.toLowerCase().includes('wait'))
        const label = skippedCodes?.length
          ? `${sc.length - skippedCodes.length} of ${sc.length} fit`
          : hasWlPick
            ? `${baseDesc.label} · Waitlist`
            : baseDesc.label
        const detail = skippedCodes?.length
          ? `Removes ${skippedCodes.join(', ')}${hasWlPick ? ' · Waitlist' : ''}`
          : baseDesc.detail
        cards.push({ id: sig, label, detail, schedule: sched, usesWaitlist: hasWlPick, skippedCodes })
        if (cards.length >= limit) return
      }
      if (cards.length >= limit) return
    }
  }

  const strictRulesActive =
    !!setup.strict &&
    (activeFreeDaysOf(setup).length > 0 || !!setup.noEarly8am || !!setup.endBefore5pm || setup.pacing === 'gapped')
  let prefNotice: string | null = null

  // 0. If "Must match" strict preferences are turned on, try strict Open-only full schedules first
  if (strictRulesActive) {
    runVariants(sc, false, undefined, 12, true)
    if (cards.length === 0) {
      prefNotice = 'No full schedule satisfied every strict preference together — showing the closest flexible suggestions instead.'
    }
  }

  // 1. Soft-scored Open-only full schedule
  if (cards.length === 0) {
    runVariants(sc, false)
  }

  // 2. Waitlist fallback for full schedule if 0 Open-only combinations exist
  if (cards.length === 0) {
    runVariants(sc, true, undefined, 8)
  }

  const fullCardsCount = cards.length

  // 3. Best-fit partial fallback (omitting 1 non-locked course at a time) when full courses still collide
  if (cards.length === 0 && sc.length > 1) {
    const activeNonLocked = courses
      .filter((c) => !state.removed.includes(c.code) && !(c.held && !state.unlocked.includes(c.code)))
      .sort((a, b) => Number(!!a.held) - Number(!!b.held))
    for (const skip of activeNonLocked) {
      const subOpen = sc.filter((x) => x.code !== skip.code)
      runVariants(subOpen, false, [skip.code], 8)
      if (cards.length >= 6) break
    }
    if (cards.length === 0) {
      for (const skip of activeNonLocked) {
        const subWl = sc.filter((x) => x.code !== skip.code)
        runVariants(subWl, true, [skip.code], 6)
        if (cards.length >= 6) break
      }
    }
  }

  // Deduplicate identical headlines (e.g. two "Thu free" cards -> "Thu free · Daytime" vs "Thu free · Evening")
  const counts = new Map<string, number>()
  for (const c of cards) counts.set(c.label, (counts.get(c.label) ?? 0) + 1)
  const usedLabels = new Set<string>()
  for (const c of cards) {
    if ((counts.get(c.label) ?? 0) <= 1) continue
    const blocks = c.schedule.blocks
    const earliest = blocks.length ? Math.min(...blocks.map((b) => b.start)) : 9 * 60
    const latest = blocks.length ? Math.max(...blocks.map((b) => b.end)) : 12 * 60
    const tag = latest >= 19 * 60 ? 'Evening' : earliest >= 10 * 60 ? 'Late start' : latest <= 14 * 60 ? 'Morning' : 'Daytime'
    let candidate = `${c.label} · ${tag}`
    if (usedLabels.has(candidate)) candidate = `${c.label} · Alt`
    usedLabels.add(candidate)
    c.label = candidate
  }

  // Preserve the user's untouched registered schedule when all held courses fit without time overlap
  const activeAll = courses.filter((c) => !state.removed.includes(c.code))
  const heldAll = activeAll.filter((c) => !!c.held)
  if (heldAll.length > 0 && activeAll.length === sc.length) {
    const origChoices: Record<string, string> = { ...state.choice }
    for (const c of activeAll) {
      const opt = c.held ?? effective(c, state)
      if (opt) origChoices[c.code] = opt.id
    }
    const origState: PlannerState = { ...state, choice: origChoices }
    const allHaveOpt = activeAll.every((c) => !!(c.held ?? effective(c, origState)))
    if (allHaveOpt && overlapsOf(blocksOf(activeAll, origState)).length === 0) {
      const origSched = asSchedule(activeAll, origState)
      const origSig = `:${scheduleSignature(origSched)}`
      const existingIdx = cards.findIndex((c) => c.id === origSig)
      const isOnlyFullFit = fullCardsCount === 0 || (fullCardsCount === 1 && existingIdx >= 0)
      if (existingIdx >= 0) {
        const match = cards.splice(existingIdx, 1)[0]
        match.isOriginal = true
        match.label = 'Original · Untouched'
        match.detail = isOnlyFullFit ? 'Only fit · keeps all registered' : 'Registered · no changes'
        cards.unshift(match)
      } else if (fullCardsCount === 0) {
        cards.unshift({
          id: origSig,
          label: 'Original · Untouched',
          detail: 'Only fit · keeps all registered',
          schedule: origSched,
          isOriginal: true,
        })
      }
    }
  }

  let partialNotice: string | null = null
  if (cards.length > 0 && fullCardsCount === 0 && !cards.some((c) => c.isOriginal)) {
    const activeCourses = courses.filter((c) => !state.removed.includes(c.code))
    const activeOverlaps = overlapsOf(blocksOf(activeCourses, state))
    const pairs = [...new Set(activeOverlaps.map((o) => `${o.a.courseCode} × ${o.b.courseCode}`))].slice(0, 2)
    const skippedList = [...new Set(cards.flatMap((c) => c.skippedCodes ?? []))]
    partialNotice = `No conflict-free suggestion fits all ${sc.length} courses together${pairs.length ? ` (${pairs.join(', ')})` : ''} — applying a suggestion below will remove ${skippedList.join(' / ')} from your schedule.`
  }

  return {
    top: cards.slice(0, 4),
    more: cards.slice(4),
    unsolvableReason: cards.length === 0 ? firstError : null,
    partialNotice,
    prefNotice: cards.length > 0 && fullCardsCount > 0 ? prefNotice : null,
  }
}

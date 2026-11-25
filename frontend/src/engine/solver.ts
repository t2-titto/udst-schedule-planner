import {
  BASE_DAYS,
  NEUTRAL,
  instructorKey,
  type ClassOption,
  type Component,
  type Course,
  type Meeting,
  type Settings,
  type TimingSettings,
} from './types'

// ------------------------------------------------------------------ scoring

/**
 * Timing score for one minute of the day. 10 inside the ideal window; outside it falls linearly to 5
 * over `fade` minutes and stays at 5. Never below 5: an unpreferred hour is neutral, not punished.
 */
export function timeScore(minute: number, t: TimingSettings, _day?: number): number {
  const start = Math.min(t.start, t.end)
  const end = Math.max(t.start, t.end)
  const x = minute + 0.5
  const d = x < start ? start - x : x > end ? x - end : 0
  if (d === 0) return 10
  if (t.fade <= 0) return NEUTRAL
  return NEUTRAL + (10 - NEUTRAL) * Math.max(0, 1 - d / t.fade)
}

/** Sum of the per-minute scores over a meeting, so a 3-hour class weighs three times a 1-hour one. */
export function meetingTimingSum(m: Meeting, t: TimingSettings): number {
  let sum = 0
  for (let min = m.start; min < m.end; min++) sum += timeScore(min, t, m.day)
  return sum
}

/**
 * Instructor score of one option: the LOWEST score among every instructor of every component
 * (lecture, lab, co-teachers). Scores are per subject: the same person in two subjects has two scores.
 */
export function optionInstructorScore(
  course: Pick<Course, 'code'>,
  option: ClassOption,
  scores: Record<string, number>,
  skipOnline = false,
): { score: number; limiting: string[]; counted: boolean } {
  const names = [...new Set(option.components.filter((c) => !(skipOnline && isOnlineRoom(c.room))).flatMap((c) => c.instructors))]
  let min = Infinity
  for (const n of names) min = Math.min(min, scores[instructorKey(course.code, n)] ?? NEUTRAL)
  // No scored instructor (a fully online option): it says nothing about this course, so it must not drag the mean.
  if (!Number.isFinite(min)) return { score: NEUTRAL, limiting: [], counted: false }
  const limiting = names.filter((n) => (scores[instructorKey(course.code, n)] ?? NEUTRAL) === min)
  return { score: min, limiting, counted: true }
}

/** "Online", "Online2", ...: the room name PeopleSoft gives a class with no place to go. */
export const isOnlineRoom = (room: string): boolean => /^online/i.test(room.trim())

type M = Meeting & { online?: boolean }

/** Sum of the per-day scores of the days that are completely empty, capped at 10. */
export function freeDayComponent(freeDays: number[], dayScores: number[]): number {
  return Math.min(10, freeDays.reduce((s, d) => s + (dayScores[d] ?? NEUTRAL), 0))
}

export function weightedTotal(
  parts: { instructor: number; timing: number; freeDay: number },
  w: { instructor: number; timing: number; freeDay: number },
): number {
  const sum = w.instructor + w.timing + w.freeDay
  if (sum <= 0) return (parts.instructor + parts.timing + parts.freeDay) / 3
  return (parts.instructor * w.instructor + parts.timing * w.timing + parts.freeDay * w.freeDay) / sum
}

// ------------------------------------------------------------------ rules

export const isFull = (o: ClassOption): boolean =>
  o.status.toLowerCase() !== 'open' || o.components.some((c) => c.seatsOpen === 0)

/** A class option the student could join the waitlist of: PeopleSoft says Wait List and every component has a place left. */
export const isJoinable = (o: ClassOption): boolean => !!o.waitlist && o.components.every((c) => c.waitPlaces == null || c.waitPlaces > 0) && o.components.some((c) => (c.waitPlaces ?? 0) > 0)

export const conflicts = (a: Meeting, b: Meeting, gap: number): boolean =>
  a.day === b.day && a.start < b.end + gap && b.start < a.end + gap

const flatMeetings = (o: ClassOption, markOnline = false): M[] =>
  o.components.flatMap((c) => c.meetings.map((m) => ({ ...m, online: markOnline && isOnlineRoom(c.room) })))

/** Online classes need no travel time, so against them only a real overlap counts (gap 0). */
const gapFor = (a: M, b: M, gap: number) => (a.online || b.online ? 0 : gap)

function selfConflict(ms: M[], gap: number): boolean {
  for (let i = 0; i < ms.length; i++) for (let j = i + 1; j < ms.length; j++) if (conflicts(ms[i], ms[j], gapFor(ms[i], ms[j], gap))) return true
  return false
}

/** Fri and Sat are shown and scored only when some selected course actually meets on them. */
export function activeDays(courses: Course[]): number[] {
  const days = new Set(BASE_DAYS)
  for (const c of courses) for (const o of c.options) for (const m of flatMeetings(o)) days.add(m.day)
  return [...days].sort((a, b) => a - b)
}

// ------------------------------------------------------------------ solver

export interface Block {
  courseIndex: number
  courseCode: string
  courseTitle: string
  kind: string
  classNbr: string
  section: string
  room: string
  instructors: string[]
  /** An online class that the options say to leave out of scoring (still shown, still checked for overlap). */
  online: boolean
  day: number
  start: number
  end: number
}

export interface Chosen {
  course: Course
  courseIndex: number
  option: ClassOption
  instructorScore: number
  limiting: string[]
}

export interface Schedule {
  picks: Chosen[]
  blocks: Block[]
  freeDays: number[]
  scores: { instructor: number; timing: number; freeDay: number; total: number }
}

export interface CourseDiagnostic {
  code: string
  total: number
  kept: number
  droppedFull: number
  droppedLate: number
  droppedOverlap: number
}

export interface SolveResult {
  schedules: Schedule[]
  /** Complete schedules that satisfy every rule (not just the ones kept in the top list). */
  validCount: number
  /** Cartesian size of the search after filtering single options. */
  searchSpace: number
  branchesOverlap: number
  branchesNoFreeDay: number
  diagnostics: CourseDiagnostic[]
  days: number[]
}

interface Cand {
  course: Course
  courseIndex: number
  option: ClassOption
  meetings: M[]
  instructorCounted: boolean
  instructorScore: number
  limiting: string[]
  timingSum: number
  minutes: number
}

function makeCand(course: Course, courseIndex: number, option: ClassOption, settings: Settings): Cand {
  const meetings = flatMeetings(option, settings.rules.ignoreOnline)
  const scored = meetings.filter((m) => !m.online)
  const ins = optionInstructorScore(course, option, settings.instructorScores, settings.rules.ignoreOnline)
  return {
    course,
    courseIndex,
    option,
    meetings,
    instructorCounted: ins.counted,
    instructorScore: ins.score,
    limiting: ins.limiting,
    timingSum: scored.reduce((s, m) => s + meetingTimingSum(m, settings.timing), 0),
    minutes: scored.reduce((s, m) => s + (m.end - m.start), 0),
  }
}

/** The instructor, timing, free-day and total scores of a set of chosen combinations. */
function aggregate(chosen: Cand[], freeDays: number[], settings: Settings): Schedule['scores'] {
  const counted = chosen.filter((c) => c.instructorCounted)
  const instructor = counted.length ? counted.reduce((s, c) => s + c.instructorScore, 0) / counted.length : NEUTRAL
  const minutes = chosen.reduce((s, c) => s + c.minutes, 0)
  const tSum = chosen.reduce((s, c) => s + c.timingSum, 0)
  const timing = minutes > 0 ? tSum / minutes : NEUTRAL
  const freeDay = freeDayComponent(freeDays, settings.freeDayScores)
  return { instructor, timing, freeDay, total: weightedTotal({ instructor, timing, freeDay }, settings.weights) }
}

/**
 * The score of one given choice of a combination per course, whether or not it satisfies the rules or overlaps itself
 * (the planner shows the score of whatever is on the calendar, and of a drop position before it happens).
 * `days` should be the same week the ranked list uses (see `activeDays`).
 */
export function scoreSelection(picks: { course: Course; option: ClassOption }[], days: number[], settings: Settings): Schedule['scores'] & { freeDays: number[] } {
  const chosen = picks.map((p, i) => makeCand(p.course, i, p.option, settings))
  const used = new Set<number>()
  for (const c of chosen) for (const m of c.meetings) if (!m.online) used.add(m.day)
  const freeDays = days.filter((d) => !used.has(d))
  return { ...aggregate(chosen, freeDays, settings), freeDays }
}

const EPS = 1e-9

export function solve(courses: Course[], settings: Settings): SolveResult {
  const { rules } = settings
  const days = activeDays(courses)
  const diagnostics: CourseDiagnostic[] = []
  const groups: Cand[][] = []

  courses.forEach((course, courseIndex) => {
    const d: CourseDiagnostic = { code: course.code, total: course.options.length, kept: 0, droppedFull: 0, droppedLate: 0, droppedOverlap: 0 }
    const cands: Cand[] = []
    for (const option of course.options) {
      const meetings = flatMeetings(option, rules.ignoreOnline)
      const scored = meetings.filter((m) => !m.online) // online ones too when the option is off: nothing is filtered
      if (rules.excludeFull && isFull(option) && !(rules.joinWaitlist && isJoinable(option))) {
        d.droppedFull++
        continue
      }
      if (rules.latestEnd !== null && !rules.latestEndExempt.includes(course.code) && scored.some((m) => m.end > rules.latestEnd!)) {
        d.droppedLate++
        continue
      }
      if (selfConflict(meetings, rules.minGap)) {
        d.droppedOverlap++
        continue
      }
      cands.push(makeCand(course, courseIndex, option, settings))
    }
    d.kept = cands.length
    diagnostics.push(d)
    groups.push(cands)
  })

  const empty = (): SolveResult => ({
    schedules: [],
    validCount: 0,
    searchSpace: 0,
    branchesOverlap: 0,
    branchesNoFreeDay: 0,
    diagnostics,
    days,
  })
  if (!courses.length || groups.some((g) => g.length === 0)) return empty()

  const searchSpace = groups.reduce((p, g) => p * g.length, 1)
  // Fail-first: the course with the fewest options goes first, so dead branches die early.
  const order = [...groups].sort((a, b) => a.length - b.length)

  const placed: M[][] = Array.from({ length: 7 }, () => [])
  const dayCount = new Array<number>(7).fill(0)
  const chosen: Cand[] = new Array(order.length)
  let occupied = 0
  let validCount = 0
  let branchesOverlap = 0
  let branchesNoFreeDay = 0
  const best: { key: number[]; ids: string; cands: Cand[]; freeDays: number[]; scores: Schedule['scores'] }[] = []

  const better = (a: { key: number[]; ids: string }, b: { key: number[]; ids: string }): boolean => {
    for (let i = 0; i < a.key.length; i++) {
      if (a.key[i] > b.key[i] + EPS) return true
      if (a.key[i] < b.key[i] - EPS) return false
    }
    return a.ids < b.ids
  }

  const leaf = () => {
    validCount++
    const freeDays = days.filter((d) => dayCount[d] === 0)
    const scores = aggregate(chosen, freeDays, settings)
    const { instructor, timing: timingScore, freeDay, total } = scores
    const key = [total, freeDay, timingScore, instructor]
    const ids = [...chosen].sort((a, b) => a.courseIndex - b.courseIndex).map((c) => c.option.id).join('|')
    const entry = { key, ids }
    if (best.length >= settings.topN && !better(entry, best[best.length - 1])) return
    const row = { key, ids, cands: [...chosen], freeDays, scores }
    let at = best.findIndex((b) => better(row, b))
    if (at < 0) at = best.length
    best.splice(at, 0, row)
    if (best.length > settings.topN) best.pop()
  }

  const dfs = (i: number) => {
    if (i === order.length) return leaf()
    for (const cand of order[i]) {
      let ok = true
      outer: for (const m of cand.meetings) for (const b of placed[m.day]) if (conflicts(m, b, gapFor(m, b, rules.minGap))) { ok = false; break outer }
      if (!ok) {
        branchesOverlap++
        continue
      }
      for (const m of cand.meetings) {
        placed[m.day].push(m) // every class takes part in the overlap check
        if (!m.online && dayCount[m.day]++ === 0) occupied++ // a day with only unscored online classes still counts as free
      }
      chosen[i] = cand
      // Days only fill up as courses are added, so if too few days remain free now, no extension can recover.
      if (days.length - occupied >= rules.minFreeDays) dfs(i + 1)
      else branchesNoFreeDay++
      for (const m of cand.meetings) {
        placed[m.day].pop()
        if (!m.online && --dayCount[m.day] === 0) occupied--
      }
    }
  }
  dfs(0)

  const schedules: Schedule[] = best.map((b) => {
    const picks: Chosen[] = [...b.cands]
      .sort((x, y) => x.courseIndex - y.courseIndex)
      .map((c) => ({ course: c.course, courseIndex: c.courseIndex, option: c.option, instructorScore: c.instructorScore, limiting: c.limiting }))
    const blocks: Block[] = []
    for (const p of picks)
      for (const comp of p.option.components as Component[])
        for (const m of comp.meetings)
          blocks.push({
            courseIndex: p.courseIndex,
            courseCode: p.course.code,
            courseTitle: p.course.title,
            kind: comp.kind,
            classNbr: comp.classNbr,
            section: comp.section,
            room: comp.room,
            instructors: comp.instructors,
            online: settings.rules.ignoreOnline && isOnlineRoom(comp.room),
            day: m.day,
            start: m.start,
            end: m.end,
          })
    blocks.sort((x, y) => x.day - y.day || x.start - y.start)
    return { picks, blocks, freeDays: b.freeDays, scores: b.scores }
  })

  return { schedules, validCount, searchSpace, branchesOverlap, branchesNoFreeDay, diagnostics, days }
}

/** Plain-language reasons for an empty result, most specific first. */
export function explainEmpty(courses: Course[], result: SolveResult, settings: Settings): string[] {
  if (!courses.length) return ['Select at least one course.']
  const out: string[] = []
  for (const d of result.diagnostics) {
    if (d.kept > 0) continue
    const parts: string[] = []
    if (d.droppedFull) parts.push(`${d.droppedFull} full or closed`)
    if (d.droppedLate) parts.push(`${d.droppedLate} end after the latest allowed time`)
    if (d.droppedOverlap) parts.push(`${d.droppedOverlap} overlap themselves (with the minimum gap)`)
    out.push(
      d.total === 0
        ? `${d.code} has no options in the data.`
        : `${d.code}: all ${d.total} options are excluded (${parts.join(', ')}).`,
    )
  }
  if (out.length) return out
  if (result.validCount === 0) {
    if (settings.rules.minFreeDays > 0 && result.branchesNoFreeDay > 0)
      out.push(`No combination leaves ${settings.rules.minFreeDays} day(s) free without overlapping. Lower "free days required".`)
    if (result.branchesOverlap > 0)
      out.push('The remaining options overlap each other (or sit closer than the minimum gap). Try a smaller gap.')
    if (!out.length) out.push('No combination satisfies all rules.')
  }
  return out
}

import { describe, expect, it } from 'vitest'
import { normKey, normalizeCourses, parseDay, parseTime, splitInstructors } from './normalize'
import { buildIcs, escapeText, firstOnOrAfter, fold, icsFileName, parseDmyRange, termRange } from './ics'
import {
  conflicts,
  explainEmpty,
  isJoinable,
  isOnlineRoom,
  freeDayComponent,
  meetingTimingSum,
  optionInstructorScore,
  activeDays,
  scoreSelection,
  solve,
  timeScore,
  weightedTotal,
} from './solver'
import { DEFAULT_SETTINGS, instructorKey, type ClassOption, type Course, type Settings } from './types'

const t = (h: number, m = 0) => h * 60 + m

function opt(id: string, comps: { kind?: string; who: string | string[]; at: [number, number, number][]; open?: number; room?: string }[], status = 'Open'): ClassOption {
  return {
    id,
    optionNo: id.split('#')[1],
    status,
    components: comps.map((c, i) => ({
      dates: '',
      kind: c.kind ?? (i === 0 ? 'Lecture' : 'Laboratory'),
      classNbr: `${id}-${i}`,
      section: String(i + 1),
      room: c.room ?? 'R',
      instructors: Array.isArray(c.who) ? c.who : [c.who],
      meetings: c.at.map(([day, s, e]) => ({ day, start: s, end: e })),
      seatsOpen: c.open ?? 10,
      seatsTotal: 20,
    })),
  }
}

const settings = (over: Partial<Settings> = {}): Settings => ({
  ...DEFAULT_SETTINGS,
  ...over,
  rules: { ...DEFAULT_SETTINGS.rules, ...(over.rules ?? {}) },
})

describe('timing curve', () => {
  const w = { start: t(8), end: t(12), fade: t(3) }
  it('is 10 inside the window and fades linearly to 5', () => {
    expect(timeScore(t(9), w)).toBe(10)
    expect(timeScore(t(13, 30) - 1, w)).toBeCloseTo(7.5, 1) // 1.5h past the end, halfway through a 3h fade
    expect(timeScore(t(15), w)).toBe(5)
    expect(timeScore(t(21), w)).toBe(5)
    expect(timeScore(t(6, 30) - 1, w)).toBeCloseTo(7.5, 1)
  })
  it('never drops below 5, even far outside', () => {
    for (let m = 0; m < 1440; m += 7) expect(timeScore(m, w)).toBeGreaterThanOrEqual(5)
  })
  it('hard edge with fade 0', () => {
    const hard = { ...w, fade: 0 }
    expect(timeScore(t(12) + 5, hard)).toBe(5)
    expect(timeScore(t(11, 59), hard)).toBe(10)
  })
  it('weights longer classes more (average over class-hours)', () => {
    const inside = meetingTimingSum({ day: 0, start: t(9), end: t(10) }, w) // 1h at 10
    const outside = meetingTimingSum({ day: 0, start: t(18), end: t(21) }, w) // 3h at 5
    expect((inside + outside) / (60 + 180)).toBeCloseTo((10 * 60 + 5 * 180) / 240, 6)
  })
})

describe('instructor score', () => {
  const course = { code: 'INFS 2201' }
  const o = opt('INFS 2201#1', [
    { who: 'Instructor One', at: [[3, t(8), t(10)]] },
    { who: ['Instructor Five', 'Instructor Six'], at: [[1, t(9), t(12)]] },
  ])
  it('defaults every instructor to 5', () => {
    expect(optionInstructorScore(course, o, {}).score).toBe(5)
  })
  it('takes the minimum across lecture, lab and co-instructors', () => {
    const s = {
      [instructorKey('INFS 2201', 'Instructor One')]: 9,
      [instructorKey('INFS 2201', 'Instructor Five')]: 8,
      [instructorKey('INFS 2201', 'Instructor Six')]: 2,
    }
    const r = optionInstructorScore(course, o, s)
    expect(r.score).toBe(2)
    expect(r.limiting).toEqual(['Instructor Six'])
  })
  it('is per subject: a score in another course does not apply', () => {
    const s = { [instructorKey('INFS 3102', 'Instructor One')]: 1 }
    expect(optionInstructorScore(course, o, s).score).toBe(5)
  })
})

describe('parsing', () => {
  it('reads times and days', () => {
    expect(parseTime('9:30AM')).toBe(t(9, 30))
    expect(parseTime('12:00AM')).toBe(0)
    expect(parseTime('12:30PM')).toBe(t(12, 30))
    expect(parseTime('8:00PM')).toBe(t(20))
    expect(parseTime('21:15')).toBe(t(21, 15))
    expect(parseTime('nope')).toBeNull()
    expect(parseDay('Sunday')).toBe(0)
    expect(parseDay('thu')).toBe(4)
    expect(parseDay('Funday')).toBeNull()
  })
  it('splits multiple instructors and falls back to Staff', () => {
    expect(splitInstructors('A B, C D')).toEqual(['A B', 'C D'])
    expect(splitInstructors('A B / C D & E F')).toEqual(['A B', 'C D', 'E F'])
    expect(splitInstructors('A B and C D')).toEqual(['A B', 'C D'])
    expect(splitInstructors('Anand Rao')).toEqual(['Anand Rao'])
    expect(splitInstructors('')).toEqual(['Staff'])
    expect(splitInstructors('TBA')).toEqual(['Staff'])
    expect(splitInstructors(['X', 'x', 'Y'])).toEqual(['X', 'Y'])
  })
  it('normalises the connector answer and reports bad rows', () => {
    const raw = {
      courses: [
        {
          row: 5,
          key: { strm: '1271' },
          sections: {
            course: 'DSAI 2201',
            title: 'Intro',
            options: [
              {
                option_no: '1',
                status: 'Open',
                components: [
                  { kind: 'Lecture', class_nbr: '3496', section: '1', room: 'Online2', instructor: 'M. A', seats: { open: 177, total: 200 }, meetings: [{ day: 'Sunday', start: '8:00PM', end: '10:00PM' }] },
                ],
              },
            ],
          },
        },
        { row: 6, error: 'NotFound: nope' },
      ],
    }
    const { courses, warnings } = normalizeCourses(raw)
    expect(courses).toHaveLength(1)
    expect(courses[0].code).toBe('DSAI 2201')
    expect(courses[0].options[0].components[0].meetings[0]).toEqual({ day: 0, start: t(20), end: t(22) })
    expect(courses[0].options[0].components[0].seatsOpen).toBe(177)
    expect(warnings.some((w) => w.includes('Row 6'))).toBe(true)
  })
})

describe('course keys', () => {
  const good = { strm: '1271', crse_id: '005947', crse_offer_nbr: '1', acad_career: 'CRED', institution: 'CNAQA' }
  it('keeps a well-formed key and rejects anything that could shape a request', () => {
    expect(normKey(good)).toEqual(good)
    expect(normKey({ ...good, strm: '1271;x' })).toBeUndefined()
    expect(normKey({ ...good, institution: '' })).toBeUndefined()
    expect(normKey({ ...good, crse_id: 'A'.repeat(13) })).toBeUndefined()
    expect(normKey({ strm: '1' })).toBeUndefined()
    expect(normKey('1271')).toBeUndefined()
  })
  it('carries the key and the row numbers through a fetch answer', () => {
    const { courses } = normalizeCourses({ courses: [{ row: 5, key: good, sections: { course: 'DSAI 2201', options: [{ row: 3, option_no: '4', components: [{ kind: 'Lecture', class_nbr: '1', meetings: [] }] }] } }] })
    expect(courses[0].key).toEqual(good)
    expect(courses[0].row).toBe(5)
    expect(courses[0].options[0].optionRow).toBe(3)
  })
})

describe('rules and free days', () => {
  it('touching classes only conflict once a gap is required', () => {
    const a = { day: 1, start: t(9), end: t(11) }
    const b = { day: 1, start: t(11), end: t(12) }
    expect(conflicts(a, b, 0)).toBe(false)
    expect(conflicts(a, b, 15)).toBe(true)
    expect(conflicts(a, { ...b, day: 2 }, 15)).toBe(false)
  })
  it('free-day score is the sum of the free days, capped at 10', () => {
    expect(freeDayComponent([4], [0, 0, 0, 0, 7])).toBe(7)
    expect(freeDayComponent([2, 4], [0, 0, 7, 0, 6])).toBe(10)
    expect(freeDayComponent([], [5, 5, 5, 5, 5])).toBe(0)
  })
  it('weighted total follows the weights, equal when all are zero', () => {
    expect(weightedTotal({ instructor: 10, timing: 0, freeDay: 0 }, { instructor: 1, timing: 0, freeDay: 0 })).toBe(10)
    expect(weightedTotal({ instructor: 9, timing: 6, freeDay: 3 }, { instructor: 0, timing: 0, freeDay: 0 })).toBe(6)
  })
})

// Three small courses. A is fixed on Sun evening; B and C have a morning and an afternoon style option.
const A: Course = {
  code: 'AAA 100',
  title: 'A',
  term: 'T',
  options: [opt('AAA 100#1', [{ who: 'Ann', at: [[0, t(20), t(22)]] }, { who: 'Ann', at: [[1, t(9, 30), t(11)], [3, t(9, 30), t(11)]] }]), opt('AAA 100#2', [{ who: 'Ann', at: [[0, t(20), t(22)]] }, { who: 'Bo', at: [[2, t(14), t(15, 30)], [4, t(14), t(15, 30)]] }])],
}
const B: Course = {
  code: 'BBB 200',
  title: 'B',
  term: 'T',
  options: [
    opt('BBB 200#1', [{ who: 'Cy', at: [[1, t(8), t(9, 30)], [3, t(8), t(9, 30)]] }]),
    opt('BBB 200#2', [{ who: 'Di', at: [[2, t(9), t(10, 30)], [4, t(9), t(10, 30)]] }]),
    opt('BBB 200#3', [{ who: 'Cy', at: [[1, t(10), t(11, 30)], [3, t(10), t(11, 30)]] }], 'Closed'),
  ],
}
const C: Course = {
  code: 'CCC 300',
  title: 'C',
  term: 'T',
  options: [
    opt('CCC 300#1', [{ who: 'Ed', at: [[0, t(9), t(12)]] }, { who: ['Fa', 'Gil'], at: [[2, t(12), t(14)]] }]),
    opt('CCC 300#2', [{ who: 'Ed', at: [[4, t(16), t(19)]] }], 'Open'),
  ],
}

function brute(courses: Course[], s: Settings) {
  const days = [0, 1, 2, 3, 4]
  let valid = 0
  let best = -Infinity
  const rec = (i: number, picks: ClassOption[]) => {
    if (i === courses.length) {
      const ms = picks.flatMap((o) => o.components.flatMap((c) => c.meetings))
      for (let a = 0; a < ms.length; a++) for (let b = a + 1; b < ms.length; b++) if (ms[a].day === ms[b].day && ms[a].start < ms[b].end + s.rules.minGap && ms[b].start < ms[a].end + s.rules.minGap) return
      const free = days.filter((d) => !ms.some((m) => m.day === d))
      if (free.length < s.rules.minFreeDays) return
      valid++
      const ins = picks.map((o, k) => Math.min(...[...new Set(o.components.flatMap((c) => c.instructors))].map((n) => s.instructorScores[instructorKey(courses[k].code, n)] ?? 5)))
      const I = ins.reduce((x, y) => x + y, 0) / ins.length
      let num = 0
      let den = 0
      for (const m of ms) for (let min = m.start; min < m.end; min++) { num += timeScore(min, s.timing); den++ }
      const T = den ? num / den : 5
      const F = Math.min(10, free.reduce((x, d) => x + s.freeDayScores[d], 0))
      best = Math.max(best, weightedTotal({ instructor: I, timing: T, freeDay: F }, s.weights))
      return
    }
    for (const o of courses[i].options) {
      if (s.rules.excludeFull && !(s.rules.joinWaitlist && isJoinable(o)) && (o.status !== 'Open' || o.components.some((c) => c.seatsOpen === 0))) continue
      rec(i + 1, [...picks, o])
    }
  }
  rec(0, [])
  return { valid, best }
}

describe('solver', () => {
  const courses = [A, B, C]
  it('matches an independent brute force on count and best score', () => {
    const s = settings({
      instructorScores: { [instructorKey('AAA 100', 'Bo')]: 9, [instructorKey('BBB 200', 'Cy')]: 3, [instructorKey('CCC 300', 'Gil')]: 7 },
      freeDayScores: [1, 4, 8, 2, 9, 5, 5],
      weights: { instructor: 3, timing: 6, freeDay: 8 },
    })
    const r = solve(courses, s)
    const b = brute(courses, s)
    expect(r.validCount).toBe(b.valid)
    expect(r.schedules[0].scores.total).toBeCloseTo(b.best, 9)
  })
  it('never returns overlapping schedules and returns them best-first', () => {
    const r = solve(courses, settings({ rules: { ...DEFAULT_SETTINGS.rules, minFreeDays: 0 } }))
    for (const sc of r.schedules) {
      for (let i = 0; i < sc.blocks.length; i++)
        for (let j = i + 1; j < sc.blocks.length; j++) expect(conflicts(sc.blocks[i], sc.blocks[j], 0)).toBe(false)
    }
    for (let i = 1; i < r.schedules.length; i++) expect(r.schedules[i - 1].scores.total).toBeGreaterThanOrEqual(r.schedules[i].scores.total - 1e-9)
  })
  it('excludes closed options by default and includes them when allowed', () => {
    const a = solve(courses, settings({ rules: { ...DEFAULT_SETTINGS.rules, minFreeDays: 0 } }))
    expect(a.schedules.every((s) => s.picks.every((p) => p.option.id !== 'BBB 200#3'))).toBe(true)
    const b = solve(courses, settings({ rules: { ...DEFAULT_SETTINGS.rules, minFreeDays: 0, excludeFull: false } }))
    expect(b.validCount).toBeGreaterThan(a.validCount)
  })
  it('latest-end rule can exempt a course', () => {
    const rules = { ...DEFAULT_SETTINGS.rules, minFreeDays: 0, latestEnd: t(18) }
    const none = solve(courses, settings({ rules }))
    expect(none.validCount).toBe(0) // AAA is only on Sunday 8 PM
    expect(explainEmpty(courses, none, settings({ rules })).join(' ')).toContain('AAA 100')
    const ok = solve(courses, settings({ rules: { ...rules, latestEndExempt: ['AAA 100'] } }))
    expect(ok.validCount).toBeGreaterThan(0)
    expect(ok.schedules.every((s) => s.picks.find((p) => p.course.code === 'CCC 300')!.option.id !== 'CCC 300#2')).toBe(true) // ends 7 PM
  })
  it('requires the chosen number of free days', () => {
    const r = solve(courses, settings({ rules: { ...DEFAULT_SETTINGS.rules, minFreeDays: 2 } }))
    for (const s of r.schedules) expect(s.freeDays.length).toBeGreaterThanOrEqual(2)
  })
  it('reports a clear reason when a course has no options left', () => {
    const only = [{ ...B, options: [B.options[2]] }]
    const r = solve(only, settings())
    expect(r.schedules).toHaveLength(0)
    expect(explainEmpty(only, r, settings())[0]).toContain('BBB 200')
  })
  it('prefers the free day with the higher score', () => {
    // C#2 frees Mon..Wed. With Tuesday worth 10 and everything else 0, the best schedule must free Tuesday.
    const s = settings({ freeDayScores: [0, 0, 10, 0, 0, 0, 0], weights: { instructor: 0, timing: 0, freeDay: 1 } })
    const r = solve(courses, s)
    expect(r.schedules[0].freeDays).toContain(2)
  })
})

describe('online classes', () => {
  const on = (o: Partial<Settings['rules']> = {}) => settings({ rules: { ...DEFAULT_SETTINGS.rules, minFreeDays: 0, ignoreOnline: true, ...o } })
  const off = (o: Partial<Settings['rules']> = {}) => settings({ rules: { ...DEFAULT_SETTINGS.rules, minFreeDays: 0, ignoreOnline: false, ...o } })
  const course = (code: string, ...options: ClassOption[]): Course => ({ code, title: code, term: 'T', options })
  const rate = (code: string, who: string, v: number) => ({ [instructorKey(code, who)]: v })

  it('leaves an online instructor out of the lowest-instructor rule', () => {
    const o = opt('X 1#1', [{ who: 'Good', at: [[1, t(9), t(11)]] }, { who: 'Bad', at: [[3, t(21), t(22)]], room: 'Online2' }])
    const scores = { ...rate('X 1', 'Good', 9), ...rate('X 1', 'Bad', 1) }
    expect(optionInstructorScore({ code: 'X 1' }, o, scores, false).score).toBe(1)
    const r = optionInstructorScore({ code: 'X 1' }, o, scores, true)
    expect(r.score).toBe(9)
    expect(r.limiting).toEqual(['Good'])
  })

  it('a fully online option does not drag the instructor mean toward 5', () => {
    const a = course('A 1', opt('A 1#1', [{ who: 'Ann', at: [[1, t(9), t(11)]] }]))
    const b = course('B 1', opt('B 1#1', [{ who: 'Zed', at: [[2, t(9), t(11)]], room: 'Online' }]))
    const s = { ...on(), instructorScores: { ...rate('A 1', 'Ann', 10), ...rate('B 1', 'Zed', 0) } }
    expect(solve([a, b], s).schedules[0].scores.instructor).toBe(10)
    expect(solve([a, b], { ...s, rules: off().rules }).schedules[0].scores.instructor).toBe(5)
  })

  it('leaves online time out of the timing score', () => {
    const a = course('A 1', opt('A 1#1', [{ who: 'Ann', at: [[1, t(9), t(11)]] }]))
    const b = course('B 1', opt('B 1#1', [{ who: 'Zed', at: [[2, t(20), t(22)]], room: 'Online2' }]))
    expect(solve([a, b], on()).schedules[0].scores.timing).toBe(10)
    expect(solve([a, b], off()).schedules[0].scores.timing).toBeLessThan(8)
  })

  it('leaves online classes out of the latest-end rule', () => {
    const b = course('B 1', opt('B 1#1', [{ who: 'Zed', at: [[0, t(20), t(22)]], room: 'Online2' }]))
    expect(solve([b], off({ latestEnd: t(18) })).validCount).toBe(0)
    expect(solve([b], on({ latestEnd: t(18) })).validCount).toBe(1)
  })

  it('a day with only online classes still counts as free', () => {
    const a = course('A 1', opt('A 1#1', [{ who: 'Ann', at: [[1, t(9), t(11)]] }]))
    const b = course('B 1', opt('B 1#1', [{ who: 'Zed', at: [[2, t(9), t(11)]], room: 'Online2' }]))
    expect(solve([a, b], on()).schedules[0].freeDays).toContain(2)
    expect(solve([a, b], off()).schedules[0].freeDays).not.toContain(2)
  })

  it('still never overlaps anything, and is still drawn', () => {
    const a = course('A 1', opt('A 1#1', [{ who: 'Ann', at: [[1, t(9), t(11)]] }]))
    const clash = course('B 1', opt('B 1#1', [{ who: 'Zed', at: [[1, t(10), t(12)]], room: 'Online2' }]))
    expect(solve([a, clash], on()).validCount).toBe(0)
    const fine = course('B 1', opt('B 1#1', [{ who: 'Zed', at: [[1, t(12), t(13)]], room: 'Online2' }]))
    const r = solve([a, fine], on())
    expect(r.validCount).toBe(1)
    expect(r.schedules[0].blocks.filter((b) => b.online)).toHaveLength(1)
  })

  it('needs no travel gap to or from an online class, but real classes keep theirs', () => {
    const a = course('A 1', opt('A 1#1', [{ who: 'Ann', at: [[1, t(9), t(11)]] }]))
    const touching = course('B 1', opt('B 1#1', [{ who: 'Zed', at: [[1, t(11), t(12)]], room: 'Online2' }]))
    expect(solve([a, touching], on({ minGap: 30 })).validCount).toBe(1)
    expect(solve([a, touching], off({ minGap: 30 })).validCount).toBe(0)
    const real = course('B 1', opt('B 1#1', [{ who: 'Zed', at: [[1, t(11), t(12)]] }]))
    expect(solve([a, real], on({ minGap: 30 })).validCount).toBe(0)
  })

  it('does nothing while the option is off, and recognises the room names', () => {
    expect(DEFAULT_SETTINGS.rules.ignoreOnline).toBe(false)
    expect(isOnlineRoom('Online2')).toBe(true)
    expect(isOnlineRoom(' online ')).toBe(true)
    expect(isOnlineRoom('10.2.27')).toBe(false)
  })
})

describe('calendar export (.ics)', () => {
  const lecture = (at: [number, number, number][], extra: Partial<{ who: string[]; room: string; open: number }> = {}) => ({ kind: 'Lecture', who: extra.who ?? ['Instructor Two'], at, room: extra.room, open: extra.open })
  const courseA: Course = {
    code: 'DACS 2202',
    title: 'Intro. to Computing Systems, with a comma; and a semicolon',
    term: 'Winter 2027',
    units: '4.00',
    grading: '60 Percent and Letter Grade',
    description: 'Line one.\nLine two, with a comma.',
    options: [opt('DACS 2202#2', [lecture([[1, t(11), t(12, 30)], [3, t(11), t(12, 30)]]), { kind: 'Laboratory', who: ['Instructor Two', 'Instructor Three'], at: [[0, t(11), t(14)]] }])],
  }
  // The real data states the range on the lecture only; the lab inherits it.
  courseA.options[0].components[0].dates = '05/01/2027 - 14/04/2027'
  courseA.options[0].components[0].classNbr = '3380'
  courseA.options[0].components[1].classNbr = '3381'
  const online: Course = { code: 'DSAI 2201', title: 'Data Science', term: 'Winter 2027', options: [opt('DSAI 2201#1', [{ kind: 'Lecture', who: ['Instructor Four'], at: [[0, t(20), t(22)]], room: 'Online2' }])] }
  online.options[0].components[0].dates = '05/01/2027 - 14/04/2027'
  const sched = () => solve([courseA, online], settings({ rules: { ...DEFAULT_SETTINGS.rules, minFreeDays: 0 } })).schedules[0]
  const NOW = new Date('2026-11-16T10:00:00Z')
  const make = (over = {}) => buildIcs(sched(), { termStart: '2027-01-05', termEnd: '2027-04-14', reminderMin: 30, calendarName: 'UDST Winter 2027', now: NOW, ...over })

  it('reads PeopleSoft dates day first', () => {
    expect(parseDmyRange('05/01/2027 - 14/04/2027')).toEqual({ start: '2027-01-05', end: '2027-04-14' })
    expect(parseDmyRange('')).toBeNull()
    expect(parseDmyRange('31/02/2027 - 14/04/2027')).toBeNull()
    expect(parseDmyRange('14/04/2027 - 05/01/2027')).toBeNull()
  })

  it('finds the first class day on or after the start', () => {
    expect(firstOnOrAfter('2027-01-05', 2)).toBe('2027-01-05') // Tue
    expect(firstOnOrAfter('2027-01-05', 0)).toBe('2027-01-10') // Sun
    expect(firstOnOrAfter('2027-01-05', 1)).toBe('2027-01-11') // Mon
  })

  it('takes the term range from the lecture and lets the lab inherit it', () => {
    expect(termRange(sched())).toEqual({ start: '2027-01-05', end: '2027-04-14' })
    const ics = make({ termStart: '1999-01-01', termEnd: '1999-02-01' }) // the typed dates are only a fallback
    expect(ics).not.toContain('1999')
    expect(ics).toContain('UNTIL=20270414T205959Z')
  })

  it('is a well-formed calendar with CRLF lines of at most 75 octets', () => {
    const ics = make()
    expect(ics.startsWith('BEGIN:VCALENDAR\r\n')).toBe(true)
    expect(ics.endsWith('END:VCALENDAR\r\n')).toBe(true)
    expect(ics).toContain('VERSION:2.0')
    expect(ics).toContain('TZID:Asia/Qatar')
    const enc = new TextEncoder()
    for (const line of ics.split('\r\n')) expect(enc.encode(line).length).toBeLessThanOrEqual(75)
    expect(ics.replace(/\r\n/g, '').includes('\n')).toBe(false) // no bare line feeds
    expect((ics.match(/BEGIN:VEVENT/g) ?? []).length).toBe((ics.match(/END:VEVENT/g) ?? []).length)
  })

  it('makes one repeating event per class and time slot, on the right weekdays', () => {
    const ics = make().replace(/\r\n /g, '') // unfold
    expect((ics.match(/BEGIN:VEVENT/g) ?? []).length).toBe(3) // lecture (Mon+Wed), lab (Sun), online lecture (Sun)
    expect(ics).toContain('RRULE:FREQ=WEEKLY;UNTIL=20270414T205959Z;BYDAY=MO,WE')
    // Mon + Wed: the series starts on the earliest of the two, Wednesday 6 Jan 2027 (Monday's first is 11 Jan)
    expect(ics).toContain('DTSTART;TZID=Asia/Qatar:20270106T110000')
    expect(ics).toContain('DTEND;TZID=Asia/Qatar:20270106T123000')
    expect(ics).toContain('BYDAY=SU')
    expect(ics).toContain('DTSTART;TZID=Asia/Qatar:20270110T110000') // first Sunday: the lab
  })

  it('puts the details in each event, escaped', () => {
    const ics = make().replace(/\r\n /g, '')
    expect(ics).toContain('SUMMARY:DACS 2202 Lecture - Intro. to Computing Systems\\, with a comma\\; and a semicolon')
    expect(ics).toContain('LOCATION:Room R\\, UDST')
    expect(ics).toContain('Instructors: Instructor Two\\, Instructor Three')
    expect(ics).toContain('class number 3381')
    expect(ics).toContain('Units: 4.00')
    expect(ics).toContain('Grading: 60 Percent and Letter Grade')
    expect(ics).toContain('Other parts of this course:')
    expect(ics).toContain('Line one.\\nLine two\\, with a comma.')
    expect(ics).toContain('Enrolment option 2 of 1')
    expect(ics).toContain('CATEGORIES:DACS 2202,Lecture')
  })

  it('marks an online class as online', () => {
    const ics = make().replace(/\r\n /g, '')
    expect(ics).toContain('LOCATION:Online')
    expect(ics).toContain('Room: Online2 (online class)')
  })

  it('adds a reminder only when asked', () => {
    expect(make()).toContain('TRIGGER:-PT30M')
    expect(make({ reminderMin: null })).not.toContain('VALARM')
  })

  it('gives events stable, unique ids so a second import updates rather than duplicates', () => {
    const uids = (s: string) => (s.match(/UID:[^\r]+/g) ?? []).map((x) => x.replace(/\s/g, ''))
    const a = uids(make())
    const b = uids(make({ now: new Date('2027-02-01T00:00:00Z') }))
    expect(a).toEqual(b)
    expect(new Set(a).size).toBe(a.length)
  })

  it('folds long lines without splitting a character, escapes text, and names the file', () => {
    const long = 'é'.repeat(100)
    const lines = fold(`X:${long}`)
    expect(lines.length).toBeGreaterThan(1)
    expect(lines.map((l, i) => (i ? l.slice(1) : l)).join('')).toBe(`X:${long}`)
    expect(escapeText('a,b;c\\d\ne')).toBe('a\\,b\\;c\\\\d\\ne')
    expect(icsFileName('Winter 2027')).toBe('schedule-winter-2027.ics')
    expect(icsFileName('')).toBe('schedule-classes.ics')
  })
})

describe('scoreSelection', () => {
  const mk = (code: string, id: string, at: [number, number, number][]): Course => ({ code, title: code, term: 'T', options: [opt(`${code}#${id}`, [{ who: 'X', at }])] })
  const a = mk('AAA 1000', '1', [[1, t(9), t(10)]])
  const b = mk('BBB 1000', '1', [[2, t(14), t(15)]])
  it('matches the ranked list for the same choice', () => {
    const st = settings()
    const top = solve([a, b], st).schedules[0]
    const sc = scoreSelection([{ course: a, option: a.options[0] }, { course: b, option: b.options[0] }], activeDays([a, b]), st)
    expect(sc.total).toBeCloseTo(top.scores.total, 9)
    expect(sc.freeDays).toEqual(top.freeDays)
  })
  it('scores overlapping choices instead of refusing them', () => {
    const c = mk('CCC 1000', '1', [[1, t(9), t(10)]])
    const sc = scoreSelection([{ course: a, option: a.options[0] }, { course: c, option: c.options[0] }], activeDays([a, c]), settings())
    expect(Number.isFinite(sc.total)).toBe(true)
  })
})

describe('waitlist options (unverified)', () => {
  const sections = (status: string, text: string) => normalizeCourses({ courses: [{ row: 1, key: { strm: '1' }, sections: { course: 'W 1000', title: 't', options: [{ option_no: '1', status, components: [{ kind: 'Lecture', class_nbr: '1', section: '1', room: 'R', instructor: 'A', seats: null, seats_text: text, meetings: [{ day: 'Monday', start: '9:00AM', end: '10:00AM' }] }] }] } }] }).courses[0].options[0]
  it('reads PeopleSoft\'s wording: status Wait List and "Waitlist Available Places 3 of 10"', () => {
    const o = sections('Wait List', 'Waitlist Available Places 3 of 10')
    expect(o.waitlist).toBe(true)
    expect(o.components[0].waitPlaces).toBe(3)
    expect(isJoinable(o)).toBe(true)
  })
  it('an open class is not a waitlist; a waitlist with no place, or no figure, cannot be joined', () => {
    expect(sections('Open', 'Open Seats 5 of 20').waitlist).toBeUndefined()
    expect(isJoinable(sections('Wait List', 'Waitlist Available Places 0 of 10'))).toBe(false)
    expect(isJoinable(sections('Wait List', 'Closed'))).toBe(false)
  })
})

import { fmtRange } from './normalize'
import type { Chosen, Schedule } from './solver'
import { DAY_SHORT, type Component, type Meeting } from './types'

/**
 * A calendar file Google Calendar (and Apple, Outlook) can import: one weekly repeating event per class and
 * time slot, in Qatar time, with the details in the description. Pure functions, no browser APIs.
 */

const TZ = 'Asia/Qatar' // UTC+3 all year, no daylight saving
const BYDAY = ['SU', 'MO', 'TU', 'WE', 'TH', 'FR', 'SA']
const pad = (n: number, w = 2) => String(n).padStart(w, '0')

// ------------------------------------------------------------------ dates

/** "05/01/2027 - 14/04/2027" (day first, as PeopleSoft writes it) -> ISO dates, or null if it cannot be read. */
export function parseDmyRange(s: string): { start: string; end: string } | null {
  const m = /^\s*(\d{1,2})\/(\d{1,2})\/(\d{4})\s*-\s*(\d{1,2})\/(\d{1,2})\/(\d{4})\s*$/.exec(s)
  if (!m) return null
  const iso = (d: string, mo: string, y: string) => {
    const dd = Number(d)
    const mm = Number(mo)
    if (mm < 1 || mm > 12 || dd < 1 || dd > 31) return null
    const t = new Date(Date.UTC(Number(y), mm - 1, dd))
    return t.getUTCMonth() === mm - 1 && t.getUTCDate() === dd ? `${y}-${pad(mm)}-${pad(dd)}` : null
  }
  const start = iso(m[1], m[2], m[3])
  const end = iso(m[4], m[5], m[6])
  return start && end && start <= end ? { start, end } : null
}

/** The term range found in the chosen classes: the earliest start and the latest end of any component that states one. */
export function termRange(schedule: Schedule): { start?: string; end?: string } {
  let start: string | undefined
  let end: string | undefined
  for (const p of schedule.picks)
    for (const c of p.option.components) {
      const r = parseDmyRange(c.dates)
      if (!r) continue
      if (!start || r.start < start) start = r.start
      if (!end || r.end > end) end = r.end
    }
  return { start, end }
}

const weekday = (iso: string) => new Date(`${iso}T00:00:00Z`).getUTCDay()

function addDays(iso: string, n: number): string {
  const d = new Date(`${iso}T00:00:00Z`)
  d.setUTCDate(d.getUTCDate() + n)
  return `${d.getUTCFullYear()}-${pad(d.getUTCMonth() + 1)}-${pad(d.getUTCDate())}`
}

/** First date on or after `from` that falls on weekday `day` (0 = Sunday). */
export function firstOnOrAfter(from: string, day: number): string {
  return addDays(from, (day - weekday(from) + 7) % 7)
}

const compact = (iso: string) => iso.replaceAll('-', '')
const local = (iso: string, minutes: number) => `${compact(iso)}T${pad(Math.floor(minutes / 60))}${pad(minutes % 60)}00`

/** The last day at 23:59:59 Qatar time, as UTC (a repeat rule's UNTIL must be UTC when the start has a time zone). */
const untilUtc = (iso: string) => `${compact(iso)}T205959Z`

const prettyDate = (iso: string) => {
  const d = new Date(`${iso}T00:00:00Z`)
  return `${['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'][d.getUTCDay()]} ${d.getUTCDate()} ${['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'][d.getUTCMonth()]} ${d.getUTCFullYear()}`
}
export { prettyDate }

// ------------------------------------------------------------------ text rules (RFC 5545)

export const escapeText = (s: string): string => s.replace(/\\/g, '\\\\').replace(/;/g, '\\;').replace(/,/g, '\\,').replace(/\r\n|\r|\n/g, '\\n')

const enc = new TextEncoder()

/** Fold a content line to at most 75 octets per physical line, never splitting a multi-byte character. */
export function fold(line: string): string[] {
  if (enc.encode(line).length <= 75) return [line]
  const out: string[] = []
  let cur = ''
  let bytes = 0
  let limit = 75
  for (const ch of line) {
    const n = enc.encode(ch).length
    if (bytes + n > limit) {
      out.push(cur)
      cur = ' ' // a folded line starts with one space, which counts toward its 75
      bytes = 1
      limit = 75
    }
    cur += ch
    bytes += n
  }
  out.push(cur)
  return out
}

// ------------------------------------------------------------------ the calendar

export interface IcsOptions {
  /** ISO dates. Used for any class that does not state its own range. */
  termStart: string
  termEnd: string
  /** Minutes before each class for a pop-up reminder, or null for none. */
  reminderMin: number | null
  calendarName: string
  /** When the file was made (UTC), for DTSTAMP and the footer. Injectable for tests. */
  now?: Date
}

interface Slot {
  pick: Chosen
  comp: Component
  start: number
  end: number
  days: number[]
}

/** One slot per (component, start, end): Mon and Wed at the same time become one event that repeats on both. */
function slotsOf(pick: Chosen): Slot[] {
  const out: Slot[] = []
  for (const comp of pick.option.components) {
    const by = new Map<string, Meeting[]>()
    for (const m of comp.meetings) by.set(`${m.start}-${m.end}`, [...(by.get(`${m.start}-${m.end}`) ?? []), m])
    for (const ms of by.values()) out.push({ pick, comp, start: ms[0].start, end: ms[0].end, days: [...new Set(ms.map((m) => m.day))].sort((a, b) => a - b) })
  }
  return out
}

const dayNames = (days: number[]) => days.map((d) => DAY_SHORT[d]).join(', ')

function describe(sl: Slot, range: { start: string; end: string }, now: Date): string {
  const { pick, comp } = sl
  const c = pick.course
  const lines: string[] = []
  lines.push(`${c.code}: ${c.title}`)
  lines.push('')
  lines.push(`This class: ${comp.kind}${comp.section ? `, section ${comp.section}` : ''}${comp.classNbr ? `, class number ${comp.classNbr}` : ''}`)
  lines.push(`When: ${dayNames(sl.days)} ${fmtRange(sl.start, sl.end)}, every week`)
  lines.push(`Dates: ${prettyDate(range.start)} to ${prettyDate(range.end)}`)
  lines.push(comp.room ? (/^online/i.test(comp.room) ? `Room: ${comp.room} (online class)` : `Room: ${comp.room}`) : 'Room: not listed')
  lines.push(`Instructor${comp.instructors.length === 1 ? '' : 's'}: ${comp.instructors.join(', ')}`)
  if (comp.seatsOpen !== null) lines.push(`Seats when exported: ${comp.seatsOpen}${comp.seatsTotal ? ` of ${comp.seatsTotal}` : ''} open (a snapshot; it may have changed)`)
  lines.push('')
  const nums = pick.option.components.map((x) => x.classNbr).filter(Boolean)
  lines.push(`Term: ${c.term || 'not listed'}`)
  lines.push(`Enrolment option ${pick.option.optionNo} of ${c.options.length}${nums.length ? `, class numbers ${nums.join(' and ')}` : ''}`)
  const meta = [c.units ? `Units: ${c.units}` : '', c.grading ? `Grading: ${c.grading}` : ''].filter(Boolean)
  if (meta.length) lines.push(meta.join('. '))
  const others = pick.option.components.filter((x) => x !== comp)
  if (others.length) {
    lines.push('')
    lines.push('Other parts of this course:')
    for (const o of others) {
      const when = o.meetings.length ? ` ${[...new Set(o.meetings.map((m) => DAY_SHORT[m.day]))].join(', ')} ${fmtRange(o.meetings[0].start, o.meetings[0].end)}` : ''
      lines.push(`- ${o.kind}${o.classNbr ? ` (class ${o.classNbr})` : ''}:${when}${o.room ? `, room ${o.room}` : ''}, ${o.instructors.join(', ')}`)
    }
  }
  if (c.description) {
    lines.push('')
    lines.push(c.description)
  }
  lines.push('')
  lines.push(`Made by Schedule planner on ${now.toISOString().slice(0, 10)}. Times are Qatar time (Asia/Qatar).`)
  return lines.join('\n')
}

const VTIMEZONE = [
  'BEGIN:VTIMEZONE',
  `TZID:${TZ}`,
  'BEGIN:STANDARD',
  'DTSTART:19700101T000000',
  'TZOFFSETFROM:+0300',
  'TZOFFSETTO:+0300',
  'TZNAME:+03',
  'END:STANDARD',
  'END:VTIMEZONE',
]

export function buildIcs(schedule: Schedule, opts: IcsOptions): string {
  const now = opts.now ?? new Date()
  const stamp = `${now.toISOString().replace(/[-:]/g, '').slice(0, 15)}Z`
  const lines: string[] = [
    'BEGIN:VCALENDAR',
    'VERSION:2.0',
    'PRODID:-//Schedule planner//Class timetable//EN',
    'CALSCALE:GREGORIAN',
    'METHOD:PUBLISH',
    `X-WR-CALNAME:${escapeText(opts.calendarName)}`,
    `X-WR-TIMEZONE:${TZ}`,
    `X-WR-CALDESC:${escapeText(`Weekly classes for ${opts.calendarName}, made by Schedule planner.`)}`,
    ...VTIMEZONE,
  ]
  const slots = schedule.picks.flatMap(slotsOf).sort((a, b) => a.days[0] - b.days[0] || a.start - b.start)
  for (const sl of slots) {
    const own = parseDmyRange(sl.comp.dates)
    // A lab that states no dates takes its lecture's; failing that, the dates the person confirmed.
    const sibling = sl.pick.option.components.map((c) => parseDmyRange(c.dates)).find((r) => r !== null) ?? null
    const range = own ?? sibling ?? { start: opts.termStart, end: opts.termEnd }
    const first = sl.days.map((d) => firstOnOrAfter(range.start, d)).sort()[0]
    const summary = `${sl.pick.course.code} ${sl.comp.kind}${sl.pick.course.title ? ` - ${sl.pick.course.title}` : ''}`
    const room = sl.comp.room.trim()
    const location = room ? (/^online/i.test(room) ? 'Online' : `Room ${room}, UDST`) : ''
    // Stable across exports, so importing the same schedule twice updates the events instead of duplicating them.
    const uid = `${sl.pick.course.code.replace(/\s+/g, '')}-${sl.comp.classNbr || sl.comp.kind}-${sl.days.join('')}-${sl.start}-${compact(range.start)}@schedule-planner`
    lines.push(
      'BEGIN:VEVENT',
      `UID:${escapeText(uid)}`,
      `DTSTAMP:${stamp}`,
      `DTSTART;TZID=${TZ}:${local(first, sl.start)}`,
      `DTEND;TZID=${TZ}:${local(first, sl.end)}`,
      `RRULE:FREQ=WEEKLY;UNTIL=${untilUtc(range.end)};BYDAY=${sl.days.map((d) => BYDAY[d]).join(',')}`,
      `SUMMARY:${escapeText(summary)}`,
      ...(location ? [`LOCATION:${escapeText(location)}`] : []),
      `DESCRIPTION:${escapeText(describe(sl, range, now))}`,
      `CATEGORIES:${escapeText(sl.pick.course.code)},${escapeText(sl.comp.kind)}`,
      'STATUS:CONFIRMED',
      'TRANSP:OPAQUE',
    )
    if (opts.reminderMin !== null) {
      lines.push('BEGIN:VALARM', 'ACTION:DISPLAY', `DESCRIPTION:${escapeText(`${summary} starts in ${opts.reminderMin} minutes`)}`, `TRIGGER:-PT${opts.reminderMin}M`, 'END:VALARM')
    }
    lines.push('END:VEVENT')
  }
  lines.push('END:VCALENDAR')
  return lines.flatMap(fold).join('\r\n') + '\r\n'
}

export const icsFileName = (term: string): string => `schedule-${(term || 'classes').toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '')}.ics`

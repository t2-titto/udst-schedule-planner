import type { ClassOption, Component, Course, CourseKey, Meeting } from './types'

type Obj = Record<string, unknown>
const isObj = (v: unknown): v is Obj => typeof v === 'object' && v !== null && !Array.isArray(v)
const str = (v: unknown): string => (typeof v === 'string' ? v.trim() : typeof v === 'number' ? String(v) : '')

const DAY_PREFIX = ['sun', 'mon', 'tue', 'wed', 'thu', 'fri', 'sat']

export function parseDay(v: unknown): number | null {
  if (typeof v === 'number' && Number.isInteger(v) && v >= 0 && v <= 6) return v
  const s = str(v).toLowerCase()
  const i = DAY_PREFIX.indexOf(s.slice(0, 3))
  return i >= 0 ? i : null
}

/** "9:30AM", "9:30 pm", "09:30", "21:00" or a number of minutes. */
export function parseTime(v: unknown): number | null {
  if (typeof v === 'number' && Number.isFinite(v) && v >= 0 && v <= 24 * 60) return Math.round(v)
  const m = /^(\d{1,2}):(\d{2})\s*([ap]m)?$/i.exec(str(v))
  if (!m) return null
  let h = Number(m[1])
  const min = Number(m[2])
  const ap = m[3]?.toLowerCase()
  if (min > 59) return null
  if (ap) {
    if (h < 1 || h > 12) return null
    if (ap === 'pm' && h !== 12) h += 12
    if (ap === 'am' && h === 12) h = 0
  } else if (h > 24) return null
  return h * 60 + min
}

/**
 * A component can have several instructors. Separators: comma, semicolon, slash, ampersand, " and ", new line.
 * An empty or "TBA"-style value becomes "Staff", which is scored like any other instructor.
 */
export function splitInstructors(v: unknown): string[] {
  const raw = Array.isArray(v) ? v.map(str) : str(v).split(/\s*(?:[;,/&\n]|\band\b)\s*/i)
  const seen = new Set<string>()
  const out: string[] = []
  for (const part of raw) {
    const name = part.trim().replace(/\s+/g, ' ')
    if (!name || /^(tba|tbd|to be announced|n\/a|-)$/i.test(name)) continue
    if (!seen.has(name.toLowerCase())) {
      seen.add(name.toLowerCase())
      out.push(name)
    }
  }
  return out.length ? out : ['Staff']
}

function normMeeting(v: unknown, where: string, warnings: string[]): Meeting | null {
  if (!isObj(v)) return null
  const day = parseDay(v.day)
  const start = parseTime(v.start)
  const end = parseTime(v.end)
  if (day === null || start === null || end === null || end <= start) {
    warnings.push(`${where}: skipped a meeting with an unreadable day or time`)
    return null
  }
  return { day, start, end }
}

/** "Waitlist Available Places 3 of 10" (seen in the PeopleSoft catalog) -> 3; null for any other text. */
export function waitPlaces(text: unknown): number | null {
  const m = typeof text === 'string' ? /^\s*waitlist available places\s+(\d+)\s+of\s+\d+\s*$/i.exec(text) : null
  return m ? Number(m[1]) : null
}

function normComponent(v: unknown, where: string, warnings: string[]): Component | null {
  if (!isObj(v)) return null
  const meetings = (Array.isArray(v.meetings) ? v.meetings : [])
    .map((m) => normMeeting(m, where, warnings))
    .filter((m): m is Meeting => m !== null)
  const seats = isObj(v.seats) ? v.seats : null
  const open = seats ? seats.open : v.seatsOpen
  const total = seats ? seats.total : v.seatsTotal
  return {
    dates: str(v.dates),
    kind: str(v.kind) || 'Class',
    classNbr: str(v.class_nbr ?? v.classNbr),
    section: str(v.section),
    room: str(v.room),
    instructors: splitInstructors(v.instructors ?? v.instructor),
    meetings,
    seatsOpen: typeof open === 'number' ? open : null,
    seatsTotal: typeof total === 'number' ? total : null,
    waitPlaces: waitPlaces(v.seats_text),
  }
}

function normOption(v: unknown, code: string, index: number, warnings: string[]): ClassOption | null {
  if (!isObj(v)) return null
  const optionNo = str(v.option_no ?? v.optionNo) || String(index + 1)
  const where = `${code} option ${optionNo}`
  const components = (Array.isArray(v.components) ? v.components : [])
    .map((c) => normComponent(c, where, warnings))
    .filter((c): c is Component => c !== null)
  if (!components.length) {
    warnings.push(`${where}: no components, skipped`)
    return null
  }
  const optionRow = typeof v.row === 'number' ? v.row : undefined
  return { id: `${code}#${optionNo}`, optionRow, optionNo, status: str(v.status) || 'Open', ...(/^wait\s?list$/i.test(str(v.status).trim()) ? { waitlist: true } : {}), components }
}

const KEY_FIELDS = ['strm', 'crse_id', 'crse_offer_nbr', 'acad_career', 'institution'] as const

/** Keep a course key only if every field is a short alphanumeric string, the same rule the server applies. */
export function normKey(v: unknown): CourseKey | undefined {
  if (!isObj(v)) return undefined
  const out: Record<string, string> = {}
  for (const f of KEY_FIELDS) {
    const x = v[f]
    if (typeof x !== 'string' || !/^[A-Za-z0-9]{1,12}$/.test(x)) return undefined
    out[f] = x
  }
  return out as unknown as CourseKey
}

function normCourse(v: unknown, warnings: string[]): Course | null {
  if (!isObj(v)) return null
  // A connector answer nests the course under `sections`; our own format has it at the top level.
  const body = isObj(v.sections) ? v.sections : v
  if (typeof v.error === 'string') {
    warnings.push(`Row ${String(v.row ?? '?')}: ${v.error}`)
    return null
  }
  const code = str(body.code ?? body.course)
  if (!code || !Array.isArray(body.options)) return null
  const options = body.options
    .map((o, i) => normOption(o, code, i, warnings))
    .filter((o): o is ClassOption => o !== null)
  return { units: str(body.units) || undefined, grading: str(body.grading) || undefined, description: str(body.description) || undefined, key: normKey(v.key), row: typeof v.row === 'number' ? v.row : undefined, code, title: str(body.title), term: str(body.term), options }
}

export interface Normalized {
  courses: Course[]
  warnings: string[]
}

/**
 * Accepts: our own JSON (`{courses: [...]}` or an array), the backend's `/api/sections` answer, or one raw
 * connector `sections` object. Anything it cannot read is reported in `warnings`, never silently dropped.
 */
export function normalizeCourses(input: unknown): Normalized {
  const warnings: string[] = []
  let list: unknown[] = []
  if (Array.isArray(input)) list = input
  else if (isObj(input) && Array.isArray(input.courses)) list = input.courses
  else if (isObj(input)) list = [input]
  const courses: Course[] = []
  const seen = new Set<string>()
  for (const item of list) {
    const c = normCourse(item, warnings)
    if (!c) continue
    if (seen.has(c.code)) {
      warnings.push(`${c.code}: listed twice, kept the first`)
      continue
    }
    seen.add(c.code)
    courses.push(c)
  }
  if (!courses.length) warnings.push('No courses with options were found in that data.')
  return { courses, warnings }
}

export const fmtTime = (m: number): string => {
  const h24 = Math.floor(m / 60)
  const min = m % 60
  const h = h24 % 12 === 0 ? 12 : h24 % 12
  return `${h}:${String(min).padStart(2, '0')} ${h24 >= 12 && h24 < 24 ? 'PM' : 'AM'}`
}

/** "8:00 – 11:00 AM" when both ends share AM/PM, otherwise "11:30 AM – 1:00 PM". */
export const fmtCompact = (a: number, b: number): string => {
  const fa = fmtTime(a)
  const fb = fmtTime(b)
  return fa.slice(-2) === fb.slice(-2) ? `${fa.slice(0, -3)} \u2013 ${fb}` : `${fa} \u2013 ${fb}`
}

export const fmtRange = (a: number, b: number): string => `${fmtTime(a)} to ${fmtTime(b)}`

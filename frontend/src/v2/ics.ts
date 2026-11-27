import { effective, type PCourse, type PlannerState } from './model'

const ICS_DAY = ['SU', 'MO', 'TU', 'WE', 'TH', 'FR', 'SA'] as const

function pad2(n: number): string {
  return String(n).padStart(2, '0')
}

function fmtTime(min: number): string {
  return `${pad2(Math.floor(min / 60))}${pad2(min % 60)}00`
}

function escapeIcs(text: string): string {
  return text.replace(/\\/g, '\\\\').replace(/;/g, '\\;').replace(/,/g, '\\,').replace(/\n/g, '\\n')
}

/** Parses DD/MM/YYYY date strings from PeopleSoft's dates range ("05/01/2027 - 14/04/2027"). */
function parseDateRange(dates: string, fallBackYear: number): { startYmd: string; untilYmd: string } {
  const m = /(\d{2})\/(\d{2})\/(\d{4})\s*-\s*(\d{2})\/(\d{2})\/(\d{4})/.exec(dates)
  if (m) {
    return {
      startYmd: `${m[3]}${m[2]}${m[1]}`,
      untilYmd: `${m[6]}${m[5]}${m[4]}T235959Z`,
    }
  }
  return { startYmd: `${fallBackYear}0110`, untilYmd: `${fallBackYear}0425T235959Z` }
}

function firstMatchingDate(baseYmd: string, targetDay: number): string {
  const y = Number(baseYmd.slice(0, 4))
  const m = Number(baseYmd.slice(4, 6)) - 1
  const d = Number(baseYmd.slice(6, 8))
  const dt = new Date(Date.UTC(y, m, d))
  const diff = (targetDay - dt.getUTCDay() + 7) % 7
  dt.setUTCDate(dt.getUTCDate() + diff)
  return `${dt.getUTCFullYear()}${pad2(dt.getUTCMonth() + 1)}${pad2(dt.getUTCDate())}`
}

/** Build an RFC 5545 `.ics` calendar string for all active non-removed schedule courses (Sun–Thu compatible). */
export function buildIcs(courses: PCourse[], state: PlannerState, term: string): string {
  const yearMatch = /\b(20\d{2})\b/.exec(term)
  const fallbackYear = yearMatch ? Number(yearMatch[1]) : new Date().getFullYear()
  const lines: string[] = [
    'BEGIN:VCALENDAR',
    'VERSION:2.0',
    'PRODID:-//Planie//UDST Schedule Planner//EN',
    'CALSCALE:GREGORIAN',
    'METHOD:PUBLISH',
    `X-WR-CALNAME:${escapeIcs(`Planie - ${term}`)}`,
  ]
  for (const c of courses) {
    if (state.removed.includes(c.code)) continue
    const opt = effective(c, state)
    if (!opt) continue
    for (const comp of opt.components) {
      const { startYmd, untilYmd } = parseDateRange(comp.dates, fallbackYear)
      // Group meetings sharing same start/end time across multiple days cleanly
      const byTime = new Map<string, number[]>()
      for (const m of comp.meetings) {
        const key = `${m.start}-${m.end}`
        const list = byTime.get(key) ?? []
        if (!list.includes(m.day)) list.push(m.day)
        byTime.set(key, list)
      }
      for (const [key, days] of byTime) {
        days.sort((a, b) => a - b)
        const [startMin, endMin] = key.split('-').map(Number)
        const dtStartDay = firstMatchingDate(startYmd, days[0])
        const byDay = days.map((d) => ICS_DAY[d] ?? 'SU').join(',')
        const uid = `${c.code.replace(/\s+/g, '')}-${comp.classNbr}-${key}@planie.local`
        const summary = `${c.code} (${comp.kind}) - Sec ${comp.section}`
        const desc = `${c.title}\nInstructor: ${comp.instructors.join(', ')}\nClass #${comp.classNbr}`
        lines.push(
          'BEGIN:VEVENT',
          `UID:${escapeIcs(uid)}`,
          `SUMMARY:${escapeIcs(summary)}`,
          `LOCATION:${escapeIcs(comp.room || 'Campus')}`,
          `DESCRIPTION:${escapeIcs(desc)}`,
          `DTSTART:${dtStartDay}T${fmtTime(startMin)}`,
          `DTEND:${dtStartDay}T${fmtTime(endMin)}`,
          `RRULE:FREQ=WEEKLY;BYDAY=${byDay};UNTIL=${untilYmd}`,
          'END:VEVENT',
        )
      }
    }
  }
  lines.push('END:VCALENDAR')
  return lines.join('\r\n') + '\r\n'
}

export function downloadIcs(courses: PCourse[], state: PlannerState, term: string): void {
  const ics = buildIcs(courses, state, term)
  const blob = new Blob([ics], { type: 'text/calendar;charset=utf-8' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = `planie-${term.toLowerCase().replace(/\s+/g, '-')}.ics`
  document.body.appendChild(a)
  a.click()
  a.remove()
  setTimeout(() => URL.revokeObjectURL(url), 1000)
}
